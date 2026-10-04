use std::error::Error;

use golemdb_branch::BranchError;
use golemdb_cells::{CellNameError, CellParseError};
use golemdb_record::RecordError;
use golemdb_storage::StorageError;

use crate::CommitId;

/// Public failures, independent of how the layers below nest their errors.
/// Sources retain detailed diagnostics without requiring consumers to match them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    #[error("operation {operation} is not implemented")]
    NotImplemented { operation: &'static str },
    #[error("record not found")]
    NotFound,
    #[error("record key already exists")]
    AlreadyExists,
    /// A `create` that does not match the database's key mode: a missing key
    /// where keys are caller-assigned, or a key where the database generates
    /// them (`Genesis::record_keys`).
    #[error("the create's key does not match the database's key mode")]
    KeyModeMismatch,
    /// The call's cost would exceed its budget; nothing was applied, and the
    /// receipt's cost is what was spent. Not returned until metering is
    /// implemented: every call costs 0 until then.
    #[error("the call's budget is exhausted")]
    OutOfBudget,
    #[error("reserved records cannot be modified through CRUD")]
    Reserved,
    #[error("invalid argument: {message}")]
    InvalidArgument {
        message: String,
        #[source]
        source: Option<Box<dyn Error + Send + Sync>>,
    },
    #[error("branch handle is unknown, consumed, or invalidated")]
    HandleInvalid,
    #[error("commit lost the publication race")]
    Conflict,
    #[error("branch is sealed")]
    Sealed,
    #[error("no frame remains to roll back")]
    NoFrameToRollback,
    #[error("commit {requested} unavailable: only current head {head} is supported")]
    CommitUnavailable { requested: CommitId, head: CommitId },
    /// The store reached its size cap; the operation (in practice `commit`)
    /// wrote nothing and the head is unchanged.
    ///
    /// This error is environmental, not deterministic: it depends on one node's
    /// store configuration, not on the operation or the state. It must never
    /// become part of a result other nodes see, such as a failed transaction.
    /// The node should stop committing; once the store is reopened with a
    /// larger cap, it continues from its last commit.
    #[error("store is full: its size cap is reached")]
    StoreFull,
    #[error("internal database error: {source}")]
    Internal {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl ApiError {
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
            source: None,
        }
    }

    pub fn internal(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Internal {
            source: Box::new(source),
        }
    }

    /// A typed cause: the message names the kind of input, the source says
    /// what is wrong with it, so error reports print each text once.
    fn invalid_input(what: &str, source: impl Error + Send + Sync + 'static) -> Self {
        Self::InvalidArgument {
            message: what.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl From<CellNameError> for ApiError {
    fn from(error: CellNameError) -> Self {
        Self::invalid_input("invalid cell name", error)
    }
}

impl From<CellParseError> for ApiError {
    fn from(error: CellParseError) -> Self {
        Self::invalid_input("invalid cell value", error)
    }
}

impl From<RecordError> for ApiError {
    fn from(error: RecordError) -> Self {
        match error {
            RecordError::NotFound => Self::NotFound,
            RecordError::AlreadyExists => Self::AlreadyExists,
            RecordError::KeyModeMismatch => Self::KeyModeMismatch,
            RecordError::Reserved => Self::Reserved,
            // The message is the whole error; repeating it as a source would
            // print it twice in error reports.
            RecordError::InvalidArgument(message) => Self::invalid_argument(message),
            RecordError::CommitUnavailable { requested, head } => {
                Self::CommitUnavailable { requested, head }
            }
            RecordError::Branch(error) => error.into(),
            RecordError::Storage(StorageError::Full) => Self::StoreFull,
            error => Self::internal(error),
        }
    }
}

impl From<BranchError> for ApiError {
    fn from(error: BranchError) -> Self {
        match error {
            BranchError::HandleInvalid => Self::HandleInvalid,
            BranchError::Conflict => Self::Conflict,
            BranchError::Sealed => Self::Sealed,
            BranchError::NoFrameToRollback => Self::NoFrameToRollback,
            BranchError::Storage(StorageError::Full) => Self::StoreFull,
            error => Self::internal(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
