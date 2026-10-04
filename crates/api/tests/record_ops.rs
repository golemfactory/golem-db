//! `RecordOp`: kinds and values, validation reported by the call, keys,
//! projections and the accessors mocks use.

use std::error::Error;

use golemdb_api::*;
use golemdb_cells::{CellNameError, CellParseError};

const KEY: RecordKey = RecordKey([0x42; 32]);

fn db() -> (Database, BranchId) {
    let db = Database::open_memory(&Genesis::DEV).unwrap();
    let branch = db.begin().unwrap();
    (db, branch)
}

fn invalid<T: std::fmt::Debug>(result: Result<T>) -> bool {
    matches!(result, Err(ApiError::InvalidArgument { .. }))
}

#[test]
fn kinds_come_from_the_method_and_types_from_the_rust_value() {
    let indexed = CellValue::from_str("kept")
        .with_kind(CellKind::Attribute)
        .unwrap();
    let op = RecordOp::create()
        .key(KEY)
        .attribute("price", 50i32)
        .field("description", indexed.clone())
        .field("payload", &[0u8, 255][..])
        .field("large", 50i64);
    assert_eq!(
        op.value("price").unwrap().encoded_bytes(),
        &[0x90, 0x80, 0, 0, 50]
    );
    // A CellValue keeps its type and gets the method's kind.
    assert_eq!(op.value("description").unwrap().kind(), CellKind::Field);
    assert_eq!(op.value("description").unwrap().as_str(), Some("kept"));
    assert_eq!(
        op.value("payload").unwrap().as_bytes(),
        Some([0, 255].as_slice())
    );
    // The Rust type decides the cell type.
    assert_eq!(op.value("large").unwrap().as_i64(), Some(50));
    assert_eq!(op.value("large").unwrap().as_i32(), None);
    assert_eq!(op.value("missing"), None);
}

#[test]
fn invalid_values_and_names_are_reported_by_the_call() {
    let (db, branch) = db();
    for op in [
        RecordOp::create().key(KEY).attribute("payload", &b""[..]),
        RecordOp::create().key(KEY).field("ratio", f64::NAN),
        RecordOp::create()
            .key(KEY)
            .field("price", 1i32)
            .field("price", 2i32),
        RecordOp::create()
            .key(KEY)
            .field("price", 1i32)
            .attribute("price", 2i32),
        RecordOp::create().key(KEY).key(RecordKey([1; 32])),
    ] {
        assert!(invalid(db.create(branch, op).into_result()));
    }
    for name in ["", "#key", "@admin", "$", "1name", "a\0b", "a b", "é"] {
        assert!(invalid(
            db.create(branch, RecordOp::create().key(KEY).field(name, true))
                .into_result()
        ));
    }
    // Grammar is checked in the builder; the deployment's name length at the call.
    assert!(invalid(
        db.create(
            branch,
            RecordOp::create().key(KEY).field(&"a".repeat(33), true)
        )
        .into_result()
    ));
    // Names are case-sensitive, and a leading `$` is ordinary syntax.
    db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .field("Price", 1i32)
            .field("price", 2i32)
            .field("$owner", 1u64),
    )
    .into_result()
    .unwrap();
    for op in [
        RecordOp::patch(KEY).field("price", 3i32).remove("price"),
        RecordOp::patch(KEY).remove("price").remove("price"),
    ] {
        assert!(invalid(db.patch(branch, op).into_result()));
    }
    // Nothing was applied by the failed patches.
    let record = db
        .get(ReadTarget::Branch(branch), RecordOp::get(KEY))
        .into_result()
        .unwrap();
    assert_eq!(record.cells[b"price".as_slice()].as_i32(), Some(2));
}

#[test]
fn a_create_without_a_key_does_not_match_caller_assigned_keys() {
    let (db, branch) = db();
    assert!(matches!(
        db.create(branch, RecordOp::create().field("price", 1i32))
            .into_result(),
        Err(ApiError::KeyModeMismatch)
    ));
}

