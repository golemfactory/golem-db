use std::collections::{BTreeMap, BTreeSet};

use golemdb_branch::{Branches, SealedCommit};
use golemdb_cells::{
    CellChange, CellKey, CellNameRef, CellType, CellValue, CellValueRef, Cells, tables as cells,
};
use golemdb_index::{Index, IndexTerm, PostingChange, tables as index};
use golemdb_merkle::{BranchNodeCompact, Hash, Keccak256Hasher, RootRef};
use golemdb_storage::{
    Database, MemoryDatabase, ReadTransaction, Table, WriteTransaction, scan_prefix,
};
use golemdb_storage_mdbx::MdbxDatabase;
use proptest::prelude::*;

const HASH: Keccak256Hasher = Keccak256Hasher;
const REFS: Table = Table("NodeRefs");
const TABLES: [Table; 4] = [
    cells::CELL_TRIE,
    index::INDEX_TRIE,
    index::BITMAP_TRIE,
    index::BITMAP_CONTAINER,
];

fn key(id: u64, name: &[u8]) -> CellKey {
    CellKey::new(id, CellNameRef::raw(name))
}

fn value(text: &str, indexed: bool) -> CellValue {
    CellValueRef::new(CellType::Str, text.as_bytes(), indexed)
        .unwrap()
        .into()
}

fn rows(tx: &impl ReadTransaction, table: Table) -> Vec<golemdb_storage::Entry> {
    scan_prefix(tx, table, vec![])
        .unwrap()
        .collect::<golemdb_storage::Result<_>>()
        .unwrap()
}

