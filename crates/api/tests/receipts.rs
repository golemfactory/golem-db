//! Receipts: present on success and failure, and the details of each write
//! path counted as in the metering spec (D4).
//!
//! `i32` cells encode in 5 bytes (tag + 4), so a cell named `price` counts
//! (8 + 5) + 5 = 18 cell bytes and, as an attribute, 5 + 2 + 4 = 11 index bytes.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use golemdb_api::*;
use golemdb_storage::{MemoryStore, Store};

const KEY: RecordKey = RecordKey([0x42; 32]);

fn db() -> (Database, BranchId) {
    let db = Database::open_memory(&Genesis::DEV).unwrap();
    let branch = db.begin().unwrap();
    (db, branch)
}

/// Details with the given counts; all other counts zero.
fn details(set: impl FnOnce(&mut Details)) -> Details {
    let mut details = Details::default();
    set(&mut details);
    details
}

#[test]
fn create_counts_every_user_cell_and_its_index_entry() {
    let (db, branch) = db();
    let created = db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .attribute("price", 50i32)
            .field("stock", 5i32),
    );
    assert_eq!(created.receipt.cost, 0);
    assert_eq!(created.receipt.priced_at, Some(0));
    assert_eq!(
        created.receipt.details,
        details(|d| {
            d.cells_created = 2;
            d.cell_bytes_written = 18 + 18;
            d.index_joins = 1;
            d.index_bytes_written = 11;
        })
    );
    assert_eq!(created.into_result().unwrap(), KEY);
}

#[test]
fn patch_counts_effects_not_requests() {
    let (db, branch) = db();
    db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .attribute("price", 50i32)
            .field("stock", 5i32)
            .field("note", 1i32),
    )
    .into_result()
    .unwrap();

    let patched = db.patch(
        branch,
        RecordOp::patch(KEY)
            .attribute("price", 75i32) // changed attribute: leave and join
            .field("stock", 5i32) // identical: nothing
            .field("added", 1i32) // new field
            .remove("note") // present: deleted
            .remove("absent"), // absent: nothing
    );
    assert_eq!(
        patched.receipt.details,
        details(|d| {
            d.cells_created = 1;
            d.cells_updated = 1;
            d.cells_deleted = 1;
            d.cell_bytes_written = 18 + 18; // price, added
            d.cell_bytes_deleted = 18 + 17; // old price, note
            d.index_joins = 1;
            d.index_leaves = 1;
            d.index_bytes_written = 11;
            d.index_bytes_deleted = 11;
        })
    );
    patched.into_result().unwrap();

    // Changing only the kind: the cell is updated and leaves the index.
    let demoted = db.patch(branch, RecordOp::patch(KEY).field("price", 75i32));
    assert_eq!(
        demoted.receipt.details,
        details(|d| {
            d.cells_updated = 1;
            d.cell_bytes_written = 18;
            d.cell_bytes_deleted = 18;
            d.index_leaves = 1;
            d.index_bytes_deleted = 11;
        })
    );
}

#[test]
fn delete_counts_user_cells_but_not_system_cells() {
    let (db, branch) = db();
    db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .attribute("price", 50i32)
            .field("stock", 5i32),
    )
    .into_result()
    .unwrap();
    let deleted = db.delete(branch, RecordOp::delete(KEY));
    assert_eq!(
        deleted.receipt.details,
        details(|d| {
            d.cells_deleted = 2;
            d.cell_bytes_deleted = 18 + 18;
            d.index_leaves = 1;
            d.index_bytes_deleted = 11;
        })
    );
    deleted.into_result().unwrap();
}

#[test]
fn failed_calls_have_receipts_without_effects() {
    let (db, branch) = db();
    let missing = RecordKey([9; 32]);
    let on_branch = [
        db.patch(branch, RecordOp::patch(missing).field("price", 1i32))
            .receipt,
        db.delete(branch, RecordOp::delete(missing)).receipt,
        db.get(ReadTarget::Branch(branch), RecordOp::get(missing))
            .receipt,
        db.create(branch, RecordOp::create().field("price", 1i32))
            .receipt,
    ];
    for receipt in on_branch {
        assert_eq!(receipt.cost, 0);
        assert_eq!(receipt.details, Details::default());
        assert_eq!(receipt.priced_at, Some(0)); // the branch's origin
    }
    // An unknown handle has no origin: no commit is made up.
    let unknown = db.create(99, RecordOp::create().key(KEY).field("price", 1i32));
    assert_eq!(unknown.receipt.priced_at, None);
    assert_eq!(unknown.receipt.details, Details::default());
}

