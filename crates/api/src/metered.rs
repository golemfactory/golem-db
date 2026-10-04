//! [`Metered`]: a record call's outcome together with its [`Receipt`].

use golemdb_record::Details;

use crate::{ApiError, CommitId};

/// The outcome of a record call and its receipt. The receipt is present
/// whether the call succeeded or failed, so a host can charge for failed
/// calls too.
///
/// Callers that do not charge drop the receipt with [`Metered::into_result`]:
/// `db.get(target, op).into_result()?`. Rust's `?` only works on `Result`, so
/// either `.into_result()?` or `.result?` is needed.
#[derive(Debug)]
#[must_use = "a record call's outcome is in `result`; it may be an error"]
pub struct Metered<T> {
    pub result: Result<T, ApiError>,
    pub receipt: Receipt,
}

impl<T> Metered<T> {
    pub fn new(result: Result<T, ApiError>, receipt: Receipt) -> Self {
        Self { result, receipt }
    }

    /// An outcome with a zero-cost receipt and no effects, for mocks and
    /// implementations without metering.
    pub fn unmetered(result: Result<T, ApiError>, priced_at: CommitId) -> Self {
        Self::new(result, Receipt::unmetered(priced_at, Details::default()))
    }

    /// The outcome without the receipt.
    pub fn into_result(self) -> Result<T, ApiError> {
        self.result
    }
}

/// What a record call cost and what it changed.
///
/// Non-exhaustive: further fields, such as the per-operation-class ledger that
/// comes with metering, can be added without breaking callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Receipt {
    /// The call's cost. **Always 0 until metering is implemented.**
    pub cost: u64,
    /// The commit whose cost schedule prices the call: for writes and branch
    /// reads the branch's base commit, for reads at a commit that commit.
    pub priced_at: CommitId,
    /// The call's effects on the record's user cells. Zero for reads and for
    /// failed calls, which apply nothing.
    ///
    /// Always present in this version; a later version may make them
    /// optional per call, as the metering spec allows.
    pub details: Details,
}

impl Receipt {
    /// A zero-cost receipt.
    pub fn unmetered(priced_at: CommitId, details: Details) -> Self {
        Self {
            cost: 0,
            priced_at,
            details,
        }
    }
}
