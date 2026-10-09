use std::panic::{AssertUnwindSafe, catch_unwind};

use golemdb_cells::{CellChange, CellKey, CellNameRef, CellType, CellValueRef};
use golemdb_merkle::{Hash, HashProvider, Keccak256Hasher};
use golemdb_storage::{Database, MemoryDatabase, WriteTransaction, scan_prefix};

use crate::{
    BranchError, Branches, create_genesis,
    metadata::{ENGINE_TABLES, Head, SUPERBLOCK, require_format, write_format, write_head},
};

const HASH: Keccak256Hasher = Keccak256Hasher;

fn snapshot(db: &impl Database) -> Vec<Vec<golemdb_storage::Entry>> {
    let tx = db.begin_read().unwrap();
    ENGINE_TABLES
        .into_iter()
        .map(|table| {
            scan_prefix(&tx, table, vec![])
                .unwrap()
                .collect::<golemdb_storage::Result<_>>()
                .unwrap()
        })
        .collect()
}

struct PanicHash;
impl HashProvider for PanicHash {
    fn hash_parts(&self, _: &[&[u8]]) -> Hash {
        panic!("unexpected hashing");
    }
}

struct ReadOnly(MemoryDatabase);
impl Database for ReadOnly {
    type Read<'a> = <MemoryDatabase as Database>::Read<'a>;
    type Write<'a> = <MemoryDatabase as Database>::Write<'a>;
    fn begin_read(&self) -> golemdb_storage::Result<Self::Read<'_>> {
        self.0.begin_read()
    }
    fn begin_write(&self) -> golemdb_storage::Result<Self::Write<'_>> {
        panic!("opening must not acquire a writer");
    }
}

#[test]
fn opening_fresh_format_does_not_write_or_hash() {
    let db = MemoryDatabase::new();
    create_genesis(&db, &HASH, []).unwrap();
    let before = snapshot(&db);
    let branches = Branches::new(ReadOnly(db.clone()), PanicHash).unwrap();
    assert_eq!(branches.head().unwrap(), 0);
    require_format(&db.begin_read().unwrap()).unwrap();
    assert_eq!(snapshot(&db), before);
}

#[test]
fn legacy_head_including_commit_zero_is_rejected_without_touching_history() {
    for commit_id in [0, 50] {
        let db = MemoryDatabase::new();
        let mut tx = db.begin_write().unwrap();
        let empty = HASH.hash(&[]);
        write_head(
            &mut tx,
            &Head {
                commit_id,
                state_root: empty,
                index_root: empty,
            },
        )
        .unwrap();
        // Admission must not scan this malformed legacy row or try to migrate it.
        tx.put(golemdb_cells::tables::CELL_TRIE, b"legacy-key", b"old-data")
            .unwrap();
        tx.commit().unwrap();
        let before = snapshot(&db);
        assert!(matches!(
            Branches::new(ReadOnly(db.clone()), PanicHash),
            Err(BranchError::UnsupportedDatabaseFormat)
        ));
        assert!(matches!(
            create_genesis(&db, &PanicHash, []),
            Err(BranchError::DatabaseNotEmpty(_))
        ));
        assert_eq!(snapshot(&db), before);
    }
}

#[test]
fn creation_rejects_every_nonempty_engine_table_and_unknown_format() {
    for table in ENGINE_TABLES {
        let db = MemoryDatabase::new();
        let mut tx = db.begin_write().unwrap();
        tx.put(table, b"orphan", b"existing-data").unwrap();
        tx.commit().unwrap();
        let before = snapshot(&db);
        assert!(matches!(create_genesis(&db, &PanicHash, []),
            Err(BranchError::DatabaseNotEmpty(found)) if found == table));
        assert_eq!(snapshot(&db), before);
    }
    let db = MemoryDatabase::new();
    create_genesis(&db, &HASH, []).unwrap();
    let mut tx = db.begin_write().unwrap();
    tx.put(SUPERBLOCK, b"format-version", &[2]).unwrap();
    tx.commit().unwrap();
    let before = snapshot(&db);
    assert!(matches!(
        Branches::new(ReadOnly(db.clone()), PanicHash),
        Err(BranchError::UnsupportedDatabaseFormat)
    ));
    assert_eq!(snapshot(&db), before);
}

#[test]
fn invalid_genesis_aborts_cell_writes_and_all_metadata_then_allows_retry() {
    let db = MemoryDatabase::new();
    let change = CellChange::Put {
        key: CellKey::new(64, CellNameRef::raw(b"\xff")),
        value: CellValueRef::new(CellType::Str, b"x", true).unwrap().into(),
    };
    assert!(matches!(
        create_genesis(&db, &HASH, [change]),
        Err(BranchError::Term(_))
    ));
    assert!(snapshot(&db).iter().all(Vec::is_empty));
    create_genesis(&db, &HASH, []).unwrap();
    Branches::new(db, HASH).unwrap();
}

#[test]
fn panicking_genesis_does_not_publish_or_poison_the_writer() {
    let db = MemoryDatabase::new();
    assert!(catch_unwind(AssertUnwindSafe(|| create_genesis(&db, &PanicHash, []))).is_err());
    assert!(snapshot(&db).iter().all(Vec::is_empty));
    db.begin_write().unwrap();
    create_genesis(&db, &HASH, []).unwrap();
    Branches::new(db, HASH).unwrap();
}

#[test]
fn missing_format_after_open_aborts_implicit_and_explicit_publication() {
    for explicit in [false, true] {
        let db = MemoryDatabase::new();
        create_genesis(&db, &HASH, []).unwrap();
        let branches = Branches::new(db.clone(), HASH).unwrap();
        let branch = branches.begin().unwrap();
        if explicit {
            branches.seal(branch).unwrap();
        }
        let mut tx = db.begin_write().unwrap();
        tx.delete(SUPERBLOCK, b"format-version").unwrap();
        tx.commit().unwrap();
        let before = snapshot(&db);
        assert!(matches!(
            branches.commit(branch),
            Err(BranchError::UnsupportedDatabaseFormat)
        ));
        assert_eq!(snapshot(&db), before);
        assert_eq!(branches.head().unwrap(), 0);
        assert_eq!(branches.branch_info(branch).unwrap().sealed, explicit);
        let mut tx = db.begin_write().unwrap();
        write_format(&mut tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(branches.commit(branch).unwrap(), 1);
    }
}