#[test]
fn get_projections_take_raw_names_and_an_empty_one_checks_existence() {
    let (db, branch) = db();
    db.create(
        branch,
        RecordOp::create()
            .key(KEY)
            .field("price", 1i32)
            .field("name", "x"),
    )
    .into_result()
    .unwrap();
    let target = ReadTarget::Branch(branch);
    let record = db
        .get(
            target,
            RecordOp::get(KEY).only([b"#key".as_slice(), b"price", b"price", b"\0"]),
        )
        .into_result()
        .unwrap();
    let names: Vec<&[u8]> = record.cells.keys().map(CellName::as_bytes).collect();
    assert_eq!(names, [b"#key".as_slice(), b"price"]);

    let empty = RecordOp::get(KEY).only(std::iter::empty::<&str>());
    assert!(
        db.get(target, empty)
            .into_result()
            .unwrap()
            .cells
            .is_empty()
    );
    let missing = RecordOp::get(RecordKey([9; 32])).only(std::iter::empty::<&str>());
    assert!(matches!(
        db.get(target, missing).into_result(),
        Err(ApiError::NotFound)
    ));
}

#[test]
fn accessors_expose_the_operation_for_mocks() {
    let create = RecordOp::create().key(KEY).field("price", 1i32).budget(100);
    assert_eq!(create.record_key(), Some(KEY));
    assert_eq!(create.max_cost(), Some(100));
    assert_eq!(RecordOp::create().record_key(), None);
    let patch = RecordOp::patch(KEY)
        .remove("description")
        .field("price", 2i32);
    assert!(patch.removes("description"));
    assert!(!patch.removes("price"));
    assert_eq!(patch.value("price").and_then(CellValue::as_i32), Some(2));
    assert_eq!(RecordOp::get(KEY).record_key(), Some(KEY));
    assert_eq!(RecordOp::delete(KEY).max_cost(), None);
}

#[test]
fn a_budget_is_accepted_but_not_enforced_yet() {
    let (db, branch) = db();
    let created = db.create(
        branch,
        RecordOp::create().key(KEY).field("price", 1i32).budget(0),
    );
    assert_eq!(created.receipt.cost, 0);
    assert_eq!(created.into_result().unwrap(), KEY);
}

#[test]
fn build_errors_name_the_step_and_keep_the_cause() {
    let message = |op: std::result::Result<(), ApiError>| match op {
        Err(ApiError::InvalidArgument { message, source }) => {
            (message, source.map(|source| source.to_string()))
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    };
    let error = RecordOp::create()
        .key(KEY)
        .field("price", 1i32)
        .attribute("pri ce", 2i32)
        .field("1st", 3i32)
        .validate()
        .unwrap_err();
    // The first invalid step is kept; its typed cause is the source.
    assert_eq!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<CellNameError>()),
        Some(&CellNameError::InvalidByte { at: 3, byte: b' ' })
    );
    assert_eq!(
        message(Err(error)).0,
        r#"attribute("pri ce"): invalid cell name"#
    );
    let (text, source) = message(
        RecordOp::create()
            .key(KEY)
            .attribute("payload", &b"x"[..])
            .validate(),
    );
    assert_eq!(text, r#"attribute("payload"): invalid cell value"#);
    assert_eq!(source, Some(CellParseError::NotIndexable.to_string()));
    assert_eq!(
        message(
            RecordOp::patch(KEY)
                .field("price", 1i32)
                .remove("price")
                .validate()
        ),
        (
            r#"remove("price"): the operation already writes or removes this cell"#.into(),
            None
        )
    );
    assert_eq!(
        message(RecordOp::create().key(KEY).key(KEY).validate()).0,
        "key(): the record key is given twice"
    );
    assert!(
        RecordOp::create()
            .key(KEY)
            .field("price", 1i32)
            .validate()
            .is_ok()
    );
}

#[test]
fn operations_are_values_that_can_be_cloned_and_compared() {
    let (db, branch) = db();
    let op = RecordOp::create().key(KEY).field("price", 1i32).budget(10);
    assert_eq!(op.clone(), op);
    assert_ne!(op.clone().field("name", "x"), op);
    db.create(branch, op.clone()).into_result().unwrap();
    // Repeating the same operation, as after a Conflict, is the same call.
    assert!(matches!(
        db.create(branch, op).into_result(),
        Err(ApiError::AlreadyExists)
    ));
    // A kept build error is part of the value too.
    let broken = RecordOp::patch(KEY).field("", 1i32);
    assert_eq!(broken.clone(), broken);
    assert_ne!(broken, RecordOp::patch(KEY));
    assert_eq!(
        RecordOp::get(KEY).only(["a", "a"]),
        RecordOp::get(KEY).only(["a"])
    );
}
