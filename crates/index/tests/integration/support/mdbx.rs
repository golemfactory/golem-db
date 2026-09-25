use golemdb_storage::MdbxDatabase;

// Bind the directory before the database so the environment is dropped first.
pub fn db() -> (tempfile::TempDir, MdbxDatabase) {
    let dir = tempfile::tempdir().unwrap();
    let db = MdbxDatabase::open(dir.path()).unwrap();
    (dir, db)
}
