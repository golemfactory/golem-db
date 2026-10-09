//! Persistent reference counting for head-only physical state.
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};

use golemdb_cells::{CELL_BRANCH_DOMAIN, Cells, tables as cells};
use golemdb_index::{
    BITMAP_BRANCH_DOMAIN, BITMAP_LEAF_DOMAIN, BitmapContainer, INDEX_BRANCH_DOMAIN, Index,
    IndexError, tables as index,
};
use golemdb_merkle::{BranchNodeCompact, Hash, HashProvider, RootRef};
use golemdb_storage::{ReadCursor, ReadTransaction, Table, WriteTransaction, scan};

use crate::{BranchError, Result, SealedCommit, head::Head};

const REFS: Table = Table("NodeRefs");
const SUPERBLOCK: Table = Table("Superblock");
const MODE_KEY: &[u8] = b"head-gc-version";
const VERSION: &[u8] = &[1];

/// Results of the initial head-only collection across the four immutable tables.
/// Byte counts include row keys and values, not MDBX pages or file allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GarbageCollectionStats {
    pub retained_rows: u64,
    pub removed_rows: u64,
    pub removed_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum Kind {
    Cell = 0,
    Index = 1,
    Bitmap = 2,
    Container = 3,
}

impl Kind {
    fn table(self) -> Table {
        match self {
            Self::Cell => cells::CELL_TRIE,
            Self::Index => index::INDEX_TRIE,
            Self::Bitmap => index::BITMAP_TRIE,
            Self::Container => index::BITMAP_CONTAINER,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Node {
    kind: Kind,
    hash: Hash,
}

impl Node {
    fn key(self) -> [u8; 33] {
        let mut key = [0; 33];
        key[0] = self.kind as u8;
        key[1..].copy_from_slice(&self.hash);
        key
    }

    fn children(
        self,
        tx: &impl ReadTransaction,
        hasher: &impl HashProvider,
        authenticate: bool,
    ) -> Result<Vec<Node>> {
        let bytes = tx
            .get(self.kind.table(), &self.hash)?
            .ok_or(BranchError::GarbageCollection("referenced node is missing"))?;
        match self.kind {
            Kind::Cell => branches::<32>(self, &bytes, CELL_BRANCH_DOMAIN, hasher, authenticate),
            Kind::Index => branches::<32>(self, &bytes, INDEX_BRANCH_DOMAIN, hasher, authenticate),
            Kind::Bitmap => {
                let children =
                    branches::<6>(self, &bytes, BITMAP_BRANCH_DOMAIN, hasher, authenticate)?;
                if authenticate {
                    // Leaf paths are stored-only metadata; authenticate them
                    // against their containers when adopting the legacy graph.
                    let node = BranchNodeCompact::<6>::decode(&bytes)?;
                    for slot in 0..16 {
                        if let Some(RootRef::Leaf(leaf)) = node.child(slot) {
                            let payload = tx.get(index::BITMAP_CONTAINER, &leaf.hash)?.ok_or(
                                BranchError::GarbageCollection("referenced container is missing"),
                            )?;
                            let container =
                                BitmapContainer::decode(&payload).map_err(IndexError::from)?;
                            if container.path().to_be_bytes()[2..] != leaf.path {
                                return Err(BranchError::GarbageCollection(
                                    "container path mismatch",
                                ));
                            }
                        }
                    }
                }
                Ok(children)
            }
            Kind::Container => {
                if authenticate && hasher.hash_parts(&[&[BITMAP_LEAF_DOMAIN], &bytes]) != self.hash
                {
                    return Err(BranchError::GarbageCollection("container hash mismatch"));
                }
                if authenticate {
                    BitmapContainer::decode(&bytes).map_err(IndexError::from)?;
                }
                Ok(Vec::new())
            }
        }
    }
}

fn branches<const N: usize>(
    parent: Node,
    bytes: &[u8],
    domain: u8,
    hasher: &impl HashProvider,
    authenticate: bool,
) -> Result<Vec<Node>> {
    let node = BranchNodeCompact::<N>::decode(bytes)?;
    if authenticate && node.hash(domain, hasher) != parent.hash {
        return Err(BranchError::GarbageCollection("branch hash mismatch"));
    }
    let mut children = Vec::new();
    for slot in 0..16 {
        match node.child(slot) {
            Some(RootRef::Branch(hash)) => children.push(Node {
                kind: parent.kind,
                hash,
            }),
            Some(RootRef::Leaf(leaf)) if parent.kind == Kind::Bitmap => {
                children.push(Node {
                    kind: Kind::Container,
                    hash: leaf.hash,
                });
            }
            _ => {} // Cell/index leaves are virtual, with no physical leaf row.
        }
    }
    Ok(children)
}

pub(crate) fn initialized(tx: &impl ReadTransaction) -> Result<bool> {
    match tx.get(SUPERBLOCK, MODE_KEY)? {
        None => Ok(false),
        Some(version) if version == VERSION => Ok(true),
        Some(_) => Err(BranchError::GarbageCollection("unsupported GC version")),
    }
}

fn root<const N: usize>(kind: Kind, root: RootRef<N>) -> Option<Node> {
    match root {
        RootRef::Branch(hash) => Some(Node { kind, hash }),
        _ => None,
    }
}

fn bitmap_root(tx: &impl ReadTransaction, hash: Hash) -> Result<Node> {
    // Bitmap singletons have a physical container row; other trie singletons do not.
    let kind = if tx.get(index::BITMAP_CONTAINER, &hash)?.is_some() {
        Kind::Container
    } else {
        Kind::Bitmap
    };
    Ok(Node { kind, hash })
}

#[derive(Default)]
pub(crate) struct RootChanges(BTreeMap<Node, (u64, u64)>);

impl RootChanges {
    fn add(&mut self, node: Option<Node>) {
        if let Some(node) = node {
            self.0.entry(node).or_default().0 += 1;
        }
    }

    fn remove(&mut self, node: Option<Node>) {
        if let Some(node) = node {
            self.0.entry(node).or_default().1 += 1;
        }
    }
}

/// Capture old root kinds before publication overwrites the flat tables.
pub(crate) fn changes(tx: &impl ReadTransaction, sealed: &SealedCommit) -> Result<RootChanges> {
    let mut changes = RootChanges::default();
    changes.remove(root(Kind::Cell, sealed.origin_cells));
    changes.remove(root(Kind::Index, sealed.origin_index));
    changes.add(root(Kind::Cell, sealed.cells.root));
    changes.add(root(Kind::Index, sealed.index.root));
    for term in &sealed.index.changed_terms {
        if let Some(hash) = term.before {
            changes.remove(Some(bitmap_root(tx, hash)?));
        }
        // Newly sealed rows are not in this transaction yet; resolve them in update().
    }
    Ok(changes)
}

struct Counter {
    before: u64,
    after: u64,
}

struct Session<'tx, T, H> {
    tx: &'tx mut T,
    hasher: &'tx H,
    counts: BTreeMap<Node, Counter>,
    initializing: bool,
    created: BTreeSet<Node>,
}

impl<T: WriteTransaction, H: HashProvider> Session<'_, T, H> {
    fn counter(&mut self, node: Node) -> Result<&mut Counter> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.counts.entry(node) {
            let before = match self.tx.get(REFS, &node.key())? {
                None if self.initializing || self.created.contains(&node) => 0,
                None => {
                    return Err(BranchError::GarbageCollection(
                        "live node has no reference count",
                    ));
                }
                Some(bytes) => {
                    let count = bytes.try_into().map(u64::from_be_bytes).map_err(|_| {
                        BranchError::GarbageCollection("reference count must be eight bytes")
                    })?;
                    if count == 0 {
                        return Err(BranchError::GarbageCollection(
                            "stored reference count is zero",
                        ));
                    }
                    count
                }
            };
            entry.insert(Counter {
                before,
                after: before,
            });
        }
        Ok(self.counts.get_mut(&node).unwrap())
    }

