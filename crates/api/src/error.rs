use std::error::Error;

use golemdb_branch::BranchError;
use golemdb_cells::{CellNameError, CellParseError};
use golemdb_record::RecordError;

use crate::CommitId;

/// Public failures, independent of how the engine nests its internal errors.
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

    fn invalid_input(source: impl Error + Send + Sync + 'static) -> Self {
        Self::InvalidArgument {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }
}

impl From<CellNameError> for ApiError {
    fn from(error: CellNameError) -> Self {
        Self::invalid_input(error)
    }
}

impl From<CellParseError> for ApiError {
    fn from(error: CellParseError) -> Self {
        Self::invalid_input(error)
    }
}

impl From<RecordError> for ApiError {
    fn from(error: RecordError) -> Self {
        match error {
            RecordError::NotFound => Self::NotFound,
            RecordError::AlreadyExists => Self::AlreadyExists,
            RecordError::Reserved => Self::Reserved,
            RecordError::InvalidArgument(ref message) => Self::InvalidArgument {
                message: message.clone(),
                source: Some(Box::new(error)),
            },
            RecordError::CommitUnavailable { requested, head } => {
                Self::CommitUnavailable { requested, head }
            }
            RecordError::Branch(error) => error.into(),
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
            error => Self::internal(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
