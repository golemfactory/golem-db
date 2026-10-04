use std::sync::Arc;

use golemdb_branch::Branches;
use golemdb_merkle::{Blake3Hasher, HashProvider, Keccak256Hasher};
use golemdb_record::Records;
use golemdb_storage::Store;

use crate::{
    Api, ApiError, BranchId, BranchInfo, CommitId, Config, Genesis, HashAlgorithm,
    ImmutableDataAddress, ImmutableDataKey, ImmutableDataOrdinal, ImmutableDataRow, OpenInfo,
    OpenResult, OpenedStore, PatchInput, Projection, ReadTarget, Record, RecordInput, RecordKey,
    Result, SealInfo,
};

/// A handle to an open database. Cheap to clone; all clones share the same open
/// branches, so a branch begun through one clone works through any other.
/// Open branches live in memory: dropping the last clone discards them, and
/// only `commit` writes to the store.
///
/// The store and hash function are chosen at opening and hidden here, so this
/// type has no generic parameters. That costs one dynamic call per operation;
/// hashing inside an operation is statically dispatched.
///
/// The handle only offers the validated `Api` operations, never raw store
/// writes or direct access to reserved cells.
#[derive(Clone)]
pub struct Database {
    inner: Arc<dyn Api + Send + Sync>,
    genesis: Genesis,
    info: OpenInfo,
}

impl Database {
    /// Initialize a fresh memory store. For a shared existing store use
    /// from_store; for another handle to the same database use clone.
    pub fn open_memory(config: &Config) -> OpenResult<Self> {
        crate::open_memory(config)?.into_database()
    }

    /// Initialize or validate a caller-supplied store and open a database on it.
    /// Separate calls create separate branch registries, even over a shared store.
    pub fn from_store<S: Store + Send + Sync + 'static>(
        store: S,
        config: &Config,
    ) -> OpenResult<Self> {
        crate::open_store(store, config)?.into_database()
    }

    /// Open durable storage. Close all handles before independently reopening the
    /// same MDBX directory within one process; use clone to share an open database.
    #[cfg(feature = "mdbx")]
    pub fn open_database(path: impl AsRef<std::path::Path>, config: &Config) -> OpenResult<Self> {
        crate::open_database(path, config)?.into_database()
    }

    /// Alias for open_database.
    #[cfg(feature = "mdbx")]
    pub fn open(path: impl AsRef<std::path::Path>, config: &Config) -> OpenResult<Self> {
        Self::open_database(path, config)
    }

    #[cfg(feature = "mdbx")]
    pub fn open_with_options(
        path: impl AsRef<std::path::Path>,
        config: &Config,
        options: crate::MdbxOptions,
    ) -> OpenResult<Self> {
        crate::open_with_options(path, config, options)?.into_database()
    }

    /// Immutable deployment configuration validated at opening.
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    /// Opening snapshot, shared in meaning by all clones. Use Api::head for the
    /// current head, or ReadTarget::Head for a consistent current record read.
    pub fn info(&self) -> &OpenInfo {
        &self.info
    }

    pub(crate) fn from_opened<S: Store + Send + Sync + 'static>(
        opened: OpenedStore<S>,
    ) -> OpenResult<Self> {
        let genesis = *opened.genesis();
        let info = *opened.info();
        let store = opened.into_store();
        let inner: Arc<dyn Api + Send + Sync> = match genesis.hash_function {
            HashAlgorithm::Keccak256 => Arc::new(Inner::new(store, Keccak256Hasher)?),
            HashAlgorithm::Blake3 => Arc::new(Inner::new(store, Blake3Hasher)?),
        };
        Ok(Self {
            inner,
            genesis,
            info,
        })
    }
}

impl Api for Database {
    fn create(&self, branch: BranchId, key: RecordKey, cells: RecordInput) -> Result<RecordKey> {
        self.inner.create(branch, key, cells)
    }
    fn get(&self, target: ReadTarget, key: RecordKey, projection: Projection) -> Result<Record> {
        self.inner.get(target, key, projection)
    }
    fn patch(&self, branch: BranchId, key: RecordKey, patch: PatchInput) -> Result<()> {
        self.inner.patch(branch, key, patch)
    }
    fn delete(&self, branch: BranchId, key: RecordKey) -> Result<()> {
        self.inner.delete(branch, key)
    }
    fn head(&self) -> Result<CommitId> {
        self.inner.head()
    }
    fn begin(&self) -> Result<BranchId> {
        self.inner.begin()
    }
    fn branch_info(&self, branch: BranchId) -> Result<BranchInfo> {
        self.inner.branch_info(branch)
    }
    fn checkpoint(&self, branch: BranchId) -> Result<()> {
        self.inner.checkpoint(branch)
    }
    fn rollback(&self, branch: BranchId) -> Result<()> {
        self.inner.rollback(branch)
    }
    fn seal(&self, branch: BranchId) -> Result<SealInfo> {
        self.inner.seal(branch)
    }
    fn commit(&self, branch: BranchId) -> Result<CommitId> {
        self.inner.commit(branch)
    }
    fn discard(&self, branch: BranchId) -> Result<()> {
        self.inner.discard(branch)
    }

    fn immutable_data_append(
        &self,
        branch: BranchId,
        segment: &str,
        key: Option<ImmutableDataKey>,
        row: ImmutableDataRow,
    ) -> Result<ImmutableDataOrdinal> {
        self.inner.immutable_data_append(branch, segment, key, row)
    }
    fn immutable_data_get(
        &self,
        segment: &str,
        address: ImmutableDataAddress,
    ) -> Result<ImmutableDataRow> {
        self.inner.immutable_data_get(segment, address)
    }
    fn immutable_data_range_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<std::ops::Range<ImmutableDataOrdinal>> {
        self.inner.immutable_data_range_of(segment, commit)
    }
    fn immutable_data_rows_of(
        &self,
        segment: &str,
        commit: CommitId,
    ) -> Result<Vec<ImmutableDataRow>> {
        self.inner.immutable_data_rows_of(segment, commit)
    }
}

/// State shared by all clones of a `Database`: the branch registry and the
/// record layer over it, both referencing the same registry. Implements `Api`
/// by forwarding; business rules, locking, and transaction boundaries remain in
/// record and branch, not in this adapter.
struct Inner<S, H> {
    branches: Branches<S, H>,
    records: Records<S, H>,
}

impl<S: Store, H: HashProvider> Inner<S, H> {
    fn new(store: S, hasher: H) -> OpenResult<Self> {
        let branches = Branches::new(store, hasher)?;
        let records = Records::new(branches.clone());
        Ok(Self { branches, records })
    }
}

impl<S: Store, H: HashProvider> Api for Inner<S, H> {
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
