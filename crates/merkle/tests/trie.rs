use std::{cell::Cell, collections::BTreeMap};

use golemdb_merkle::{
    BranchDomain, Hash, HashAlgorithm, HashProvider, LeafRef, MerkleError, RootRef, Trie,
};
use golemdb_storage::{Database, MemoryDatabase, ReadTransaction, Table, WriteTransaction};
use proptest::prelude::*;

const TABLE: Table = Table("TestBranches");
const HASH: HashAlgorithm = HashAlgorithm::Keccak256;

fn leaf<const N: usize>(path: [u8; N], value: u32) -> LeafRef<N> {
    LeafRef {
        path,
        hash: HASH.hash_parts(&[&[0x04], &path, &value.to_be_bytes()]),
    }
}

// Independent sorted-leaf oracle: groups paths by their first differing nibble,
// directly frames preimages, and never uses production node codecs or mutation.
fn bulk<const N: usize>(leaves: &[LeafRef<N>], depth: usize) -> Hash {
    let digit = |path: &[u8; N], i: usize| {
        if i.is_multiple_of(2) {
            path[i / 2] >> 4
        } else {
            path[i / 2] & 15
        }
    };
    if leaves.is_empty() {
        return HASH.hash(b"");
    }
    if leaves.len() == 1 {
        return leaves[0].hash;
    }
    let mut split = depth;
    while digit(&leaves[0].path, split) == digit(&leaves.last().unwrap().path, split) {
        split += 1;
    }
    let mut preimage = vec![0x05, (split - depth) as u8];
    for i in (depth..split).step_by(2) {
        preimage.push(
            digit(&leaves[0].path, i) * 16
                + if i + 1 < split {
                    digit(&leaves[0].path, i + 1)
                } else {
                    0
                },
        );
    }
    let mut groups: BTreeMap<u8, Vec<LeafRef<N>>> = BTreeMap::new();
    for leaf in leaves {
        groups
            .entry(digit(&leaf.path, split))
            .or_default()
            .push(*leaf);
    }
    let state = groups.keys().fold(0u16, |mask, slot| mask | 1 << slot);
    let tree = groups
        .iter()
        .filter(|(_, group)| group.len() > 1)
        .fold(0u16, |mask, (slot, _)| mask | 1 << slot);
    preimage.extend(state.to_be_bytes());
    preimage.extend(tree.to_be_bytes());
    for group in groups.values() {
        preimage.extend(bulk(group, split + 1));
    }
    HASH.hash(&preimage)
}

fn key<const N: usize>(id: u8) -> [u8; N] {
    let mut path = [0xaa; N];
    // Deep prefixes, odd splits, different root slots and repeated keys.
    if id & 8 != 0 {
        path[0] = id >> 4;
    }
    path[N - 1] = id & 0x37;
    path
}

