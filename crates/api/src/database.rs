use std::sync::Arc;

use golemdb_branch::Branches;
use golemdb_merkle::{Blake3Hasher, HashProvider, Keccak256Hasher};
use golemdb_record::Records;
use golemdb_storage::Store;

use crate::{
    Api, ApiError, BranchId, BranchInfo, CommitId, GenesisConfig, HashAlgorithm,
    ImmutableDataAddress, ImmutableDataKey, ImmutableDataOrdinal, ImmutableDataRow, OpenConfig,
    OpenInfo, OpenResult, OpenedDatabase, PatchInput, Projection, ReadTarget, Record, RecordInput,
    RecordKey, Result, SealInfo,
};

/// Cloneable public database handle. Clones share one engine and branch registry;
/// a branch begun through one clone can be used through any other clone.
/// Dropping the last clone discards pending work; only commit publishes it.
///
/// Backend and hash-provider types stay inside the engine. Dispatch occurs once
/// per API operation; hashing within an operation uses a concrete provider.
/// No storage writer or unrestricted cell access is exposed by this handle.
#[derive(Clone)]
pub struct GolemDb {
    engine: Arc<dyn Api + Send + Sync>,
    genesis: GenesisConfig,
    info: OpenInfo,
}

impl GolemDb {
    /// Initialize a fresh memory backend. For a shared existing backend use
    /// from_backend; for another handle to the same engine use clone.
    pub fn open_memory(config: &OpenConfig) -> OpenResult<Self> {
        crate::open_memory(config)?.into_golem_db()
    }

    /// Initialize or validate a caller-supplied backend and construct one engine.
    /// Separate calls create separate branch registries, even over shared storage.
    pub fn from_backend<S: Store + Send + Sync + 'static>(
        store: S,
        config: &OpenConfig,
    ) -> OpenResult<Self> {
        crate::open_backend(store, config)?.into_golem_db()
    }

    /// Open durable storage. Close all handles before independently reopening the
    /// same MDBX directory within one process; use clone to share a live engine.
    #[cfg(feature = "mdbx")]
    pub fn open_database(
        path: impl AsRef<std::path::Path>,
        config: &OpenConfig,
    ) -> OpenResult<Self> {
        crate::open_database(path, config)?.into_golem_db()
    }

    /// Alias for open_database.
    #[cfg(feature = "mdbx")]
    pub fn open(path: impl AsRef<std::path::Path>, config: &OpenConfig) -> OpenResult<Self> {
        Self::open_database(path, config)
    }

    #[cfg(feature = "mdbx")]
    pub fn open_with_options(
        path: impl AsRef<std::path::Path>,
        config: &OpenConfig,
        options: crate::MdbxOptions,
    ) -> OpenResult<Self> {
        crate::open_with_options(path, config, options)?.into_golem_db()
    }

    /// Immutable deployment configuration validated at opening.
    pub fn genesis(&self) -> &GenesisConfig {
        &self.genesis
    }

    /// Opening snapshot, shared in meaning by all clones. Use Api::head for the
    /// current head, or ReadTarget::Head for a consistent current record read.
    pub fn info(&self) -> &OpenInfo {
        &self.info
    }

    pub(crate) fn from_opened<S: Store + Send + Sync + 'static>(
        opened: OpenedDatabase<S>,
    ) -> OpenResult<Self> {
        let genesis = *opened.genesis();
        let info = *opened.info();
        let store = opened.into_database();
        let engine: Arc<dyn Api + Send + Sync> = match genesis.hash_function {
            HashAlgorithm::Keccak256 => Arc::new(Engine::new(store, Keccak256Hasher)?),
            HashAlgorithm::Blake3 => Arc::new(Engine::new(store, Blake3Hasher)?),
        };
        Ok(Self {
            engine,
            genesis,
            info,
        })
    }
}

impl Api for GolemDb {
    fn create(&self, branch: BranchId, key: RecordKey, cells: RecordInput) -> Result<RecordKey> {
        self.engine.create(branch, key, cells)
    }
    fn get(&self, target: ReadTarget, key: RecordKey, projection: Projection) -> Result<Record> {
        self.engine.get(target, key, projection)
    }
    fn patch(&self, branch: BranchId, key: RecordKey, patch: PatchInput) -> Result<()> {
        self.engine.patch(branch, key, patch)
    }
    fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()> {
        self.engine.delete(branch, key)
    }
    fn head(&self) -> Result<CommitId> {
        self.engine.head()
    }
    fn begin(&self) -> Result<BranchId> {
        self.engine.begin()
    }
    fn branch_info(&self, branch: BranchId) -> Result<BranchInfo> {
        self.engine.branch_info(branch)
    }
    fn checkpoint(&self, branch: BranchId) -> Result<()> {
        self.engine.checkpoint(branch)
    }
    fn rollback(&self, branch: BranchId) -> Result<()> {
        self.engine.rollback(branch)
    }
    fn seal(&self, branch: BranchId) -> Result<SealInfo> {
        self.engine.seal(branch)
    }
    fn commit(&self, branch: BranchId) -> Result<CommitId> {
        self.engine.commit(branch)
    }
    fn discard(&self, branch: BranchId) -> Result<()> {
        self.engine.discard(branch)
    }

