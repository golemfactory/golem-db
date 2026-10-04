use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use golemdb_api::*;
use golemdb_storage::{MemoryStore, StorageError, Store};

struct FailingStore<S> {
    store: S,
    fail_reads: Arc<AtomicBool>,
    fail_writes: Arc<AtomicBool>,
}

impl<S: Store> Store for FailingStore<S> {
    type Read<'a>
        = S::Read<'a>
    where
        Self: 'a;
    type Write<'a>
        = S::Write<'a>
    where
        Self: 'a;
    fn max_key_size(&self) -> usize {
        self.store.max_key_size()
    }
    fn max_value_size(&self) -> usize {
        self.store.max_value_size()
    }
    fn begin_read(&self) -> golemdb_storage::Result<Self::Read<'_>> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(injected());
        }
        self.store.begin_read()
    }
    fn begin_write(&self) -> golemdb_storage::Result<Self::Write<'_>> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(injected());
        }
        self.store.begin_write()
    }
}

fn injected() -> StorageError {
    StorageError::Implementation(std::io::Error::other("injected facade storage failure").into())
}

fn diagnostic(error: ApiError) {
    assert!(matches!(error, ApiError::Internal { .. }));
    assert!(
        error
            .to_string()
            .contains("injected facade storage failure")
    );
    let mut source = error.source();
    while let Some(cause) = source {
        if cause.downcast_ref::<std::io::Error>().is_some() {
            return;
        }
        source = cause.source();
    }
    panic!("the store diagnostic must survive the facade's error conversion");
}

fn failure_contract(store: impl Store + Send + Sync + 'static) {
    let fail_reads = Arc::new(AtomicBool::new(false));
    let fail_writes = Arc::new(AtomicBool::new(false));
    let config = Config::new(Genesis {
        hash_function: HashAlgorithm::Blake3,
        ..Genesis::DEV
    });
    let db = Database::from_store(
        FailingStore {
            store,
            fail_reads: fail_reads.clone(),
            fail_writes: fail_writes.clone(),
        },
        &config,
    )
    .unwrap();
    let key = RecordKey([0x42; 32]);
    let branch = db.begin().unwrap();
    db.create(
        branch,
        key,
        RecordInput::new()
            .attribute("price", CellValue::from_i32(50))
            .unwrap(),
    )
    .unwrap();
    let pending = db
        .get(ReadTarget::Branch(branch), key, Projection::All)
        .unwrap();
    let info = db.branch_info(branch).unwrap();
    fail_reads.store(true, Ordering::SeqCst);
    diagnostic(db.get(ReadTarget::Head, key, Projection::All).unwrap_err());
    diagnostic(
        db.patch(branch, key, PatchInput::new().remove("price").unwrap())
            .unwrap_err(),
    );
    diagnostic(db.head().unwrap_err());
    fail_reads.store(false, Ordering::SeqCst);
    assert_eq!(db.branch_info(branch).unwrap(), info);
    assert_eq!(
        db.get(ReadTarget::Branch(branch), key, Projection::All)
            .unwrap(),
        pending
    );

    fail_writes.store(true, Ordering::SeqCst);
    diagnostic(db.commit(branch).unwrap_err());
    assert_eq!(db.head().unwrap(), 0);
    assert!(db.branch_info(branch).unwrap().sealed);
    let sealed = db.seal(branch).unwrap();
    fail_writes.store(false, Ordering::SeqCst);
    assert_eq!(db.commit(branch).unwrap(), sealed.commit_id);
    assert_eq!(
        db.get(ReadTarget::Head, key, Projection::All).unwrap(),
        pending
    );
}

#[test]
fn memory_facade_retains_error_sources_and_can_retry_failed_commit() {
    failure_contract(MemoryStore::new());
}

#[cfg(feature = "mdbx")]
#[test]
fn mdbx_facade_retains_error_sources_and_can_retry_failed_commit() {
    let dir = tempfile::tempdir().unwrap();
    failure_contract(golemdb_storage::MdbxStore::open(dir.path()).unwrap());
}
