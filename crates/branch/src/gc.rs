//! Persistent reference counting for head-only physical state.
//!
//! The head owns cell/index trie roots; current index terms own bitmap roots.
//! `NodeRefs` counts these root references and edges from live physical parents.
//! Each parent's edges count once, regardless of how many owners that parent has.
//! Acquire new roots before releasing old ones; a zero count releases children
//! and deletes the node. Counts, deletions and head publication share one writer.
//! Genesis seeds counts in empty tables; existing databases are never migrated.
use std::collections::{BTreeMap, BTreeSet};

use golemdb_cells::tables as cells;
use golemdb_index::tables as index;
use golemdb_merkle::{BranchNodeCompact, Hash, RootRef};
use golemdb_storage::{ReadTransaction, Table, WriteTransaction};

use crate::{BranchError, Result, SealedCommit, metadata::NODE_REFS as REFS};

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

    fn children(self, tx: &impl ReadTransaction) -> Result<Vec<Node>> {
        // Containers have no outgoing edges. Their immutable payload was
        // authenticated/created during seal; publication need not read it.
        if self.kind == Kind::Container {
            return Ok(Vec::new());
        }
        let bytes = tx
            .get(self.kind.table(), &self.hash)?
            .ok_or(BranchError::GarbageCollection("referenced node is missing"))?;
        match self.kind {
            Kind::Cell | Kind::Index => branches::<32>(self, &bytes),
            Kind::Bitmap => branches::<6>(self, &bytes),
            Kind::Container => unreachable!(),
        }
    }
}

fn branches<const N: usize>(parent: Node, bytes: &[u8]) -> Result<Vec<Node>> {
    let node = BranchNodeCompact::<N>::decode(bytes)?;
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
            _ => {} // Cell/index leaves have no physical row.
        }
    }
    Ok(children)
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
struct RootChanges(BTreeMap<Node, (u64, u64)>);

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

struct Counter {
    before: u64,
    after: u64,
}

struct Session<'tx, T> {
    tx: &'tx mut T,
    counts: BTreeMap<Node, Counter>,
    creating: bool,
    created: BTreeSet<Node>,
}

impl<T: WriteTransaction> Session<'_, T> {
    fn counter(&mut self, node: Node) -> Result<&mut Counter> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.counts.entry(node) {
            let before = match self.tx.get(REFS, &node.key())? {
                None if self.creating || self.created.contains(&node) => 0,
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
                pending.extend(node.children(self.tx)?.into_iter().map(|child| (child, 1)));
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
                pending.extend(node.children(self.tx)?.into_iter().map(|child| (child, 1)));
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

/// Publish root ownership after replay; old and new bitmap roots can both
/// be resolved here because rows are not reclaimed until the final flush.
pub(crate) fn update(tx: &mut impl WriteTransaction, sealed: &SealedCommit) -> Result<()> {
    let mut changes = RootChanges::default();
    changes.remove(root(Kind::Cell, sealed.origin_cells));
    changes.remove(root(Kind::Index, sealed.origin_index));
    changes.add(root(Kind::Cell, sealed.cells.root));
    changes.add(root(Kind::Index, sealed.index.root));
    for term in &sealed.index.changed_terms {
        if let Some(hash) = term.before {
            changes.remove(Some(bitmap_root(tx, hash)?));
        }
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
        counts: BTreeMap::new(),
        creating: false,
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

/// Seed counters only for freshly created genesis rows. The creation path
/// requires every engine table to be empty before building these commitments;
/// this never imports, validates, sweeps or migrates an existing database.
pub(crate) fn seed_counts(
    tx: &mut impl WriteTransaction,
    cells: RootRef<32>,
    index: RootRef<32>,
    bitmaps: impl IntoIterator<Item = Hash>,
) -> Result<()> {
    let mut roots = RootChanges::default();
    roots.add(root(Kind::Cell, cells));
    roots.add(root(Kind::Index, index));
    for hash in bitmaps {
        roots.add(Some(bitmap_root(tx, hash)?));
    }
    let mut session = Session {
        tx,
        counts: BTreeMap::new(),
        creating: true,
        created: BTreeSet::new(),
    };
    for (node, (count, _)) in roots.0 {
        session.retain(node, count)?;
    }
    session.flush()
}