    fn immutable_data_append(
        &self,
        branch: BranchId,
        segment: &str,
        key: Option<ImmutableDataKey>,
        row: ImmutableDataRow,
    ) -> Result<ImmutableDataOrdinal> {
        self.engine.immutable_data_append(branch, segment, key, row)
    }
    fn immutable_data_get(
        &self,
        segment: &str,
        address: ImmutableDataAddress,
    ) -> Result<ImmutableDataRow> {
        self.engine.immutable_data_get(segment, address)
    }
    fn immutable_data_range_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<std::ops::Range<ImmutableDataOrdinal>> {
        self.engine.immutable_data_range_of(segment, commit)
    }
    fn immutable_data_rows_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<Vec<ImmutableDataRow>> {
        self.engine.immutable_data_rows_of(segment, commit)
    }
}

/// Both layers reference the same branch registry. Business rules, locking, and
/// transaction boundaries remain in record and branch, not in this adapter.
struct Engine<S, H> {
    branches: Branches<S, H>,
    records: Records<S, H>,
}

impl<S: Store, H: HashProvider> Engine<S, H> {
    fn new(store: S, hasher: H) -> OpenResult<Self> {
        let branches = Branches::new(store, hasher)?;
        let records = Records::new(branches.clone());
        Ok(Self { branches, records })
    }
}

impl<S: Store, H: HashProvider> Api for Engine<S, H> {
    fn create(&self, branch: BranchId, key: RecordKey, cells: RecordInput) -> Result<RecordKey> {
        self.records
            .create(branch, key, cells.into_cells())
            .map_err(Into::into)
    }
    fn get(&self, target: ReadTarget, key: RecordKey, projection: Projection) -> Result<Record> {
        self.records
            .get(target, key, projection.as_names())
            .map_err(Into::into)
    }
    fn patch(&self, branch: BranchId, key: RecordKey, patch: PatchInput) -> Result<()> {
        self.records
            .patch(branch, key, patch.into_patch())
            .map_err(Into::into)
    }
    fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()> {
        self.records.delete(branch, key).map_err(Into::into)
    }
    fn head(&self) -> Result<CommitId> {
        self.branches.head().map_err(Into::into)
    }
    fn begin(&self) -> Result<BranchId> {
        self.branches.begin().map_err(Into::into)
    }
    fn branch_info(&self, branch: BranchId) -> Result<BranchInfo> {
        self.branches.branch_info(branch).map_err(Into::into)
    }
    fn checkpoint(&self, branch: BranchId) -> Result<()> {
        self.branches.checkpoint(branch).map_err(Into::into)
    }
    fn rollback(&self, branch: BranchId) -> Result<()> {
        self.branches.rollback(branch).map_err(Into::into)
    }
    fn seal(&self, branch: BranchId) -> Result<SealInfo> {
        self.branches
            .seal(branch)
            .map(|sealed| SealInfo::from(sealed.as_ref()))
            .map_err(Into::into)
    }
    fn commit(&self, branch: BranchId) -> Result<CommitId> {
        self.branches.commit(branch).map_err(Into::into)
    }
    fn discard(&self, branch: BranchId) -> Result<()> {
        self.branches.discard(branch).map_err(Into::into)
    }

    fn immutable_data_append(
        &self,
        _: BranchId,
        _: &str,
        _: Option<ImmutableDataKey>,
        _: ImmutableDataRow,
    ) -> Result<ImmutableDataOrdinal> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_append",
        })
    }
    fn immutable_data_get(&self, _: &str, _: ImmutableDataAddress) -> Result<ImmutableDataRow> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_get",
        })
    }
    fn immutable_data_range_of(
        &self,
        _: &str,
        _: CommitId,
    ) -> Result<std::ops::Range<ImmutableDataOrdinal>> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_range_of",
        })
    }
    fn immutable_data_rows_of(&self, _: &str, _: CommitId) -> Result<Vec<ImmutableDataRow>> {
        Err(ApiError::NotImplemented {
            operation: "immutable_data_rows_of",
        })
    }
}
