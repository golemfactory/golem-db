//! Public Rust contracts for GolemDB record CRUD and branch lifecycle.
//!
//! [`Api`] is the synchronous facade contract for CRUD and branch operations.
//! Consumers can use `Arc<dyn Api + Send + Sync>`; builders accept explicit cell values.
//! All results own their data and expose no storage transaction or cell writer.
//!
//! [`Database`] implements Api on a cloneable handle with a shared branch registry.
//! Its constructors initialize or validate genesis and select the configured hash.
//! These contracts are unmetered: no budget, receipt, debug flag, or record version.
//! Current record storage supports branch reads and committed-head reads only.
//! Immutable-data types and methods are exposed, but every such Database call
//! returns [`ApiError::NotImplemented`] immediately without I/O or state changes.
//!
//! ```
//! use golemdb_api::{Api, CellLimits, CellValue, Database, GenesisConfig,
//!     HashAlgorithm, OpenConfig, ReadTarget, RecordInput, RecordKey, Projection};
//!
//! let config = OpenConfig::new(GenesisConfig {
//!     hash_function: HashAlgorithm::Keccak256,
//!     cell_limits: CellLimits {
//!         max_cell_name_len: 32, max_str_len: 64, max_bytes_len: 128,
//!     },
//! });
//! let db = Database::open_memory(&config)?;
//! let branch = db.begin()?;
//! let key = RecordKey([0x42; 32]);
//! db.create(branch, key, RecordInput::new()
//!     .attribute("price", CellValue::from_i32(50))?
//!     .field("description", CellValue::from_str("A product"))?)?;
//! let pending = db.clone().get(ReadTarget::Branch(branch), key, Projection::All)?;
//! assert_eq!(pending.cells[b"price".as_slice()].as_i32(), Some(50));
//! let commit = db.commit(branch)?;
//! assert_eq!(commit, 1);
//! assert_eq!(db.get(ReadTarget::Head, key, Projection::All)?, pending);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod config;
mod database;
mod error;
mod genesis;
mod immutable_data;
mod input;
mod open;
mod open_error;
mod types;

pub use config::{GenesisConfig, OpenConfig, OpenMode};
pub use database::Database;
pub use error::{ApiError, Result};
pub use golemdb_cells::CellLimits;
pub use golemdb_merkle::HashAlgorithm;
#[cfg(feature = "mdbx")]
pub use golemdb_storage::MdbxOptions;
pub use immutable_data::{
    ImmutableDataAddress, ImmutableDataKey, ImmutableDataOrdinal, ImmutableDataRow,
};
pub use input::{PatchInput, RecordInput};
pub use open::{OpenInfo, OpenedStore, open_memory, open_store};
#[cfg(feature = "mdbx")]
pub use open::{open, open_database, open_with_options};
pub use open_error::{OpenError, OpenResult};
pub use types::{Projection, SealInfo};

pub use golemdb_branch::{BranchId, BranchInfo, CommitId};
pub use golemdb_cells::{CellKind, CellName, CellNameRef, CellType, CellValue, FloatWidth, Width};
pub use golemdb_record::{CellPatch, ReadTarget, Record, RecordCells, RecordKey, RecordPatch};

/// Synchronous record and branch facade. Builders assist callers; implementations
/// enforce deployment limits, reserved-record rules, identity, and atomic mutations.
/// Adding a required method requires updating concrete implementations and mocks.
pub trait Api {
    /// Stage a caller-keyed record with at least one user cell and return its key.
    fn create(&self, branch: BranchId, key: RecordKey, cells: RecordInput) -> Result<RecordKey>;

    /// Read pending or committed state. Head selection and materialization share
    /// one snapshot. Missing projected cells are omitted; an empty projection
    /// still verifies existence. Explicit non-head commits are not yet supported.
    fn get(&self, target: ReadTarget, key: RecordKey, projection: Projection) -> Result<Record>;

    /// Atomically edit one record, preserving at least one user cell. Empty
    /// patches and removals of missing cells are valid on an existing record.
    fn patch(&self, branch: BranchId, key: RecordKey, patch: PatchInput) -> Result<()>;

    /// Delete the record and its binding without reclaiming the internal ID.
    fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()>;

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
