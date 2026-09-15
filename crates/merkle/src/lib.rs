//! GolemDB merkle: a persistent (copy-on-write) sparse radix merkle tree.
//!
//! Generic over the hash `H` (any [`Digest`]), branching factor `B` (a power
//! of two, 2..=256) and key width `K` bytes. Each level consumes `log2(B)` key
//! bits, most significant first.
//!
//! # Sharing across blocks
//!
//! Nodes are `Arc`s. Cloning a [`Tree`] is O(1); writing to the clone copies
//! only the root-to-leaf path and shares every untouched subtree with its
//! parent block. Dropping a tree (see [`History`]) releases its references,
//! freeing exactly the nodes no other retained block still uses.
//!
//! Hashes are computed lazily and cached per node, so a block's batch of
//! writes hashes each touched node once, at [`Tree::root`], not once per write.
//! Dirty subtrees are hashed in parallel on the rayon pool.
//!
//! # Shape
//!
//! ```text
//! leaf    H(0x00 ‖ key ‖ H(value))
//! branch  H(0x01 ‖ bitmap ‖ present children, in slot order)
//! empty   zero digest
//! ```
//!
//! `bitmap` is `ceil(B/8)` bytes, bit `i % 8` of byte `i / 8` set when slot
//! `i` is occupied.
//!
//! A leaf sits at the shallowest depth where its key prefix is unique, and a
//! branch always holds at least two leaves below it, so the root depends only
//! on the key/value set, not on write order.
//!
//! There are no extension nodes: with uniformly distributed keys (hashes)
//! chains of single-child branches only appear below ~log_B(n) depth with
//! vanishing probability. Hash non-uniform keys before inserting.

use digest::{Digest, Output};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

/// Dirty subtrees to fan out to rayon in [`Tree::root`]; a few per core.
const PAR_TASKS: usize = 64;

type Child<H, const B: usize, const K: usize> = Option<Arc<Node<H, B, K>>>;

enum Kind<H: Digest, const B: usize, const K: usize> {
    Leaf { key: [u8; K], value: Output<H> },
    Branch([Child<H, B, K>; B]),
}

struct Node<H: Digest, const B: usize, const K: usize> {
    kind: Kind<H, B, K>,
    hash: OnceLock<Output<H>>,
}

// Manual impls: `derive` would demand `H: Clone`.
impl<H: Digest, const B: usize, const K: usize> Clone for Node<H, B, K> {
    fn clone(&self) -> Self {
        let kind = match &self.kind {
            Kind::Leaf { key, value } => Kind::Leaf {
                key: *key,
                value: value.clone(),
            },
            Kind::Branch(children) => Kind::Branch(children.clone()),
        };
        Node {
            kind,
            hash: self.hash.clone(),
        }
    }
}

impl<H: Digest, const B: usize, const K: usize> Node<H, B, K> {
    fn new(kind: Kind<H, B, K>) -> Arc<Self> {
        Arc::new(Node {
            kind,
            hash: OnceLock::new(),
        })
    }

    fn hash(&self) -> &Output<H> {
        self.hash.get_or_init(|| match &self.kind {
            Kind::Leaf { key, value } => H::new()
                .chain_update([0])
                .chain_update(key)
                .chain_update(value)
                .finalize(),
            Kind::Branch(children) => {
                let mut bitmap = [0u8; 32];
                for (i, child) in children.iter().enumerate() {
                    if child.is_some() {
                        bitmap[i / 8] |= 1 << (i % 8);
                    }
                }
                let mut h = H::new()
                    .chain_update([1])
                    .chain_update(&bitmap[..B.div_ceil(8)]);
                for node in children.iter().flatten() {
                    h.update(node.hash());
                }
                h.finalize()
            }
        })
    }
}

/// One block's state. Clone it to start the next block.
pub struct Tree<H: Digest, const B: usize, const K: usize> {
    root: Child<H, B, K>,
}

impl<H: Digest, const B: usize, const K: usize> Clone for Tree<H, B, K> {
    fn clone(&self) -> Self {
        Tree {
            root: self.root.clone(),
        }
    }
}