    fn retain(&mut self, node: Node, count: u64) -> Result<()> {
        let mut pending = vec![(node, count)];
        while let Some((node, count)) = pending.pop() {
            let counter = self.counter(node)?;
            let activate = counter.after == 0;
            counter.after = counter
                .after
                .checked_add(count)
                .ok_or(BranchError::GarbageCollection("reference count overflow"))?;
            if activate {
                pending.extend(
                    node.children(self.tx, self.hasher, self.initializing)?
                        .into_iter()
                        .map(|child| (child, 1)),
                );
            }
        }
        Ok(())
    }

    fn release(&mut self, node: Node, count: u64) -> Result<()> {
        let mut pending = vec![(node, count)];
        while let Some((node, count)) = pending.pop() {
            let counter = self.counter(node)?;
            counter.after = counter
                .after
                .checked_sub(count)
                .ok_or(BranchError::GarbageCollection("reference count underflow"))?;
            if counter.after == 0 {
                pending.extend(
                    node.children(self.tx, self.hasher, self.initializing)?
                        .into_iter()
                        .map(|child| (child, 1)),
                );
            }
        }
        Ok(())
    }

    fn flush(self) -> Result<()> {
        for (node, counter) in self.counts {
            if counter.before == counter.after {
                continue;
            }
            if counter.after == 0 {
                self.tx.delete(REFS, &node.key())?;
                self.tx.delete(node.kind.table(), &node.hash)?;
            } else {
                self.tx
                    .put(REFS, &node.key(), &counter.after.to_be_bytes())?;
            }
        }
        Ok(())
    }
}

