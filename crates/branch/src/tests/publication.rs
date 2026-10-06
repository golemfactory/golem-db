use crate::{BranchError, Branches};
use golemdb_cells::{
    CellChange, CellKey, CellNameRef, CellType, CellValue, CellValueRef, Cells, tables,
};
use golemdb_index::{Index, IndexTerm, PostingChange};
use golemdb_merkle::{Blake3Hasher, Hash, HashProvider, Keccak256Hasher, RootRef};
use golemdb_storage::{Database, MemoryDatabase, Table, WriteTransaction, scan_prefix};
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const SUPERBLOCK: Table = Table("Superblock");
const TABLES: [Table; 7] = [
    tables::CELL,
    tables::CELL_TRIE,
    golemdb_index::tables::INDEX,
    golemdb_index::tables::INDEX_TRIE,
    golemdb_index::tables::BITMAP_TRIE,
    golemdb_index::tables::BITMAP_CONTAINER,
    SUPERBLOCK,
];
fn key(id: u64, name: &[u8]) -> CellKey {
    CellKey::new(id, CellNameRef::raw(name))
}
fn value(text: &str, indexed: bool) -> CellValue {
    CellValueRef::new(CellType::Str, text.as_bytes(), indexed)
        .unwrap()
        .into()
}
fn head(tx: &mut impl WriteTransaction, id: u64, state: Hash, index: Hash) {
    tx.put(
        SUPERBLOCK,
        b"head",
        &[id.to_be_bytes().as_slice(), &state, &index].concat(),
    )
    .unwrap();
}
fn seed(db: &impl Database, hash: &impl HashProvider, id: u64, rows: &[(CellKey, CellValue)]) {
    let mut tx = db.begin_write().unwrap();
    let cells = Cells::new(hash)
        .apply(
            &mut tx,
            RootRef::Empty,
            rows.iter().map(|(key, value)| CellChange::Put {
                key: key.clone(),
                value: value.clone(),
            }),
        )
        .unwrap();
    let postings = rows
        .iter()
        .filter(|(_, value)| value.is_indexable())
        .map(|(key, value)| PostingChange::Add {
            term: IndexTerm::from_cell(
                std::str::from_utf8(key.name().as_bytes()).unwrap(),
                value.as_view(),
            )
            .unwrap()
            .unwrap(),
            record_id: key.record_id(),
        });
    let index = Index::new(hash)
        .apply(&mut tx, RootRef::Empty, postings)
        .unwrap();
    head(&mut tx, id, cells.root.hash(hash), index.root.hash(hash));
    tx.commit().unwrap();
}
fn snapshot(db: &impl Database) -> Vec<Vec<golemdb_storage::Entry>> {
    let tx = db.begin_read().unwrap();
    TABLES
        .iter()
        .map(|table| {
            scan_prefix(&tx, *table, vec![])
                .unwrap()
                .collect::<golemdb_storage::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
}
// A guard fixture: these operations must not even try to open a writer.
struct ReadOnly<D>(D);
impl<D> ReadOnly<D> {
    fn new(inner: D) -> Self {
        Self(inner)
    }
}
impl<D: Database> Database for ReadOnly<D> {
    type Read<'a>
        = D::Read<'a>
    where
        Self: 'a;
    type Write<'a>
        = D::Write<'a>
    where
        Self: 'a;
    fn begin_read(&self) -> golemdb_storage::Result<Self::Read<'_>> {
        self.0.begin_read()
    }
    fn begin_write(&self) -> golemdb_storage::Result<Self::Write<'_>> {
        panic!("operation opened a database writer");
    }
}

struct PanicHasher(Arc<AtomicBool>);
impl HashProvider for PanicHasher {
    fn hash_parts(&self, parts: &[&[u8]]) -> Hash {
        assert!(!self.0.load(Ordering::Relaxed), "injected hash panic");
        Keccak256Hasher.hash_parts(parts)
    }
}

#[test]
fn invalid_attribute_failure_keeps_overlay_and_frames_retryable() {
    let db = MemoryDatabase::new();
    seed(&db, &Keccak256Hasher, 0, &[]);
    let before = snapshot(&db);
    let branches = Branches::new(ReadOnly::new(db.clone()), Keccak256Hasher).unwrap();
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            cells.put(key(64, b"ok"), value("keep", true));
            Ok::<_, ()>(())
        })
        .unwrap();
    branches.checkpoint(branch).unwrap();
    branches
        .write(branch, |cells| {
            cells.put(key(64, b"\xff"), value("invalid attribute", true));
            Ok::<_, ()>(())
        })
        .unwrap();
    let info = branches.branch_info(branch).unwrap();
    assert!(matches!(branches.seal(branch), Err(BranchError::Term(_))));
    assert_eq!(branches.branch_info(branch).unwrap(), info);
    assert_eq!(snapshot(&db), before);
    branches.rollback(branch).unwrap();
    assert_eq!(
        branches
            .read(branch, |cells| cells.get(&key(64, b"ok")))
            .unwrap(),
        Some(value("keep", true))
    );
    assert!(branches.seal(branch).is_ok());
}

