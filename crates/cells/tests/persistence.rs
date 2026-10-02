#![cfg(feature = "mdbx")]

use golemdb_cells::{CellChange, CellKey, CellNameRef, CellValue, Cells, tables};
use golemdb_merkle::{Hash, Keccak256Hasher, RootRef};
use golemdb_storage::{Database, MdbxDatabase, ReadTransaction, Table, WriteTransaction};

const HASH: Keccak256Hasher = Keccak256Hasher;
const HEAD: Table = Table("TestHead");
fn key(name: &[u8]) -> CellKey {
    CellKey::new(42, CellNameRef::raw(name))
}
fn put(name: &[u8]) -> CellChange {
    CellChange::Put {
        key: key(name),
        value: CellValue::parse(b"\x02value".to_vec()).unwrap(),
    }
}
fn saved_root(tx: &impl ReadTransaction) -> Hash {
    tx.get(HEAD, b"root").unwrap().unwrap().try_into().unwrap()
}

#[test]
fn empty_singleton_and_branch_roots_survive_environment_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let cells = Cells::new(&HASH);
    {
        let db = MdbxDatabase::open(dir.path()).unwrap();
        let mut tx = db.begin_write().unwrap();
        let root = cells
            .apply(&mut tx, RootRef::Empty, [put(b"a")])
            .unwrap()
            .root;
        tx.put(HEAD, b"root", &root.hash(&HASH)).unwrap();
        tx.commit().unwrap();
    }
    {
        let db = MdbxDatabase::open(dir.path()).unwrap();
        let mut tx = db.begin_write().unwrap();
        let root = cells.reopen(&tx, saved_root(&tx)).unwrap();
        assert!(matches!(root, RootRef::Leaf(_)));
        let root = cells.apply(&mut tx, root, [put(b"b")]).unwrap().root;
        assert!(matches!(root, RootRef::Branch(_)));
        tx.put(HEAD, b"root", &root.hash(&HASH)).unwrap();
        tx.put(HEAD, b"old_branch", &root.hash(&HASH)).unwrap();
        tx.commit().unwrap();
    }
    {
        let db = MdbxDatabase::open(dir.path()).unwrap();
        let mut tx = db.begin_write().unwrap();
        let root = cells.reopen(&tx, saved_root(&tx)).unwrap();
        assert!(matches!(root, RootRef::Branch(_)));
        assert_eq!(cells.scan_record(&tx, 42).unwrap().count(), 2);
        let root = cells
            .apply(&mut tx, root, [CellChange::Delete { key: key(b"b") }])
            .unwrap()
            .root;
        tx.put(HEAD, b"root", &root.hash(&HASH)).unwrap();
        tx.commit().unwrap();
    }
    {
        let db = MdbxDatabase::open(dir.path()).unwrap();
        let mut tx = db.begin_write().unwrap();
        let root = cells.reopen(&tx, saved_root(&tx)).unwrap();
        assert!(matches!(root, RootRef::Leaf(_)));
        let root = cells
            .apply(&mut tx, root, [CellChange::Delete { key: key(b"a") }])
            .unwrap()
            .root;
        assert_eq!(root, RootRef::Empty);
        tx.put(HEAD, b"root", &root.hash(&HASH)).unwrap();
        tx.commit().unwrap();
        let mut tx = db.begin_write().unwrap();
        let discarded = cells
            .apply(&mut tx, root, [put(b"uncommitted"), put(b"other")])
            .unwrap()
            .root;
        tx.put(HEAD, b"root", &discarded.hash(&HASH)).unwrap();
        // Drop leaves the empty committed state and its head intact.
    }
    let db = MdbxDatabase::open(dir.path()).unwrap();
    let tx = db.begin_read().unwrap();
    assert_eq!(cells.reopen(&tx, saved_root(&tx)).unwrap(), RootRef::Empty);
    assert!(cells.scan_record(&tx, 42).unwrap().next().is_none());
    let old = tx.get(HEAD, b"old_branch").unwrap().unwrap();
    assert!(tx.get(tables::CELL_TRIE, &old).unwrap().is_some());
}
