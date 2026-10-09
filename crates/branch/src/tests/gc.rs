use golemdb_cells::{
    CellChange, CellKey, CellNameRef, CellType, CellValue, CellValueRef, Cells, tables as cells,
};
use golemdb_index::{Index, PostingChange, tables as index};
use golemdb_merkle::{HashProvider, Keccak256Hasher, RootRef};
use golemdb_storage::{
    Database, MemoryDatabase, ReadTransaction, Table, WriteTransaction, scan_prefix,
};

use crate::{
    BranchError, Branches, gc,
    head::{Head, read_head_state, write_head},
};

const HASH: Keccak256Hasher = Keccak256Hasher;
const REFS: Table = Table("NodeRefs");

fn key(id: u64, name: &str) -> CellKey {
    CellKey::new(id, CellNameRef::raw(name.as_bytes()))
}

fn value(text: &str) -> CellValue {
    CellValueRef::new(CellType::Str, text.as_bytes(), true)
        .unwrap()
        .into()
}

fn database() -> MemoryDatabase {
    let db = MemoryDatabase::new();
    let mut tx = db.begin_write().unwrap();
    let empty = HASH.hash(&[]);
    write_head(
        &mut tx,
        &Head {
            commit_id: 0,
            state_root: empty,
            index_root: empty,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    db
}

fn rows(tx: &impl ReadTransaction, table: Table) -> Vec<golemdb_storage::Entry> {
    scan_prefix(tx, table, Vec::new())
        .unwrap()
        .collect::<golemdb_storage::Result<_>>()
        .unwrap()
}

fn mutate(branches: &Branches<MemoryDatabase, Keccak256Hasher>, values: &[(u64, &str, &str)]) {
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            for &(id, name, text) in values {
                cells.put(key(id, name), value(text));
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    branches.commit(branch).unwrap();
}

fn assert_bounded(db: &MemoryDatabase) {
    let tx = db.begin_read().unwrap();
    let cell_rows = rows(&tx, cells::CELL);
    let terms = rows(&tx, index::INDEX);
    assert!(rows(&tx, cells::CELL_TRIE).len() <= cell_rows.len().saturating_sub(1));
    assert!(rows(&tx, index::INDEX_TRIE).len() <= terms.len().saturating_sub(1));
    let physical = [
        cells::CELL_TRIE,
        index::INDEX_TRIE,
        index::BITMAP_TRIE,
        index::BITMAP_CONTAINER,
    ]
    .into_iter()
    .map(|table| rows(&tx, table).len())
    .sum::<usize>();
    assert_eq!(rows(&tx, REFS).len(), physical);
    assert!(
        rows(&tx, REFS)
            .iter()
            .all(|(_, count)| u64::from_be_bytes(count.as_slice().try_into().unwrap()) > 0)
    );
}

#[test]
fn commits_collect_by_default_and_remain_bounded_under_churn() {
    let db = database();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    for round in 0..80 {
        mutate(
            &branches,
            &[
                (64, "a", if round % 2 == 0 { "x" } else { "y" }),
                (65536, "b", "z"),
            ],
        );
        assert_bounded(&db);
    }
    assert!(gc::initialized(&db.begin_read().unwrap()).unwrap());
    let reopened = Branches::new(db.clone(), HASH).unwrap();
    mutate(&reopened, &[(64, "a", "final")]);
    assert_bounded(&db);
}

#[test]
fn shared_bitmap_roots_and_children_survive_until_last_owner_leaves() {
    let db = database();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    mutate(
        &branches,
        &[
            (64, "a", "x"),
            (65536, "a", "x"),
            (64, "b", "x"),
            (65536, "b", "x"),
        ],
    );
    let snapshot = db.begin_read().unwrap();
    let old_bitmap = rows(&snapshot, index::BITMAP_TRIE);
    assert_eq!(old_bitmap.len(), 1);
    mutate(&branches, &[(64, "a", "y")]);
    let tx = db.begin_read().unwrap();
    assert_eq!(rows(&tx, index::BITMAP_TRIE), old_bitmap);
    let ref_key = [vec![2], old_bitmap[0].0.clone()].concat();
    assert_eq!(
        tx.get(REFS, &ref_key).unwrap(),
        Some(1u64.to_be_bytes().to_vec())
    );
    drop(tx);
    mutate(&branches, &[(64, "b", "y")]);
    let tx = db.begin_read().unwrap();
    assert!(rows(&tx, index::BITMAP_TRIE).is_empty());
    assert_eq!(rows(&tx, index::BITMAP_CONTAINER).len(), 2);
    for (hash, _) in rows(&tx, index::BITMAP_CONTAINER) {
        let ref_key = [vec![3], hash].concat();
        assert_eq!(
            tx.get(REFS, &ref_key).unwrap(),
            Some(2u64.to_be_bytes().to_vec())
        );
    }
    // The old physical branch and all its containers remain in the old snapshot.
    assert_eq!(rows(&snapshot, index::BITMAP_TRIE), old_bitmap);
    assert_bounded(&db);
}

// Build an old-format database using the lower layers, which deliberately retain
// immutable history. Engine collection must bootstrap and sweep this backlog.
fn old_format(db: &MemoryDatabase, rounds: usize) {
    let mut tx = db.begin_write().unwrap();
    let mut cell_root = RootRef::Empty;
    let mut index_root = RootRef::Empty;
    let mut previous = None;
    for round in 0..rounds {
        let value = value(&format!("v{round}"));
        let field = CellValueRef::new(CellType::Str, value.value(), false)
            .unwrap()
            .into();
        let cells = Cells::new(&HASH)
            .apply(
                &mut tx,
                cell_root,
                [
                    CellChange::Put {
                        key: key(64, "a"),
                        value: value.clone(),
                    },
                    CellChange::Put {
                        key: key(65, "b"),
                        value: field,
                    },
                ],
            )
            .unwrap();
        cell_root = cells.root;
        let term = golemdb_index::IndexTerm::from_cell("a", value.as_view())
            .unwrap()
            .unwrap();
        let mut changes = vec![PostingChange::Add {
            term: term.clone(),
            record_id: 64,
        }];
        if let Some(term) = previous.take() {
            changes.push(PostingChange::Remove {
                term,
                record_id: 64,
            });
        }
        index_root = Index::new(&HASH)
            .apply(&mut tx, index_root, changes)
            .unwrap()
            .root;
        previous = Some(term);
    }
    write_head(
        &mut tx,
        &Head {
            commit_id: rounds as u64,
            state_root: cell_root.hash(&HASH),
            index_root: index_root.hash(&HASH),
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn bootstrap_sweeps_multiple_batches_without_changing_head_or_flat_data() {
    let db = database();
    old_format(&db, 600);
    let snapshot = db.begin_read().unwrap();
    let before = rows(&snapshot, cells::CELL_TRIE);
    assert!(before.len() > 512);
    let head = read_head_state(&snapshot).unwrap();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    let stats = branches.initialize_gc().unwrap();
    assert!(stats.removed_rows > 512);
    assert!(stats.removed_bytes > 0);
    assert_eq!(branches.head().unwrap(), head.commit_id);
    let tx = db.begin_read().unwrap();
    assert_eq!(rows(&tx, cells::CELL), rows(&snapshot, cells::CELL));
    assert_eq!(rows(&tx, index::INDEX), rows(&snapshot, index::INDEX));
    assert_eq!(read_head_state(&tx).unwrap().state_root, head.state_root);
    assert_eq!(read_head_state(&tx).unwrap().index_root, head.index_root);
    assert_eq!(rows(&snapshot, cells::CELL_TRIE), before);
    assert_eq!(branches.initialize_gc().unwrap(), Default::default());
    assert_bounded(&db);
}

#[test]
fn a_seal_made_before_bootstrap_is_refreshed_under_the_publication_writer() {
    for maintenance in [false, true] {
        let db = database();
        old_format(&db, 20);
        let branches = Branches::new(db.clone(), HASH).unwrap();
        let branch = branches.begin().unwrap();
        branches
            .write(branch, |cells| {
                // Reuse a historical bitmap singleton (and possibly interior hashes).
                cells.put(key(64, "a"), value("v0"));
                Ok::<_, ()>(())
            })
            .unwrap();
        let sealed = branches.seal(branch).unwrap();
        if maintenance {
            let other = Branches::new(db.clone(), HASH).unwrap();
            other.initialize_gc().unwrap();
        }
        assert_eq!(branches.commit(branch).unwrap(), sealed.commit_id);
        let head = read_head_state(&db.begin_read().unwrap()).unwrap();
        assert_eq!(head.state_root, sealed.state_root);
        assert_eq!(head.index_root, sealed.index_root);
        assert_bounded(&db);
    }
}

#[test]
fn invalid_legacy_graph_aborts_bootstrap_before_publishing_metadata_or_deletions() {
    let db = database();
    old_format(&db, 20);
    let mut tx = db.begin_write().unwrap();
    let live_hash = rows(&tx, index::INDEX)[0].1.clone();
    tx.put(index::BITMAP_CONTAINER, &live_hash, b"invalid")
        .unwrap();
    tx.commit().unwrap();
    let before = db.begin_read().unwrap();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    assert!(matches!(
        branches.initialize_gc(),
        Err(BranchError::GarbageCollection(_))
    ));
    let after = db.begin_read().unwrap();
    for table in [
        cells::CELL_TRIE,
        index::INDEX_TRIE,
        index::BITMAP_TRIE,
        index::BITMAP_CONTAINER,
        REFS,
        Table("Superblock"),
    ] {
        assert_eq!(rows(&before, table), rows(&after, table));
    }
    assert!(!gc::initialized(&after).unwrap());
}

#[test]
fn corrupt_reference_count_aborts_publication_and_keeps_the_seal_for_retry() {
    let db = database();
    let branches = Branches::new(db.clone(), HASH).unwrap();
    mutate(&branches, &[(64, "a", "x"), (65, "b", "y")]);
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            cells.put(key(64, "a"), value("z"));
            Ok::<_, ()>(())
        })
        .unwrap();
    let sealed = branches.seal(branch).unwrap();
    let head = read_head_state(&db.begin_read().unwrap()).unwrap();
    let ref_key = [vec![0], head.state_root.to_vec()].concat();
    let mut tx = db.begin_write().unwrap();
    let original = tx.get(REFS, &ref_key).unwrap().unwrap();
    tx.put(REFS, &ref_key, &[0; 8]).unwrap();
    tx.commit().unwrap();
    let before = db.begin_read().unwrap();
    assert!(matches!(
        branches.commit(branch),
        Err(BranchError::GarbageCollection(_))
    ));
    let after = db.begin_read().unwrap();
    for table in [
        cells::CELL,
        cells::CELL_TRIE,
        index::INDEX,
        index::INDEX_TRIE,
        index::BITMAP_TRIE,
        index::BITMAP_CONTAINER,
        REFS,
        Table("Superblock"),
    ] {
        assert_eq!(rows(&before, table), rows(&after, table));
    }
    assert!(branches.branch_info(branch).unwrap().sealed);
    let mut tx = db.begin_write().unwrap();
    tx.put(REFS, &ref_key, &original).unwrap();
    tx.commit().unwrap();
    assert_eq!(branches.commit(branch).unwrap(), sealed.commit_id);
    assert_bounded(&db);
}
