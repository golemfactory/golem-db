//! `#meta`: written by every mutation, always equal to a recount of the
//! record's user cells, and present on empty records.

use golemdb_api::*;

const KEY: RecordKey = RecordKey([0x42; 32]);

fn db() -> (Database, BranchId) {
    let db = Database::open_memory(&Genesis::DEV).unwrap();
    let branch = db.begin().unwrap();
    (db, branch)
}

/// The D4 counts recomputed from a full read, independently of `#meta`.
fn recount(record: &Record) -> RecordMeta {
    let mut meta = RecordMeta::default();
    for (name, value) in &record.cells {
        let name = name.as_bytes();
        if name.starts_with(b"#") || name.starts_with(b"@") {
            continue; // system cells are not counted
        }
        meta.cells += 1;
        meta.cell_bytes += (8 + name.len() + value.encoded_bytes().len()) as u64;
        if value.is_indexable() {
            meta.indexed_cells += 1;
            meta.index_bytes += (name.len() + 2 + value.value().len()) as u64;
        }
    }
    meta
}

/// Reads the record and checks that `#meta` matches a recount; returns it.
fn checked_meta(db: &Database, target: ReadTarget) -> RecordMeta {
    let record = db.get(target, RecordOp::get(KEY)).into_result().unwrap();
    let meta = record.meta().expect("a full read includes #meta");
    assert_eq!(meta, recount(&record));
    meta
}

#[test]
fn meta_tracks_every_mutation_path() {
    let (db, branch) = db();
    let target = ReadTarget::Branch(branch);
    db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .attribute("price", 50i32)
            .attribute("name", "Laptop")
            .field("stock", 5u32),
    )
    .into_result()
    .unwrap();
    let created = checked_meta(&db, target);
    assert_eq!((created.cells, created.indexed_cells), (3, 2));

    for op in [
        RecordOp::patch(KEY).attribute("price", 60i32), // changed attribute
        RecordOp::patch(KEY).field("stock", 5u32),      // identical
        RecordOp::patch(KEY).field("price", 60i32),     // kind change only
        RecordOp::patch(KEY).field("note", "a longer note"), // new cell
        RecordOp::patch(KEY).remove("absent"),          // absent: no-op
        RecordOp::patch(KEY).remove("name").attribute("tag", 7i64), // mixed
    ] {
        db.patch(branch, op).into_result().unwrap();
        checked_meta(&db, target);
    }
    assert_eq!(checked_meta(&db, target).indexed_cells, 1); // only tag

    // Rollback restores #meta with the cells.
    let before = checked_meta(&db, target);
    db.checkpoint(branch).unwrap();
    db.patch(branch, RecordOp::patch(KEY).remove("note").remove("tag"))
        .into_result()
        .unwrap();
    assert_ne!(checked_meta(&db, target), before);
    db.rollback(branch).unwrap();
    assert_eq!(checked_meta(&db, target), before);

    // Committed state has the same #meta.
    db.commit(branch).unwrap();
    assert_eq!(checked_meta(&db, ReadTarget::Head), before);
}

#[test]
fn empty_records_exist_until_deleted() {
    let (db, branch) = db();
    let target = ReadTarget::Branch(branch);
    let created = db.create(branch, RecordOp::create().key(KEY));
    assert_eq!(created.receipt.details, Details::default());
    created.into_result().unwrap();
    assert_eq!(checked_meta(&db, target), RecordMeta::default());
    let record = db.get(target, RecordOp::get(KEY)).into_result().unwrap();
    assert_eq!(record.cells.len(), 2); // #key and #meta

    // Emptying a record keeps it.
    db.patch(branch, RecordOp::patch(KEY).field("price", 1i32))
        .into_result()
        .unwrap();
    db.patch(branch, RecordOp::patch(KEY).remove("price"))
        .into_result()
        .unwrap();
    assert_eq!(checked_meta(&db, target), RecordMeta::default());

    // Only delete removes it, #meta included.
    db.delete(branch, RecordOp::delete(KEY))
        .into_result()
        .unwrap();
    assert!(matches!(
        db.get(target, RecordOp::get(KEY)).into_result(),
        Err(ApiError::NotFound)
    ));
    db.commit(branch).unwrap();
    assert!(matches!(
        db.get(ReadTarget::Head, RecordOp::get(KEY)).into_result(),
        Err(ApiError::NotFound)
    ));
}

#[test]
fn meta_is_readable_by_projection_and_has_a_fixed_encoding() {
    let (db, branch) = db();
    db.create(
        branch,
        RecordOp::create().key(KEY).attribute("price", 50i32),
    )
    .into_result()
    .unwrap();
    let only_meta = db
        .get(
            ReadTarget::Branch(branch),
            RecordOp::get(KEY).only(["#meta"]),
        )
        .into_result()
        .unwrap();
    let meta = only_meta.meta().unwrap();
    // `price` = 50i32 as an attribute: 18 cell bytes, 11 index bytes.
    assert_eq!(
        meta,
        RecordMeta {
            cells: 1,
            cell_bytes: 18,
            indexed_cells: 1,
            index_bytes: 11,
        }
    );
    // Four u64 big-endian counts, stored as a non-indexed bytes32 cell.
    let value = &only_meta.cells[b"#meta".as_slice()];
    assert!(!value.is_indexable());
    let mut expected = [0u8; 32];
    expected[7] = 1;
    expected[15] = 18;
    expected[23] = 1;
    expected[31] = 11;
    assert_eq!(value.as_bytes32(), Some(expected));
    assert_eq!(RecordMeta::from_value(&meta.to_value()), Some(meta));
    // A projection without #meta has no metadata.
    let price_only = db
        .get(
            ReadTarget::Branch(branch),
            RecordOp::get(KEY).only(["price"]),
        )
        .into_result()
        .unwrap();
    assert_eq!(price_only.meta(), None);
}
