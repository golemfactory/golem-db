use std::error::Error;

use crate::ApiError;
use golemdb_branch::BranchError;
use golemdb_cells::{CellNameError, CellValueParseError};
use golemdb_record::RecordError;

#[test]
fn public_errors_preserve_meaning_without_nested_matching() {
    assert!(matches!(
        ApiError::from(RecordError::NotFound),
        ApiError::NotFound
    ));
    assert!(matches!(
        ApiError::from(RecordError::AlreadyExists),
        ApiError::AlreadyExists
    ));
    assert!(matches!(
        ApiError::from(RecordError::Reserved),
        ApiError::Reserved
    ));
    assert!(matches!(
        ApiError::from(RecordError::Branch(BranchError::Conflict)),
        ApiError::Conflict
    ));
    assert!(matches!(
        ApiError::from(BranchError::HandleInvalid),
        ApiError::HandleInvalid
    ));
    assert!(matches!(
        ApiError::from(BranchError::Sealed),
        ApiError::Sealed
    ));
    assert!(matches!(
        ApiError::from(BranchError::NoFrameToRollback),
        ApiError::NoFrameToRollback
    ));
    assert!(matches!(
        ApiError::from(RecordError::CommitUnavailable {
            requested: 0,
            head: 2
        }),
        ApiError::CommitUnavailable {
            requested: 0,
            head: 2
        }
    ));
}

#[test]
fn invalid_input_and_internal_failures_keep_diagnostic_sources() {
    let invalid = ApiError::from(CellValueParseError::NotIndexable);
    assert!(matches!(&invalid, ApiError::InvalidArgument { .. }));
    assert_eq!(
        invalid
            .source()
            .unwrap()
            .downcast_ref::<CellValueParseError>(),
        Some(&CellValueParseError::NotIndexable)
    );
    let invalid = ApiError::from(CellNameError::Empty);
    assert_eq!(
        invalid.source().unwrap().downcast_ref::<CellNameError>(),
        Some(&CellNameError::Empty)
    );
    let invalid = ApiError::from(RecordError::InvalidArgument("deployment limit".into()));
    assert!(
        invalid
            .source()
            .unwrap()
            .downcast_ref::<RecordError>()
            .is_some()
    );
    let error = ApiError::from(BranchError::Storage(
        golemdb_storage::StorageError::Backend("disk error".into()),
    ));
    assert!(matches!(&error, ApiError::Internal { .. }));
    assert!(error.to_string().contains("disk error"));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<BranchError>()
            .is_some()
    );
    let error = ApiError::from(RecordError::CorruptState("missing identity"));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<RecordError>()
            .is_some()
    );
    // Public errors can be shared by the same thread-safe consumers as dyn Api.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ApiError>();
}