// Independently enumerate the entire head graph, rather than replaying GC deltas.
// Every physical parent contributes one edge even if the parent has many owners.
fn assert_exact_graph(tx: &impl ReadTransaction, sealed: &SealedCommit) {
    let mut pending = Vec::new();
    if let RootRef::Branch(hash) = sealed.cells.root {
        pending.push((0u8, hash));
    }
    if let RootRef::Branch(hash) = sealed.index.root {
        pending.push((1u8, hash));
    }
    for (_, root) in rows(tx, index::INDEX) {
        let hash: Hash = root.try_into().unwrap();
        let kind = if tx.get(index::BITMAP_CONTAINER, &hash).unwrap().is_some() {
            3
        } else {
            2
        };
        pending.push((kind, hash));
    }
    let mut counts = BTreeMap::<(u8, Hash), u64>::new();
    let mut visited = BTreeSet::new();
    while let Some((kind, hash)) = pending.pop() {
        *counts.entry((kind, hash)).or_default() += 1;
        if !visited.insert((kind, hash)) {
            continue;
        }
        let bytes = tx.get(TABLES[kind as usize], &hash).unwrap().unwrap();
        if kind == 2 {
            let node = BranchNodeCompact::<6>::decode(&bytes).unwrap();
            for slot in 0..16 {
                match node.child(slot) {
                    Some(RootRef::Branch(child)) => pending.push((2, child)),
                    Some(RootRef::Leaf(leaf)) => pending.push((3, leaf.hash)),
                    _ => {}
                }
            }
        } else if kind < 2 {
            let node = BranchNodeCompact::<32>::decode(&bytes).unwrap();
            for slot in 0..16 {
                if let Some(RootRef::Branch(child)) = node.child(slot) {
                    pending.push((kind, child));
                }
            }
        }
    }
    for (kind, table) in TABLES.iter().enumerate() {
        assert_eq!(
            rows(tx, *table)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<BTreeSet<_>>(),
            visited
                .iter()
                .filter(|(k, _)| *k == kind as u8)
                .map(|(_, h)| h.to_vec())
                .collect::<BTreeSet<_>>()
        );
    }
    let expected = counts
        .into_iter()
        .map(|((kind, hash), count)| {
            (
                [vec![kind], hash.to_vec()].concat(),
                count.to_be_bytes().to_vec(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(rows(tx, REFS), expected);
}

fn verify_rebuild(db: &impl Database, sealed: &SealedCommit) {
    let tx = db.begin_read().unwrap();
    let values = rows(&tx, cells::CELL)
        .into_iter()
        .map(|(k, v)| (CellKey::decode(&k).unwrap(), CellValue::parse(v).unwrap()))
        .collect::<Vec<_>>();
    let expected = MemoryDatabase::new();
    super::seal::seed(&expected, &HASH, sealed.commit_id, &values);
    let reference = expected.begin_read().unwrap();
    assert_eq!(
        tx.get(Table("Superblock"), b"head").unwrap(),
        reference.get(Table("Superblock"), b"head").unwrap()
    );
    assert_eq!(rows(&tx, index::INDEX), rows(&reference, index::INDEX));
    for term in Index::new(&HASH)
        .terms_with_prefix(&reference, vec![])
        .unwrap()
    {
        let (term, _) = term.unwrap();
        assert_eq!(
            Index::new(&HASH).bitmap(&tx, &term).unwrap(),
            Index::new(&HASH).bitmap(&reference, &term).unwrap()
        );
    }
    assert_exact_graph(&tx, sealed);
}

fn shared(
    branches: &Branches<MdbxDatabase, Keccak256Hasher>,
    text: &str,
) -> std::sync::Arc<SealedCommit> {
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            for id in [64, 65536, 131072] {
                for name in [b"a", b"b"] {
                    cells.put(key(id, name), value(text, true));
                }
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    let sealed = branches.seal(branch).unwrap();
    branches.commit(branch).unwrap();
    verify_rebuild(branches.database(), &sealed);
    sealed
}

#[test]
fn mdbx_readers_keep_old_graph_and_counts_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = MdbxDatabase::open(dir.path()).unwrap();
        super::seal::seed(&db, &HASH, 0, &[]);
        let branches = Branches::new(db.clone(), HASH).unwrap();
        let first = shared(&branches, "old");
        let snapshot = db.begin_read().unwrap();
        let old_rows = TABLES.map(|t| rows(&snapshot, t));
        let second = shared(&branches, "new");
        let latest = db.begin_read().unwrap();
        assert!(
            latest
                .get(cells::CELL_TRIE, &first.state_root)
                .unwrap()
                .is_none()
        );
        assert!(
            latest
                .get(index::INDEX_TRIE, &first.index_root)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            Cells::new(&HASH)
                .reopen(&snapshot, first.state_root)
                .unwrap(),
            first.cells.root
        );
        assert_eq!(
            Index::new(&HASH)
                .reopen(&snapshot, first.index_root)
                .unwrap(),
            first.index.root
        );
        let term = IndexTerm::new("a", CellType::Str, b"old").unwrap();
        assert_eq!(
            Index::new(&HASH)
                .bitmap(&snapshot, &term)
                .unwrap()
                .unwrap()
                .treemap()
                .iter()
                .collect::<Vec<_>>(),
            vec![64, 65536, 131072]
        );
        assert_eq!(Index::new(&HASH).bitmap(&latest, &term).unwrap(), None);
        assert_eq!(TABLES.map(|t| rows(&snapshot, t)), old_rows);
        assert_exact_graph(&latest, &second);
    }
    let db = MdbxDatabase::open(dir.path()).unwrap();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    assert_eq!(branches.initialize_gc().unwrap(), Default::default());
    shared(&branches, "after-restart");
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            for id in [64, 65536, 131072] {
                for name in [b"a", b"b"] {
                    cells.delete(key(id, name));
                }
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    let sealed = branches.seal(branch).unwrap();
    branches.commit(branch).unwrap();
    verify_rebuild(&db, &sealed);
    let tx = db.begin_read().unwrap();
    for table in [
        index::INDEX,
        index::INDEX_TRIE,
        index::BITMAP_TRIE,
        index::BITMAP_CONTAINER,
    ] {
        assert!(rows(&tx, table).is_empty());
    }
}

#[test]
fn mdbx_first_commit_sweeps_legacy_history_and_restores_reused_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let db = MdbxDatabase::open(dir.path()).unwrap();
    super::seal::seed(&db, &HASH, 0, &[]);
    let mut tx = db.begin_write().unwrap();
    let (mut state, mut index_root) = (RootRef::Empty, RootRef::Empty);
    for round in 0..75 {
        let text = format!("v{round}");
        let before = format!("v{}", round - 1);
        let mut changes = Vec::new();
        let mut postings = Vec::new();
        for id in [64, 65536] {
            for name in ["a", "b"] {
                changes.push(CellChange::Put {
                    key: key(id, name.as_bytes()),
                    value: value(&text, true),
                });
                postings.push(PostingChange::Add {
                    term: IndexTerm::new(name, CellType::Str, text.as_bytes()).unwrap(),
                    record_id: id,
                });
                if round > 0 {
                    postings.push(PostingChange::Remove {
                        term: IndexTerm::new(name, CellType::Str, before.as_bytes()).unwrap(),
                        record_id: id,
                    });
                }
            }
        }
        state = Cells::new(&HASH)
            .apply(&mut tx, state, changes)
            .unwrap()
            .root;
        index_root = Index::new(&HASH)
            .apply(&mut tx, index_root, postings)
            .unwrap()
            .root;
    }
    tx.put(
        Table("Superblock"),
        b"head",
        &[
            75u64.to_be_bytes().as_slice(),
            &state.hash(&HASH),
            &index_root.hash(&HASH),
        ]
        .concat(),
    )
    .unwrap();
    tx.commit().unwrap();
    let snapshot = db.begin_read().unwrap();
    let before = TABLES.map(|t| rows(&snapshot, t));
    assert!(before[0].len() > 50);
    assert!(before[1].len() > 50);
    let branches = Branches::new(db.clone(), HASH).unwrap();
    // Seal can deduplicate v0's physical rows against old history. The first
    // commit must collect that history, then recreate any rows the seal reused.
    shared(&branches, "v0");
    let latest = db.begin_read().unwrap();
    assert!(
        TABLES
            .iter()
            .map(|t| rows(&latest, *t).len())
            .sum::<usize>()
            < 20
    );
    assert_eq!(TABLES.map(|t| rows(&snapshot, t)), before);
    let term = IndexTerm::new("a", CellType::Str, b"v74").unwrap();
    assert_eq!(
        Index::new(&HASH)
            .bitmap(&snapshot, &term)
            .unwrap()
            .unwrap()
            .treemap()
            .iter()
            .collect::<Vec<_>>(),
        vec![64, 65536]
    );
    assert_eq!(branches.initialize_gc().unwrap(), Default::default());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn churn_matches_rebuild_and_has_no_orphans(
        batches in prop::collection::vec(prop::collection::vec(
            (0u8..12, any::<bool>(), 0u8..5, 0u8..4), 0..8), 1..25)
    ) {
        let db = MemoryDatabase::new();
        super::seal::seed(&db, &HASH, 0, &[]);
        let branches = Branches::new(db.clone(), HASH).unwrap();
        for batch in batches {
            let branch = branches.begin().unwrap();
            branches.write(branch, |cells| {
                for &(id, name, text, action) in &batch {
                    let id = (u64::from(id % 3) << 16) + 64 + u64::from(id / 3);
                    let key = key(id, if name { b"a" } else { b"b" });
                    if action == 0 {
                        cells.delete(key);
                    } else {
                        cells.put(key, value(&format!("v{text}"), action > 1));
                    }
                }
                Ok::<_, ()>(())
            }).unwrap();
            let sealed = branches.seal(branch).unwrap();
            branches.commit(branch).unwrap();
            verify_rebuild(&db, &sealed);
        }
    }
}