impl<H: Digest, const B: usize, const K: usize> Default for Tree<H, B, K> {
    fn default() -> Self {
        Tree { root: None }
    }
}

impl<H: Digest, const B: usize, const K: usize> Tree<H, B, K> {
    /// Key bits per level. Evaluating it rejects a bad `B` at compile time.
    const BITS: usize = {
        assert!(
            B.is_power_of_two() && B >= 2 && B <= 256,
            "B must be a power of two in 2..=256"
        );
        B.trailing_zeros() as usize
    };

    pub fn new() -> Self {
        Self::default()
    }

    fn nibble(key: &[u8; K], depth: usize) -> usize {
        let bit = depth * Self::BITS;
        let hi = key[bit / 8] as u16;
        let lo = key.get(bit / 8 + 1).copied().unwrap_or(0) as u16;
        (((hi << 8 | lo) << (bit % 8)) >> (16 - Self::BITS)) as usize
    }

    /// Commitment to the whole key/value set; zero digest when empty.
    pub fn root(&self) -> Output<H> {
        let Some(root) = &self.root else {
            return Output::<H>::default();
        };
        // Walk down dirty branches until there are enough dirty subtrees to
        // be worth a task each, hash those in parallel, then finish the top
        // levels serially from the cache.
        let mut frontier = vec![root];
        while frontier.len() < PAR_TASKS {
            let next: Vec<_> = frontier
                .iter()
                .filter_map(|n| match &n.kind {
                    Kind::Branch(children) => Some(children),
                    Kind::Leaf { .. } => None,
                })
                .flatten()
                .flatten()
                .filter(|n| n.hash.get().is_none())
                .collect();
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        frontier.par_iter().for_each(|n| {
            n.hash();
        });
        root.hash().clone()
    }

    /// `H(value)` stored under `key`.
    pub fn get(&self, key: &[u8; K]) -> Option<&Output<H>> {
        let mut node = self.root.as_deref()?;
        let mut depth = 0;
        loop {
            match &node.kind {
                Kind::Leaf { key: k, value } => return (k == key).then_some(value),
                Kind::Branch(children) => {
                    node = children[Self::nibble(key, depth)].as_deref()?;
                    depth += 1;
                }
            }
        }
    }

    pub fn insert(&mut self, key: [u8; K], value: &[u8]) {
        Self::insert_at(&mut self.root, 0, key, H::digest(value));
    }

    fn insert_at(slot: &mut Child<H, B, K>, depth: usize, key: [u8; K], value: Output<H>) {
        let Some(node) = slot else {
            *slot = Some(Node::new(Kind::Leaf { key, value }));
            return;
        };
        // A different leaf here: push it one level down under a new branch.
        // Leaf hashes don't depend on depth, so the old leaf is moved, not copied.
        if let Kind::Leaf { key: k, .. } = &node.kind
            && *k != key
        {
            let nibble = Self::nibble(k, depth);
            let mut children = std::array::from_fn(|_| None);
            children[nibble] = slot.take();
            *slot = Some(Node::new(Kind::Branch(children)));
        }
        let node = Arc::make_mut(slot.as_mut().unwrap());
        node.hash.take();
        match &mut node.kind {
            Kind::Branch(children) => Self::insert_at(
                &mut children[Self::nibble(&key, depth)],
                depth + 1,
                key,
                value,
            ),
            Kind::Leaf { value: v, .. } => *v = value,
        }
    }

    /// Returns whether `key` was present.
    pub fn remove(&mut self, key: &[u8; K]) -> bool {
        // Check first so a miss copies no path.
        if self.get(key).is_none() {
            return false;
        }
        Self::remove_at(&mut self.root, 0, key);
        true
    }

    fn remove_at(slot: &mut Child<H, B, K>, depth: usize, key: &[u8; K]) {
        // The leaf itself (`remove` checked the key). Test before `make_mut`
        // so a shared leaf isn't copied just to be dropped.
        if let Some(Kind::Leaf { .. }) = slot.as_deref().map(|n| &n.kind) {
            *slot = None;
            return;
        }
        let node = Arc::make_mut(slot.as_mut().unwrap());
        let Kind::Branch(children) = &mut node.kind else {
            unreachable!()
        };
        node.hash.take();
        Self::remove_at(&mut children[Self::nibble(key, depth)], depth + 1, key);
        // Keep the shape canonical: a branch left with one leaf becomes that leaf.
        let mut rest = children.iter().flatten();
        let lone_leaf = match (rest.next(), rest.next()) {
            (Some(only), None) if matches!(only.kind, Kind::Leaf { .. }) => Some(only.clone()),
            _ => None,
        };
        if lone_leaf.is_some() {
            *slot = lone_leaf;
        }
    }
}

/// The last `cap` block trees, oldest first. Pushing past `cap` prunes the oldest.
pub struct History<H: Digest, const B: usize, const K: usize> {
    cap: usize,
    trees: VecDeque<Tree<H, B, K>>,
}

impl<H: Digest, const B: usize, const K: usize> History<H, B, K> {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "cap must be positive");
        History {
            cap,
            trees: VecDeque::with_capacity(cap + 1),
        }
    }

    pub fn push(&mut self, tree: Tree<H, B, K>) {
        self.trees.push_back(tree);
        if self.trees.len() > self.cap {
            self.trees.pop_front();
        }
    }

    /// `back = 0` is the latest block.
    pub fn get(&self, back: usize) -> Option<&Tree<H, B, K>> {
        self.trees
            .len()
            .checked_sub(back + 1)
            .map(|i| &self.trees[i])
    }

    pub fn latest(&self) -> Option<&Tree<H, B, K>> {
        self.get(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use sha2::Sha256;

    fn build<const B: usize, const K: usize>(keys: &[[u8; K]]) -> Tree<Sha256, B, K> {
        let mut t = Tree::new();
        for k in keys {
            t.insert(*k, k);
        }
        t
    }

    proptest! {
        // K = 1, B = 2 forces deep splits; K = 32, B = 16 is the realistic shape.
        #[test]
        fn root_is_canonical(
            small in proptest::collection::btree_set(any::<[u8; 1]>(), 0..64),
            wide in proptest::collection::btree_set(any::<[u8; 32]>(), 0..64),
        ) {
            check::<2, 1>(small.into_iter().collect());
            check::<16, 32>(wide.into_iter().collect());
        }
    }

    fn check<const B: usize, const K: usize>(keys: Vec<[u8; K]>) {
        let forward = build::<B, K>(&keys);
        let rev: Vec<_> = keys.iter().rev().copied().collect();
        assert_eq!(forward.root(), build::<B, K>(&rev).root());

        let (keep, drop) = keys.split_at(keys.len() / 2);
        let mut pruned = forward.clone();
        for k in drop {
            assert!(pruned.remove(k));
            assert!(!pruned.remove(k));
        }
        assert_eq!(pruned.root(), build::<B, K>(keep).root());
        for k in keep {
            assert_eq!(pruned.get(k), Some(&Sha256::digest(k)));
        }
        // The clone's writes never leak into the original.
        for k in &keys {
            assert!(forward.get(k).is_some());
        }
    }

    #[test]
    fn history_prunes_and_shares() {
        let mut h = History::<Sha256, 16, 32>::new(2);
        let mut t = Tree::new();
        let mut roots = vec![];
        // 0x00…, 0x11…, 0x22… land in distinct top-level children.
        for block in 0u8..3 {
            t.insert([block * 0x11; 32], b"v");
            roots.push(t.root());
            h.push(t.clone());
        }
        assert_eq!(h.latest().unwrap().root(), roots[2]);
        assert_eq!(h.get(1).unwrap().root(), roots[1]);
        assert!(h.get(2).is_none());
        // Block 0's leaf outlives block 0: one reference from each retained
        // block's root (block 2's root is the same `Arc` as `t`'s).
        let Kind::Branch(c) = &t.root.as_ref().unwrap().kind else {
            panic!()
        };
        assert_eq!(Arc::strong_count(c[0].as_ref().unwrap()), 2);
    }
}