#[test]
fn reads_are_priced_at_the_commit_they_read() {
    let (db, branch) = db();
    db.create(branch, RecordOp::create().key(KEY).field("price", 1i32))
        .into_result()
        .unwrap();
    db.commit(branch).unwrap();
    let next = db.begin().unwrap();
    for target in [
        ReadTarget::Head,
        ReadTarget::Commit(1),
        ReadTarget::Branch(next),
    ] {
        let read = db.get(target, RecordOp::get(KEY));
        assert_eq!(read.receipt.priced_at, Some(1));
        assert_eq!(read.receipt.details, Details::default());
        read.into_result().unwrap();
    }
    // A write on the new branch is priced at its base commit.
    let patched = db.patch(next, RecordOp::patch(KEY).field("price", 2i32));
    assert_eq!(patched.receipt.priced_at, Some(1));
}

/// A hook to run when the given read snapshot is opened.
type Hook = Arc<Mutex<Option<(usize, Box<dyn FnOnce() + Send>)>>>;

/// A store that counts read snapshots and can run a hook when the n-th one
/// is opened, before it is taken.
struct HookStore {
    store: MemoryStore,
    reads: Arc<AtomicUsize>,
    hook: Hook,
}

impl Store for HookStore {
    type Read<'a> = <MemoryStore as Store>::Read<'a>;
    type Write<'a> = <MemoryStore as Store>::Write<'a>;
    fn max_key_size(&self) -> usize {
        self.store.max_key_size()
    }
    fn max_value_size(&self) -> usize {
        self.store.max_value_size()
    }
    fn begin_read(&self) -> golemdb_storage::Result<Self::Read<'_>> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        let hook = {
            let mut slot = self.hook.lock().unwrap();
            match slot.take() {
                Some((at, hook)) if at == read => Some(hook),
                other => {
                    *slot = other;
                    None
                }
            }
        };
        if let Some(hook) = hook {
            hook();
        }
        self.store.begin_read()
    }
    fn begin_write(&self) -> golemdb_storage::Result<Self::Write<'_>> {
        self.store.begin_write()
    }
}

struct Hooked {
    db: Database,
    reads: Arc<AtomicUsize>,
    hook: Hook,
}

fn hooked() -> Hooked {
    let reads = Arc::new(AtomicUsize::new(0));
    let hook = Arc::new(Mutex::new(None));
    let store = HookStore {
        store: MemoryStore::new(),
        reads: reads.clone(),
        hook: hook.clone(),
    };
    let db = Database::from_store(store, &Config::new(Genesis::DEV)).unwrap();
    Hooked { db, reads, hook }
}

impl Hooked {
    /// Snapshots opened by `call`.
    fn snapshots<T>(&self, call: impl FnOnce() -> T) -> (T, usize) {
        let before = self.reads.load(Ordering::SeqCst);
        let result = call();
        (result, self.reads.load(Ordering::SeqCst) - before)
    }

    /// Run `hook` when the call's `nth` snapshot (counting from 1) is opened.
    fn arm(&self, nth: usize, hook: impl FnOnce() + Send + 'static) {
        let at = self.reads.load(Ordering::SeqCst) + nth;
        *self.hook.lock().unwrap() = Some((at, Box::new(hook)));
    }
}

#[test]
fn building_a_receipt_never_invalidates_the_branch() {
    let h = hooked();
    let a = h.db.begin().unwrap();
    let b = h.db.begin().unwrap();
    h.db.create(b, RecordOp::create().key(RecordKey([2; 32])))
        .into_result()
        .unwrap();
    // Commit b at the snapshot a receipt would open after the write. Before
    // the fix, that snapshot found a stale and removed it from the registry.
    let other = h.db.clone();
    h.arm(2, move || assert_eq!(other.commit(b).unwrap(), 1));
    let created = h.db.create(a, RecordOp::create().key(KEY));
    assert_eq!(created.receipt.priced_at, Some(0)); // a's origin, not the new head
    created.into_result().unwrap();
    // The receipt opened no snapshot, so the hook is still armed: disarm it
    // and commit b now, after a's write.
    assert!(h.hook.lock().unwrap().take().is_some());
    assert_eq!(h.db.commit(b).unwrap(), 1);
    // a lost the race, and learns it from commit.
    assert!(matches!(h.db.commit(a), Err(ApiError::Conflict)));
}

#[test]
fn record_calls_take_one_snapshot_each() {
    let h = hooked();
    let branch = h.db.begin().unwrap();
    let (created, reads) = h.snapshots(|| {
        h.db.create(branch, RecordOp::create().key(KEY).field("price", 1i32))
    });
    created.into_result().unwrap();
    assert_eq!(reads, 1);
    let (patched, reads) = h.snapshots(|| {
        h.db.patch(branch, RecordOp::patch(KEY).field("price", 2i32))
    });
    patched.into_result().unwrap();
    assert_eq!(reads, 1);
    h.db.commit(branch).unwrap();
    // A head read's receipt comes from the snapshot the record was read from.
    let (read, reads) = h.snapshots(|| h.db.get(ReadTarget::Head, RecordOp::get(KEY)));
    assert_eq!(reads, 1);
    assert_eq!(read.receipt.priced_at, Some(1));
    assert_eq!(
        read.into_result().unwrap().cells[b"price".as_slice()].as_i32(),
        Some(2)
    );
}