#[test]
fn panicking_seal_does_not_freeze_or_poison_the_branch() {
    let db = MemoryDatabase::new();
    seed(&db, &Keccak256Hasher, 0, &[]);
    let fail = Arc::new(AtomicBool::new(true));
    let branches = Branches::new(ReadOnly::new(db), PanicHasher(fail.clone())).unwrap();
    let branch = branches.begin().unwrap();
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| branches.seal(branch))).is_err());
    assert!(!branches.branch_info(branch).unwrap().sealed);
    fail.store(false, Ordering::Relaxed);
    assert!(branches.seal(branch).is_ok());
}

#[test]
fn stale_sealed_branches_and_commit_number_overflow_are_rejected() {
    let db = MemoryDatabase::new();
    seed(&db, &Keccak256Hasher, 0, &[]);
    let branches = Branches::new(ReadOnly::new(db.clone()), Keccak256Hasher).unwrap();
    let branch = branches.begin().unwrap();
    let sealed = branches.seal(branch).unwrap();
    let publisher = Branches::new(db.clone(), Keccak256Hasher).unwrap();
    publisher.commit(publisher.begin().unwrap()).unwrap();
    assert!(matches!(
        branches.seal(branch),
        Err(BranchError::HandleInvalid)
    ));
    // Reopen over an actual branch root and append the next lag-one cell.
    let next = branches.begin().unwrap();
    let second = branches.seal(next).unwrap();
    assert_eq!(
        second.cells.changed_cells[0].key,
        key(2, &1u64.to_be_bytes())
    );
    let mut tx = db.begin_write().unwrap();
    head(&mut tx, u64::MAX, sealed.state_root, sealed.index_root);
    tx.commit().unwrap();
    let last = branches.begin().unwrap();
    assert!(matches!(
        branches.seal(last),
        Err(BranchError::CommitNumberExhausted)
    ));
    assert!(!branches.branch_info(last).unwrap().sealed);
}

#[test]
fn wrong_roots_and_duplicate_lag_one_cell_fail_without_freezing() {
    let db = MemoryDatabase::new();
    seed(&db, &Blake3Hasher, 0, &[]);
    let branches = Branches::new(ReadOnly::new(db), Keccak256Hasher).unwrap();
    let branch = branches.begin().unwrap();
    assert!(matches!(branches.seal(branch), Err(BranchError::Cells(_))));
    assert!(!branches.branch_info(branch).unwrap().sealed);

    let db = MemoryDatabase::new();
    seed(
        &db,
        &Keccak256Hasher,
        0,
        &[(key(2, &0u64.to_be_bytes()), value("already exists", false))],
    );
    let branches = Branches::new(ReadOnly::new(db), Keccak256Hasher).unwrap();
    let branch = branches.begin().unwrap();
    assert!(matches!(
        branches.seal(branch),
        Err(BranchError::RootsCellExists)
    ));
    assert!(!branches.branch_info(branch).unwrap().sealed);
}

fn stage<D: Database, H: HashProvider>(branches: &Branches<D, H>, text: &str) -> u64 {
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            cells.put(key(64, b"name"), value(text, true));
            Ok::<_, ()>(())
        })
        .unwrap();
    branch
}

struct SwitchHash(Arc<AtomicBool>);
impl HashProvider for SwitchHash {
    fn hash_parts(&self, parts: &[&[u8]]) -> Hash {
        assert!(
            !self.0.load(Ordering::Relaxed),
            "commit recalculated a sealed hash"
        );
        Keccak256Hasher.hash_parts(parts)
    }
}

#[test]
fn explicit_seal_is_not_recomputed_by_commit() {
    let db = MemoryDatabase::new();
    seed(&db, &Keccak256Hasher, 0, &[]);
    let fail = Arc::new(AtomicBool::new(false));
    let branches = Branches::new(db, SwitchHash(fail.clone())).unwrap();
    let branch = stage(&branches, "Alice");
    branches.seal(branch).unwrap();
    fail.store(true, Ordering::Relaxed);
    assert_eq!(branches.commit(branch).unwrap(), 1);
}

#[test]
fn implicit_seal_failure_keeps_branch_open_and_never_opens_writer() {
    let db = MemoryDatabase::new();
    seed(&db, &Keccak256Hasher, 0, &[]);
    let controlled = ReadOnly::new(db);
    let branches = Branches::new(controlled, Keccak256Hasher).unwrap();
    let branch = branches.begin().unwrap();
    branches
        .write(branch, |cells| {
            cells.put(key(64, b"\xff"), value("invalid attribute name", true));
            Ok::<_, ()>(())
        })
        .unwrap();
    let info = branches.branch_info(branch).unwrap();
    assert!(matches!(branches.commit(branch), Err(BranchError::Term(_))));
    assert_eq!(branches.branch_info(branch).unwrap(), info);
    branches.rollback(branch).unwrap();
    branches.discard(branch).unwrap();
}
