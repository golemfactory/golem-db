//! Public Rust contracts for GolemDB record CRUD and branch lifecycle.
//!
//! [`Api`] is the synchronous facade contract for record and branch operations.
//! Consumers can use `Arc<dyn Api + Send + Sync>`. Each record call takes a
//! [`RecordOp`] and returns a [`Metered`] outcome: the result together with a
//! [`Receipt`] of its cost and its effects on the record's cells. All results own
//! their data and expose no storage transaction or cell writer.
//!
//! [`Database`] implements Api on a cloneable handle with a shared branch registry.
//! Its constructors initialize or validate genesis and select the configured hash.
//! Metering is not implemented yet: every receipt's cost is 0 and budgets are not
//! enforced. Current record storage supports branch reads and committed-head reads
//! only. Immutable-data types and methods are exposed, but every such Database call
//! returns [`ApiError::NotImplemented`] immediately without I/O or state changes.
//!
//! ```
//! use golemdb_api::{Api, Database, Genesis, ReadTarget, RecordKey, RecordOp};
//!
//! let db = Database::open_memory(&Genesis::DEV)?;
//! let branch = db.begin()?;
//! let key = RecordKey([0x42; 32]);
//! db.create(branch, RecordOp::create().key(key)
//!     .attribute("price", 50i32)
//!     .field("description", "A product")).into_result()?;
//! let pending = db.clone().get(ReadTarget::Branch(branch), RecordOp::get(key)).into_result()?;
//! assert_eq!(pending.cells[b"price".as_slice()].as_i32(), Some(50));
//! let commit = db.commit(branch)?;
//! assert_eq!(commit, 1);
//! assert_eq!(db.get(ReadTarget::Head, RecordOp::get(key)).into_result()?, pending);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod config;
mod database;
mod error;
mod genesis;
mod immutable_data;
mod metered;
mod open;
mod open_error;
mod record_op;
mod types;

pub use config::{Config, Genesis, OpenMode, StoreConfig};
pub use database::Database;
pub use error::{ApiError, Result};
pub use golemdb_cells::CellLimits;
pub use golemdb_merkle::HashAlgorithm;
#[cfg(feature = "mdbx")]
pub use golemdb_storage::MdbxOptions;
pub use immutable_data::{
    ImmutableDataAddress, ImmutableDataKey, ImmutableDataOrdinal, ImmutableDataRow,
};
pub use metered::{Metered, Receipt};
pub use open::OpenInfo;
/// The lower-level opening layer, for trusted tooling and tests.
#[cfg(feature = "internals")]
pub use open::{OpenedStore, open_store};
pub use open_error::{OpenError, OpenResult};
pub use record_op::RecordOp;
pub use types::SealInfo;

/// The operations a [`RecordOp`] can be built for. Usually inferred from the
/// constructor; named in signatures such as `RecordOp<op::Create>`.
pub mod op {
    pub use crate::record_op::{Create, Delete, Get, Patch};
}

pub use golemdb_branch::{BranchId, BranchInfo, CommitId};
pub use golemdb_cells::{
    CellKind, CellName, CellNameRef, CellType, CellValue, FloatWidth, IntoCellValue, Width,
};
pub use golemdb_record::{Details, ReadTarget, Record, RecordKey};

/// Synchronous record and branch facade. Record calls take a [`RecordOp`] and
/// return [`Metered`] outcomes; implementations enforce deployment limits,
/// reserved-record rules, identity, and atomic mutations.
/// Adding a required method requires updating concrete implementations and mocks.
pub trait Api {
    /// Stage a new record with at least one user cell and return its key. The
    /// key must be named with `RecordOp::key` (`KeyModeMismatch` otherwise):
    /// every database uses caller-assigned keys today.
    fn create(&self, branch: BranchId, op: RecordOp<op::Create>) -> Metered<RecordKey>;

    /// Read pending or committed state. Head selection and materialization share
    /// one snapshot. Missing projected cells are omitted; an empty projection
    /// still verifies existence. Explicit non-head commits are not yet supported.
    fn get(&self, target: ReadTarget, op: RecordOp<op::Get>) -> Metered<Record>;

    /// Atomically edit one record, preserving at least one user cell. Empty
    /// patches and removals of missing cells are valid on an existing record.
    fn patch(&self, branch: BranchId, op: RecordOp<op::Patch>) -> Metered<()>;

    /// Delete the record and its binding without reclaiming the internal ID.
    fn delete(&self, branch: BranchId, op: RecordOp<op::Delete>) -> Metered<()>;

    // Branch lifecycle. IDs are valid only for the database that issued them.
    fn head(&self) -> Result<CommitId>;
    fn begin(&self) -> Result<BranchId>;
    fn branch_info(&self, branch: BranchId) -> Result<BranchInfo>;
    fn checkpoint(&self, branch: BranchId) -> Result<()>;

    /// Undo one frame. Repeated rollback moves to preceding frames.
    fn rollback(&self, branch: BranchId) -> Result<()>;

    /// Freeze without publishing. Sealed branches reject record access,
    /// checkpoints, and rollback; metadata, commit, and discard remain legal.
    fn seal(&self, branch: BranchId) -> Result<SealInfo>;

    /// Publish and consume the branch. A live losing branch returns Conflict;
    /// unknown, consumed, or previously invalidated IDs return HandleInvalid.
    fn commit(&self, branch: BranchId) -> Result<CommitId>;
    fn discard(&self, branch: BranchId) -> Result<()>;

    /// Future contract: stage a row on a sealed branch with an optional unique
    /// segment-local key; return a provisional ordinal. Duplicate keys never
    /// overwrite rows. Row and key binding become visible together at commit.
    /// Currently returns NotImplemented immediately, without validating arguments.
    fn immutable_data_append(
        &self,
        branch: BranchId,
        segment: &str,
        key: Option<ImmutableDataKey>,
        row: ImmutableDataRow,
    ) -> Result<ImmutableDataOrdinal>;

    /// Future contract: read a committed row by ordinal or caller-provided key.
    /// Currently returns NotImplemented without storage access.
    fn immutable_data_get(
        &self,
        segment: &str,
        address: ImmutableDataAddress,
    ) -> Result<ImmutableDataRow>;

    /// Future contract: the half-open ordinal range appended by this commit.
    /// Currently returns NotImplemented without storage access.
    fn immutable_data_range_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<std::ops::Range<ImmutableDataOrdinal>>;

    /// Future contract: this commit's rows in append order.
    /// Currently returns NotImplemented without storage access.
    fn immutable_data_rows_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<Vec<ImmutableDataRow>>;
}
