use crate::{BranchDomain, BranchNodeCompact, Hash, HashProvider, MerkleError, Result, path};
use golemdb_storage::{ReadTransaction, StorageError, Table, WriteTransaction};

/// The owner supplies a hash binding the complete path and its leaf payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeafRef<const N: usize> {
    pub path: [u8; N],
    pub hash: Hash,
}

/// Keep this metadata with retained roots. A bare singleton hash cannot recover
/// its path, and a missing branch must never be interpreted as a singleton.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RootRef<const N: usize> {
    #[default]
    Empty,
    Leaf(LeafRef<N>),
    Branch(Hash),
}

impl<const N: usize> RootRef<N> {
    /// Empty root = H(empty bytes); singleton root = its leaf hash.
    pub fn hash(&self, hasher: &(impl HashProvider + ?Sized)) -> Hash {
        match self {
            Self::Empty => hasher.hash(&[]),
            Self::Leaf(leaf) => leaf.hash,
            Self::Branch(hash) => *hash,
        }
    }
}

/// Branch-only compressed hexary trie over the caller's storage transaction.
/// Each table fixes a path width, domain, hash algorithm and canonical codec.
/// Mutations retain old branches; no operation commits or opens a transaction.
/// Abort the enclosing write transaction after a mutation error: intermediate
/// immutable branches may already have been inserted.
pub struct Trie<'h, H: HashProvider + ?Sized, const N: usize> {
    table: Table,
    domain: BranchDomain,
    hasher: &'h H,
}

impl<'h, H: HashProvider + ?Sized, const N: usize> Trie<'h, H, N> {
    pub fn new(table: Table, domain: BranchDomain, hasher: &'h H) -> Result<Self> {
        path::check_width::<N>()?;
        Ok(Self {
            table,
            domain,
            hasher,
        })
    }

    /// Validate a branch root's row, hash and routing at depth zero without
    /// traversing descendants. Empty/leaf references have no branch to load;
    /// authenticating a leaf payload remains the owner's responsibility.
    pub fn validate_root(&self, tx: &impl ReadTransaction, root: RootRef<N>) -> Result<()> {
        if let RootRef::Branch(hash) = root {
            self.load(tx, hash, &[])?;
        }
        Ok(())
    }

    pub fn get(
        &self,
        tx: &impl ReadTransaction,
        mut root: RootRef<N>,
        path: &[u8; N],
    ) -> Result<Option<LeafRef<N>>> {
        let target = path::nibbles(path);
        let mut route = Vec::new();
        loop {
            match root {
                RootRef::Empty => return Ok(None),
                RootRef::Leaf(leaf) => return Ok((leaf.path == *path).then_some(leaf)),
                RootRef::Branch(hash) => {
                    let node = self.load(tx, hash, &route)?;
                    route.extend(node.unpack_prefix());
                    if !target.starts_with(&route) {
                        return Ok(None);
                    }
                    let slot = target[route.len()];
                    route.push(slot);
                    root = node.child(slot).unwrap_or(RootRef::Empty);
                }
            }
        }
    }

    pub fn insert(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<N>,
        leaf: LeafRef<N>,
    ) -> Result<RootRef<N>> {
        self.edit(tx, root, &leaf.path, Some(leaf), &[])
    }

    pub fn remove(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<N>,
        path: &[u8; N],
    ) -> Result<RootRef<N>> {
        self.edit(tx, root, path, None, &[])
    }