fn exercise<const N: usize>(ops: &[(u8, u32, u8)]) {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, N>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let mut tx = db.begin_write().unwrap();
    let mut expected = BTreeMap::new();
    let mut root = RootRef::Empty;
    let mut history = Vec::new();
    for &(id, value, action) in ops {
        let path = key(id);
        if action == 0 {
            root = trie.remove(&mut tx, root, &path).unwrap();
            expected.remove(&path);
        } else {
            let leaf = leaf(path, value);
            root = trie.insert(&mut tx, root, leaf).unwrap();
            expected.insert(path, leaf);
        }
        let sorted: Vec<_> = expected.values().copied().collect();
        assert_eq!(root.hash(&HASH), bulk(&sorted, 0));
        assert_eq!(
            trie.walk(&tx, root).collect::<Result<Vec<_>, _>>().unwrap(),
            sorted
        );
        for id in 0..64 {
            assert_eq!(
                trie.get(&tx, root, &key(id)).unwrap(),
                expected.get(&key(id)).copied()
            );
        }
        history.push((root, sorted));
    }
    tx.commit().unwrap();
    let read = db.begin_read().unwrap();
    for (root, leaves) in history {
        assert_eq!(
            trie.walk(&read, root)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            leaves
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn random_edits_match_bulk_and_preserve_history(ops in prop::collection::vec((0u8..64, any::<u32>(), 0u8..3), 0..90)) {
        exercise::<6>(&ops);
        exercise::<32>(&ops);
    }
}

#[test]
fn empty_singleton_branch_and_replacement() {
    exercise::<6>(&[
        (1, 10, 0),
        (1, 10, 1),
        (1, 10, 1),
        (2, 20, 1),
        (1, 30, 1),
        (3, 0, 0),
        (2, 0, 0),
        (1, 0, 0),
    ]);
    exercise::<32>(&[
        (1, 10, 0),
        (1, 10, 1),
        (1, 10, 1),
        (2, 20, 1),
        (1, 30, 1),
        (3, 0, 0),
        (2, 0, 0),
        (1, 0, 0),
    ]);
}

fn deepest<const N: usize>() {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, N>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let mut tx = db.begin_write().unwrap();
    let mut root = RootRef::Empty;
    let mut leaves = vec![leaf([0; N], 1)];
    for nibble in 0..N * 2 {
        let mut path = [0; N];
        path[nibble / 2] = if nibble.is_multiple_of(2) { 0x10 } else { 0x01 };
        leaves.push(leaf(path, 2));
    }
    for leaf in &leaves {
        root = trie.insert(&mut tx, root, *leaf).unwrap();
    }
    let original = root;
    for leaf in &leaves {
        root = trie.remove(&mut tx, root, &leaf.path).unwrap();
        let remaining = trie.walk(&tx, root).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(root.hash(&HASH), bulk(&remaining, 0));
    }
    assert_eq!(root, RootRef::Empty);
    leaves.sort_by_key(|leaf| leaf.path);
    assert_eq!(
        trie.walk(&tx, original)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        leaves
    );
}

#[test]
fn every_depth_splits_and_collapses() {
    deepest::<6>();
    deepest::<32>();
}

#[test]
fn insertion_order_does_not_change_root() {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, 6>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let leaves = [1, 2, 8, 32].map(|id| leaf(key(id), id as u32));
    let mut sorted = leaves.to_vec();
    sorted.sort_by_key(|leaf| leaf.path);
    let expected = bulk(&sorted, 0);
    let mut tx = db.begin_write().unwrap();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    if [a, b, c, d]
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != 4
                    {
                        continue;
                    }
                    let mut root = RootRef::Empty;
                    for i in [a, b, c, d] {
                        root = trie.insert(&mut tx, root, leaves[i]).unwrap();
                    }
                    assert_eq!(root.hash(&HASH), expected);
                }
            }
        }
    }
}

#[test]
fn snapshots_commit_abort_and_missing_branches() {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, 6>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let old_read = db.begin_read().unwrap();
    let a = leaf(key(1), 1);
    let b = leaf(key(2), 2);
    let mut tx = db.begin_write().unwrap();
    let root = trie.insert(&mut tx, RootRef::Leaf(a), b).unwrap();
    assert!(matches!(
        trie.get(&old_read, root, &a.path),
        Err(MerkleError::MissingBranch(_))
    ));
    tx.commit().unwrap();
    assert!(matches!(
        trie.get(&old_read, root, &a.path),
        Err(MerkleError::MissingBranch(_))
    ));
    let read = db.begin_read().unwrap();
    assert_eq!(trie.get(&read, root, &a.path).unwrap(), Some(a));
    let mut tx = db.begin_write().unwrap();
    let aborted = trie.insert(&mut tx, root, leaf(key(3), 3)).unwrap();
    tx.abort();
    let latest = db.begin_read().unwrap();
    assert!(matches!(
        trie.get(&latest, aborted, &a.path),
        Err(MerkleError::MissingBranch(_))
    ));
    assert_eq!(trie.walk(&latest, root).count(), 2);
    let mut walk = trie.walk(&latest, aborted);
    assert!(matches!(
        walk.next(),
        Some(Err(MerkleError::MissingBranch(_)))
    ));
    assert!(walk.next().is_none());
    assert!(walk.next().is_none());
}

