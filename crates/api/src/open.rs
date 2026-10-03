use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use golemdb_branch::Head;
use golemdb_merkle::{Blake3Hasher, HashAlgorithm, Keccak256Hasher};
use golemdb_storage::{MemoryStore, Store, WriteTransaction};

use crate::{CommitId, GenesisConfig, OpenConfig, OpenResult};

/// Snapshot of opening, not a live head monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenInfo {
    pub created: bool,
    pub commit_id: CommitId,
    pub state_root: [u8; 32],
    pub index_root: [u8; 32],
    pub genesis_id: [u8; 32],
}

impl OpenInfo {
    pub(crate) fn new(created: bool, head: Head, genesis_id: [u8; 32]) -> Self {
        Self {
            created,
            commit_id: head.commit_id,
            state_root: head.state_root,
            index_root: head.index_root,
            genesis_id,
        }
    }
}

/// Validated storage ready for engine construction. This setup boundary is for
/// trusted library code; ordinary data consumers use GolemDb's constructors.
/// Consume this handle with into_golem_db to connect it to the public facade.
pub struct OpenedStore<S> {
    store: S,
    genesis: GenesisConfig,
    info: OpenInfo,
}

impl<S> OpenedStore<S> {
    pub fn info(&self) -> &OpenInfo {
        &self.info
    }
    pub fn genesis(&self) -> &GenesisConfig {
        &self.genesis
    }
    /// Hand initialized storage to the engine; low-level writes remain trusted.
    pub fn into_store(self) -> S {
        self.store
    }
}

impl<S: Store + Send + Sync + 'static> OpenedStore<S> {
    /// Construct one engine using the validated hash selection. The returned
    /// facade's clones share its branch registry and storage lifetime.
    pub fn into_golem_db(self) -> OpenResult<crate::GolemDb> {
        crate::GolemDb::from_opened(self)
    }
}

/// Open a caller-supplied store. Clone an existing store to share its
/// environment; never open the same MDBX directory twice within one process.
/// Inspect, validate, and initialize under one writer to serialize with other
/// initializers and committers. Reopening does not commit or rewrite any rows.
pub fn open_store<S: Store>(store: S, config: &OpenConfig) -> OpenResult<OpenedStore<S>> {
    config
        .genesis
        .validate(store.max_key_size(), store.max_value_size())?;
    let mut tx = store.begin_write()?;
    let prepared = catch_unwind(AssertUnwindSafe(|| match config.genesis.hash_function {
        HashAlgorithm::Keccak256 => crate::genesis::prepare(&mut tx, config, &Keccak256Hasher),
        HashAlgorithm::Blake3 => crate::genesis::prepare(&mut tx, config, &Blake3Hasher),
    }));
    let info = match prepared {
        Ok(result) => result?,
        Err(panic) => {
            drop(tx);
            resume_unwind(panic)
        }
    };
    if info.created {
        tx.commit()?;
    } else {
        tx.abort();
    }
    Ok(OpenedStore {
        store,
        genesis: config.genesis,
        info,
    })
}

/// Open a fresh in-memory database using the same genesis transaction as MDBX.
/// To reopen shared memory state, pass a store clone to open_store instead.
///
/// ```
/// use golemdb_api::{GenesisConfig, OpenConfig, open_memory};
/// let genesis = GenesisConfig::from_yaml("\
/// hash_function: keccak-256
/// cell_limits:
///   max_cell_name_len: 32
///   max_str_len: 64
///   max_bytes_len: 128
/// ")?;
/// let opened = open_memory(&OpenConfig::new(genesis))?;
/// assert!(opened.info().created);
/// assert_eq!(opened.info().commit_id, 0);
/// # Ok::<(), golemdb_api::OpenError>(())
/// ```
pub fn open_memory(config: &OpenConfig) -> OpenResult<OpenedStore<MemoryStore>> {
    open_store(MemoryStore::new(), config)
}

#[cfg(feature = "mdbx")]
/// Open persistent storage with default local MDBX capacity settings.
/// See open_with_options for lifecycle and shared-environment constraints.
pub fn open_database(
    path: impl AsRef<std::path::Path>,
    config: &OpenConfig,
) -> OpenResult<OpenedStore<golemdb_storage::MdbxStore>> {
    open_with_options(path, config, golemdb_storage::MdbxOptions::default())
}

/// Alias for open_database.
#[cfg(feature = "mdbx")]
pub fn open(
    path: impl AsRef<std::path::Path>,
    config: &OpenConfig,
) -> OpenResult<OpenedStore<golemdb_storage::MdbxStore>> {
    open_database(path, config)
}

/// Capacity settings are local store options and do not affect genesis identity.
/// ExistingOnly may open an empty environment but will never initialize it.
#[cfg(feature = "mdbx")]
pub fn open_with_options(
    path: impl AsRef<std::path::Path>,
    config: &OpenConfig,
    options: golemdb_storage::MdbxOptions,
) -> OpenResult<OpenedStore<golemdb_storage::MdbxStore>> {
    // Do not create a missing directory for ExistingOnly. Initialization itself
    // is always determined from the contents under the storage writer lock.
    if config.mode == crate::OpenMode::ExistingOnly
        && !path.as_ref().try_exists().map_err(crate::OpenError::Io)?
    {
        return Err(crate::OpenError::NotInitialized);
    }
    let store = golemdb_storage::MdbxStore::open_with_options(path, options)?;
    open_store(store, config)
}