/// Apply net root changes, activating every new graph before releasing any old graph.
pub(crate) fn update(
    tx: &mut impl WriteTransaction,
    hasher: &impl HashProvider,
    mut changes: RootChanges,
    sealed: &SealedCommit,
) -> Result<()> {
    for term in &sealed.index.changed_terms {
        if let Some(hash) = term.after {
            changes.add(Some(bitmap_root(tx, hash)?));
        }
    }
    let mut created = BTreeSet::new();
    for kind in [Kind::Cell, Kind::Index, Kind::Bitmap, Kind::Container] {
        if let Some(rows) = sealed.writes.get(&kind.table()) {
            for (key, value) in rows {
                if value.is_some() {
                    let hash = key.as_slice().try_into().map_err(|_| {
                        BranchError::GarbageCollection("immutable row key must be 32 bytes")
                    })?;
                    created.insert(Node { kind, hash });
                }
            }
        }
    }
    let mut session = Session {
        tx,
        hasher,
        counts: BTreeMap::new(),
        // Bootstrap authenticated existing rows; seal authenticates changed
        // paths and creates the new rows. Publication only decodes graph edges,
        // preserving the seal/commit split without hashing or bitmap expansion.
        initializing: false,
        created,
    };
    for (&node, &(added, removed)) in &changes.0 {
        if added > removed {
            session.retain(node, added - removed)?;
        }
    }
    for (&node, &(added, removed)) in &changes.0 {
        if removed > added {
            session.release(node, removed - added)?;
        }
    }
    session.flush()
}

pub(crate) fn initialize(
    tx: &mut impl WriteTransaction,
    head: &Head,
    hasher: &impl HashProvider,
) -> Result<GarbageCollectionStats> {
    if initialized(tx)? {
        return Ok(GarbageCollectionStats::default());
    }
    if tx.cursor(REFS, b"")?.next()?.is_some() {
        return Err(BranchError::GarbageCollection(
            "reference counts exist without GC initialization",
        ));
    }
    let mut roots = RootChanges::default();
    roots.add(root(
        Kind::Cell,
        Cells::new(hasher).reopen(tx, head.state_root)?,
    ));
    let index = Index::new(hasher);
    let index_root = index.reopen(tx, head.index_root)?;
    roots.add(root(Kind::Index, index_root));
    for row in index.terms_with_prefix(tx, Vec::new())? {
        let (_, hash) = row?;
        roots.add(Some(bitmap_root(tx, hash)?));
    }
    let mut session = Session {
        tx,
        hasher,
        counts: BTreeMap::new(),
        initializing: true,
        created: BTreeSet::new(),
    };
    for (node, (count, _)) in roots.0 {
        session.retain(node, count)?;
    }
    let live = &session.counts;
    let mut stats = GarbageCollectionStats {
        retained_rows: live.len() as u64,
        ..Default::default()
    };
    for kind in [Kind::Cell, Kind::Index, Kind::Bitmap, Kind::Container] {
        // Bounded sweep batches: no second allocation proportional to all dead rows.
        let mut lower = Bound::Unbounded;
        loop {
            let rows = scan(session.tx, kind.table(), lower.clone(), Bound::Unbounded)?
                .take(256)
                .collect::<golemdb_storage::Result<Vec<_>>>()?;
            let Some((last, _)) = rows.last() else { break };
            lower = Bound::Excluded(last.clone());
            for (key, value) in rows {
                let hash = key.as_slice().try_into().map_err(|_| {
                    BranchError::GarbageCollection("immutable row key must be 32 bytes")
                })?;
                if !live.contains_key(&Node { kind, hash }) {
                    session.tx.delete(kind.table(), &key)?;
                    stats.removed_rows += 1;
                    stats.removed_bytes += (key.len() + value.len()) as u64;
                }
            }
        }
    }
    session.flush()?;
    tx.put(SUPERBLOCK, MODE_KEY, VERSION)?;
    Ok(stats)
}
