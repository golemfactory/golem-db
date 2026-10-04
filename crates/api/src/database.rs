use std::sync::Arc;

use golemdb_branch::Branches;
use golemdb_merkle::{Blake3Hasher, HashProvider, Keccak256Hasher};
use golemdb_record::{Details, Records};
use golemdb_storage::{MemoryStore, Store};

use crate::open::OpenedStore;

use crate::{
    Api, ApiError, BranchId, BranchInfo, CommitId, Config, Genesis, HashAlgorithm,
    ImmutableDataAddress, ImmutableDataKey, ImmutableDataOrdinal, ImmutableDataRow, Metered,
    OpenInfo, OpenResult, ReadTarget, Receipt, Record, RecordKey, RecordOp, Result, SealInfo,
    StoreConfig, op,
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
    /// Open a database on a built-in store, creating or validating genesis
    /// according to `config.mode`. This is the standard way to open GolemDB.
    ///
    /// `store` is anything that converts into a `StoreConfig`. A path means
    /// MDBX with default options: `Database::open("./data", &config)`. For
    /// tuned capacity pass `StoreConfig::Mdbx { path, options }`; for a
    /// throwaway database, `StoreConfig::Memory`.
    ///
    /// For MDBX:
    /// - With `OpenMode::ExistingOnly`, a missing directory fails with
    ///   `NotInitialized` and nothing is created. An existing but empty
    ///   directory also fails with `NotInitialized`, but MDBX has created its
    ///   environment files in it by then.
    /// - Open a directory at most once per process; share an open database
    ///   with `clone`. Another process may open the same directory: its
    ///   branches are separate, and of two competing commits one gets `Conflict`.
    /// - A larger `max_map_size` takes effect on reopening; a smaller one than
    ///   the store already has is ignored.
    pub fn open(store: impl Into<StoreConfig>, config: &Config) -> OpenResult<Self> {
        match store.into() {
            StoreConfig::Memory => {
                crate::open::open_store(MemoryStore::new(), config)?.into_database()
            }
            #[cfg(feature = "mdbx")]
            StoreConfig::Mdbx { path, options } => {
                crate::open::open_mdbx(&path, config, options)?.into_database()
            }
        }
    }

    /// Open a fresh in-memory database: the standard for tests. Each call
    /// creates a new, empty database, so there is no `OpenMode` to choose.
    /// To reopen in-memory state, keep a clone of a `MemoryStore` and pass it
    /// to `from_store`; to share an open database, use `clone`.
    pub fn open_memory(genesis: &Genesis) -> OpenResult<Self> {
        crate::open::open_store(MemoryStore::new(), &Config::new(*genesis))?.into_database()
    }

    /// Open a database on a caller-supplied store, for custom or wrapped
    /// stores and tests.
    ///
    /// The store is trusted: its limits and durability are taken as reported,
    /// and writes made through it outside a database are not re-validated
    /// beyond the startup checks. The path guarantees of `open` do not apply,
    /// since the store already exists when this is called.
    ///
    /// A store should back one open database at a time; share an open
    /// database with `clone`. Reopening a store after its database is dropped
    /// is fine. Two databases open on one store at once behave like two
    /// processes: separate branches, and `Conflict` for the losing commit.
    pub fn from_store<S: Store + Send + Sync + 'static>(
        store: S,
        config: &Config,
    ) -> OpenResult<Self> {
        crate::open::open_store(store, config)?.into_database()
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
    fn create(&self, branch: BranchId, op: RecordOp<op::Create>) -> Metered<RecordKey> {
        self.inner.create(branch, op)
    }
    fn get(&self, target: ReadTarget, op: RecordOp<op::Get>) -> Metered<Record> {
        self.inner.get(target, op)
    }
    fn patch(&self, branch: BranchId, op: RecordOp<op::Patch>) -> Metered<()> {
        self.inner.patch(branch, op)
    }
    fn delete(&self, branch: BranchId, op: RecordOp<op::Delete>) -> Metered<()> {
        self.inner.delete(branch, op)
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

impl<S: Store, H: HashProvider> Inner<S, H> {
    /// The commit pricing a call on `branch`: its base commit, or the head if
    /// the handle is unknown (the call fails then, with a zero-cost receipt).
    fn priced_at(&self, branch: BranchId) -> CommitId {
        self.branches
            .branch_info(branch)
            .map(|info| info.commit_id)
            .or_else(|_| self.branches.head())
            .unwrap_or_default()
    }

    /// A write's outcome with its receipt: the effects on success, none on failure.
    fn write_receipt<T>(&self, branch: BranchId, outcome: Result<(T, Details)>) -> Metered<T> {
        let priced_at = self.priced_at(branch);
        match outcome {
            Ok((value, details)) => Metered::new(Ok(value), Receipt::unmetered(priced_at, details)),
            Err(error) => Metered::new(
                Err(error),
                Receipt::unmetered(priced_at, Details::default()),
            ),
        }
    }
}

impl<S: Store, H: HashProvider> Api for Inner<S, H> {
    fn create(&self, branch: BranchId, op: RecordOp<op::Create>) -> Metered<RecordKey> {
        let outcome = op.into_create().and_then(|(key, cells)| {
            // Every database uses caller-assigned keys until key modes exist.
            let key = key.ok_or(ApiError::KeyModeMismatch)?;
            Ok(self.records.create(branch, key, cells)?)
        });
        self.write_receipt(branch, outcome)
    }
    fn get(&self, target: ReadTarget, op: RecordOp<op::Get>) -> Metered<Record> {
        let priced_at = match target {
            ReadTarget::Commit(commit) => commit,
            ReadTarget::Branch(branch) => self.priced_at(branch),
            ReadTarget::Head => self.branches.head().unwrap_or_default(),
        };
        let (key, projection) = op.into_get();
        let result = self
            .records
            .get(target, key, projection.as_deref())
            .map_err(Into::into);
        Metered::new(result, Receipt::unmetered(priced_at, Details::default()))
    }
    fn patch(&self, branch: BranchId, op: RecordOp<op::Patch>) -> Metered<()> {
        let outcome = op
            .into_patch()
            .and_then(|(key, changes)| Ok(((), self.records.patch(branch, key, changes)?)));
        self.write_receipt(branch, outcome)
    }
    fn delete(&self, branch: BranchId, op: RecordOp<op::Delete>) -> Metered<()> {
        let outcome = self
            .records
            .delete(branch, op.into_delete())
            .map(|details| ((), details))
            .map_err(Into::into);
        self.write_receipt(branch, outcome)
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
