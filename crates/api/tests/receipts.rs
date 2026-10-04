//! Receipts: present on success and failure, and the details of each write
//! path counted as in the metering spec (D4).
//!
//! `i32` cells encode in 5 bytes (tag + 4), so a cell named `price` counts
//! (8 + 5) + 5 = 18 cell bytes and, as an attribute, 5 + 2 + 4 = 11 index bytes.

use golemdb_api::*;

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
    assert_eq!(created.receipt.priced_at, 0);
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
    let outcomes = [
        db.patch(branch, RecordOp::patch(missing).field("price", 1i32))
            .receipt,
        db.delete(branch, RecordOp::delete(missing)).receipt,
        db.get(ReadTarget::Branch(branch), RecordOp::get(missing))
            .receipt,
        db.create(branch, RecordOp::create().field("price", 1i32))
            .receipt,
        db.create(99, RecordOp::create().key(KEY).field("price", 1i32))
            .receipt,
    ];
    for receipt in outcomes {
        assert_eq!(receipt.cost, 0);
        assert_eq!(receipt.details, Details::default());
        assert_eq!(receipt.priced_at, 0);
    }
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
        assert_eq!(read.receipt.priced_at, 1);
        assert_eq!(read.receipt.details, Details::default());
        read.into_result().unwrap();
    }
    // A write on the new branch is priced at its base commit.
    let patched = db.patch(next, RecordOp::patch(KEY).field("price", 2i32));
    assert_eq!(patched.receipt.priced_at, 1);
}