struct Counted<T> {
    tx: T,
    reads: Cell<usize>,
    writes: usize,
    fail_reads: bool,
    fail_write: Option<usize>,
}
impl<T: ReadTransaction> ReadTransaction for Counted<T> {
    type Cursor<'a>
        = T::Cursor<'a>
    where
        Self: 'a;
    fn get(&self, table: Table, key: &[u8]) -> golemdb_storage::Result<Option<Vec<u8>>> {
        self.reads.set(self.reads.get() + 1);
        if self.fail_reads {
            return Err(injected_error());
        }
        self.tx.get(table, key)
    }
    fn cursor(&self, table: Table, key: &[u8]) -> golemdb_storage::Result<Self::Cursor<'_>> {
        self.tx.cursor(table, key)
    }
}
impl<T: WriteTransaction> WriteTransaction for Counted<T> {
    fn insert(&mut self, t: Table, k: &[u8], v: &[u8]) -> golemdb_storage::Result<()> {
        self.writes += 1;
        if self.fail_write == Some(self.writes) {
            return Err(injected_error());
        }
        self.tx.insert(t, k, v)
    }
    fn put(&mut self, t: Table, k: &[u8], v: &[u8]) -> golemdb_storage::Result<()> {
        panic!("unexpected upsert: {t:?} {k:?} {v:?}")
    }
    fn delete(&mut self, _: Table, _: &[u8]) -> golemdb_storage::Result<bool> {
        panic!("unexpected delete")
    }
    fn commit(self) -> golemdb_storage::Result<()> {
        self.tx.commit()
    }
    fn abort(self) {
        self.tx.abort()
    }
}

#[test]
fn updates_touch_only_affected_branches_and_walk_is_lazy() {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, 6>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let mut tx = Counted {
        tx: db.begin_write().unwrap(),
        reads: Cell::new(0),
        writes: 0,
        fail_reads: false,
        fail_write: None,
    };
    let mut root = RootRef::Empty;
    for i in 0u16..256 {
        let mut path = [0; 6];
        path[0] = i as u8;
        root = trie.insert(&mut tx, root, leaf(path, 0)).unwrap();
    }
    tx.writes = 0;
    tx.reads.set(0);
    let unchanged = trie.insert(&mut tx, root, leaf([0; 6], 0)).unwrap();
    assert_eq!(unchanged, root);
    assert_eq!(tx.writes, 0);
    let mut absent = [0; 6];
    absent[5] = 1;
    assert_eq!(trie.remove(&mut tx, root, &absent).unwrap(), root);
    assert_eq!(tx.writes, 0);
    tx.reads.set(0);
    root = trie.insert(&mut tx, root, leaf([0; 6], 1)).unwrap();
    assert_eq!(tx.writes, 2);
    assert_eq!(tx.reads.get(), 2);
    tx.reads.set(0);
    let mut walk = trie.walk(&tx, root);
    assert_eq!(tx.reads.get(), 0);
    assert_eq!(walk.next().unwrap().unwrap(), leaf([0; 6], 1));
    assert_eq!(tx.reads.get(), 2);
}

fn injected_error() -> golemdb_storage::StorageError {
    golemdb_storage::StorageError::Backend(std::io::Error::other("injected failure").into())
}

#[test]
fn storage_errors_propagate_and_abort_discards_partial_branches() {
    let db = MemoryDatabase::new();
    let trie = Trie::<_, 6>::new(TABLE, BranchDomain::Bitmap, &HASH).unwrap();
    let mut tx = db.begin_write().unwrap();
    let a = leaf([0; 6], 1);
    let b = leaf([0, 0, 0, 0, 0, 1], 2);
    let root = trie.insert(&mut tx, RootRef::Leaf(a), b).unwrap();
    tx.commit().unwrap();
    let mut tx = Counted {
        tx: db.begin_write().unwrap(),
        reads: Cell::new(0),
        writes: 0,
        fail_reads: true,
        fail_write: Some(2),
    };
    assert!(matches!(
        trie.get(&tx, root, &a.path),
        Err(MerkleError::Storage(_))
    ));
    let mut walk = trie.walk(&tx, root);
    assert!(matches!(walk.next(), Some(Err(MerkleError::Storage(_)))));
    assert!(walk.next().is_none());
    tx.fail_reads = false;
    // Splits the existing long prefix: the trimmed child is written first,
    // then writing the new parent fails. The enclosing transaction must abort.
    assert!(matches!(
        trie.insert(&mut tx, root, leaf([0x10; 6], 3)),
        Err(MerkleError::Storage(_))
    ));
    assert_eq!(tx.writes, 2);
    let rows = golemdb_storage::scan_prefix(&tx, TABLE, vec![])
        .unwrap()
        .collect::<golemdb_storage::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    tx.abort();
    let read = db.begin_read().unwrap();
    assert_eq!(
        golemdb_storage::scan_prefix(&read, TABLE, vec![])
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        trie.walk(&read, root)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        vec![a, b]
    );
}