    /// Lazy ascending path traversal. Storage/corruption errors are yielded once
    /// and terminate iteration. Only branches on the traversal frontier are held.
    pub fn walk<'a, T: ReadTransaction>(
        &'a self,
        tx: &'a T,
        root: RootRef<N>,
    ) -> Walk<'a, T, H, N> {
        Walk {
            trie: Trie {
                table: self.table,
                domain: self.domain,
                hasher: self.hasher,
            },
            tx,
            stack: vec![(root, Vec::new())],
        }
    }

    fn load(
        &self,
        tx: &impl ReadTransaction,
        hash: Hash,
        route: &[u8],
    ) -> Result<BranchNodeCompact<N>> {
        let bytes = tx
            .get(self.table, &hash)?
            .ok_or(MerkleError::MissingBranch(hash))?;
        let node = BranchNodeCompact::decode(&bytes)?;
        if node.hash(self.domain, self.hasher) != hash {
            return Err(MerkleError::HashMismatch);
        }
        node.validate_at(route)?;
        Ok(node)
    }

    fn store(
        &self,
        tx: &mut impl WriteTransaction,
        node: BranchNodeCompact<N>,
        route: &[u8],
    ) -> Result<RootRef<N>> {
        node.validate_at(route)?;
        let hash = node.hash(self.domain, self.hasher);
        let bytes = node.encode();
        match tx.insert(self.table, &hash, &bytes) {
            Ok(()) => (),
            Err(StorageError::AlreadyExists) => {
                if tx.get(self.table, &hash)?.as_deref() != Some(bytes.as_slice()) {
                    return Err(MerkleError::ConflictingBranch);
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(RootRef::Branch(hash))
    }

    fn edit(
        &self,
        tx: &mut impl WriteTransaction,
        root: RootRef<N>,
        target: &[u8; N],
        replacement: Option<LeafRef<N>>,
        route: &[u8],
    ) -> Result<RootRef<N>> {
        match root {
            RootRef::Empty => Ok(replacement.map(RootRef::Leaf).unwrap_or(RootRef::Empty)),
            RootRef::Leaf(old) => {
                if old.path == *target {
                    return Ok(replacement.map(RootRef::Leaf).unwrap_or(RootRef::Empty));
                }
                let Some(new) = replacement else {
                    return Ok(root);
                };
                let old_path = path::nibbles(&old.path);
                let new_path = path::nibbles(target);
                let depth = (route.len()..N * 2)
                    .find(|&i| old_path[i] != new_path[i])
                    .unwrap();
                let mut children = [RootRef::Empty; 16];
                children[old_path[depth] as usize] = root;
                children[new_path[depth] as usize] = RootRef::Leaf(new);
                self.store(
                    tx,
                    BranchNodeCompact::from_children(&new_path[route.len()..depth], &children)?,
                    route,
                )
            }
            RootRef::Branch(hash) => {
                let node = self.load(tx, hash, route)?;
                let prefix = node.unpack_prefix();
                let target_path = path::nibbles(target);
                let shared = prefix
                    .iter()
                    .zip(&target_path[route.len()..])
                    .take_while(|(a, b)| a == b)
                    .count();
                if shared != prefix.len() {
                    let Some(new) = replacement else {
                        return Ok(root);
                    };
                    let mut old_route = route.to_vec();
                    old_route.extend_from_slice(&prefix[..shared + 1]);
                    let old = self.store(
                        tx,
                        BranchNodeCompact::from_children(&prefix[shared + 1..], &node.children())?,
                        &old_route,
                    )?;
                    let mut children = [RootRef::Empty; 16];
                    children[prefix[shared] as usize] = old;
                    children[target_path[route.len() + shared] as usize] = RootRef::Leaf(new);
                    return self.store(
                        tx,
                        BranchNodeCompact::from_children(&prefix[..shared], &children)?,
                        route,
                    );
                }
                let mut child_route = route.to_vec();
                child_route.extend_from_slice(&prefix);
                let slot = target_path[child_route.len()] as usize;
                child_route.push(slot as u8);
                let mut children = node.children();
                let changed = self.edit(tx, children[slot], target, replacement, &child_route)?;
                if changed == children[slot] {
                    return Ok(root);
                }
                children[slot] = changed;
                let active: Vec<_> = children
                    .iter()
                    .enumerate()
                    .filter(|(_, child)| **child != RootRef::Empty)
                    .collect();
                if let [(slot, only)] = active.as_slice() {
                    if let RootRef::Branch(hash) = **only {
                        let mut child_route = route.to_vec();
                        child_route.extend_from_slice(&prefix);
                        child_route.push(*slot as u8);
                        let child = self.load(tx, hash, &child_route)?;
                        let mut merged = prefix;
                        merged.push(*slot as u8);
                        merged.extend(child.unpack_prefix());
                        self.store(
                            tx,
                            BranchNodeCompact::from_children(&merged, &child.children())?,
                            route,
                        )
                    } else {
                        Ok(**only)
                    }
                } else {
                    self.store(
                        tx,
                        BranchNodeCompact::from_children(&prefix, &children)?,
                        route,
                    )
                }
            }
        }
    }
}

pub struct Walk<'a, T: ReadTransaction, H: HashProvider + ?Sized, const N: usize> {
    trie: Trie<'a, H, N>,
    tx: &'a T,
    stack: Vec<(RootRef<N>, Vec<u8>)>,
}

impl<T: ReadTransaction, H: HashProvider + ?Sized, const N: usize> Iterator for Walk<'_, T, H, N> {
    type Item = Result<LeafRef<N>>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((root, mut route)) = self.stack.pop() {
            match root {
                RootRef::Empty => (),
                RootRef::Leaf(leaf) => return Some(Ok(leaf)),
                RootRef::Branch(hash) => {
                    let node = match self.trie.load(self.tx, hash, &route) {
                        Ok(node) => node,
                        Err(error) => {
                            self.stack.clear();
                            return Some(Err(error));
                        }
                    };
                    route.extend(node.unpack_prefix());
                    for slot in (0..16).rev() {
                        if let Some(child) = node.child(slot) {
                            let mut child_route = route.clone();
                            child_route.push(slot);
                            self.stack.push((child, child_route));
                        }
                    }
                }
            }
        }
        None
    }
}

impl<T: ReadTransaction, H: HashProvider + ?Sized, const N: usize> std::iter::FusedIterator
    for Walk<'_, T, H, N>
{
}
