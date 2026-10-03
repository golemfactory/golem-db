use golemdb_storage::MdbxStore;

// Bind the directory before the store so the environment is dropped first.
pub fn db() -> (tempfile::TempDir, MdbxStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = MdbxStore::open(dir.path()).unwrap();
    (dir, db)
}
