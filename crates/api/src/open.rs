use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use golemdb_branch::Head;
use golemdb_merkle::{Blake3Hasher, HashAlgorithm, Keccak256Hasher};
use golemdb_storage::{Store, WriteTransaction};

use crate::{CommitId, Config, Genesis, OpenResult};

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

/// A validated store, ready to open as a database. This setup boundary is for
/// trusted library code and is public only with the `internals` feature;
/// ordinary consumers use `Database`'s constructors. Consume this handle with
/// into_database to connect it to the public facade.
pub struct OpenedStore<S> {
    store: S,
    genesis: Genesis,
    info: OpenInfo,
}

impl<S> OpenedStore<S> {
    pub fn info(&self) -> &OpenInfo {
        &self.info
    }
    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }
    /// Return the validated store for trusted library code. Writes through it
    /// bypass all record and reserved-record checks.
    pub fn into_store(self) -> S {
        self.store
    }
}

impl<S: Store + Send + Sync + 'static> OpenedStore<S> {
    /// Open one database using the validated hash selection. The returned
    /// facade's clones share its branch registry and storage lifetime.
    pub fn into_database(self) -> OpenResult<crate::Database> {
        crate::Database::from_opened(self)
    }
}

/// Validate or initialize a caller-supplied store (public only with the
/// `internals` feature). Clone an existing store to share its
/// environment; never open the same MDBX directory twice within one process.
/// Inspect, validate, and initialize under one writer to serialize with other
/// initializers and committers. Reopening does not commit or rewrite any rows.
pub fn open_store<S: Store>(store: S, config: &Config) -> OpenResult<OpenedStore<S>> {
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

/// MDBX at `path`, validated or initialized. Behind `Database::open`.
#[cfg(feature = "mdbx")]
pub(crate) fn open_mdbx(
    path: &std::path::Path,
    config: &Config,
    options: golemdb_storage::MdbxOptions,
) -> OpenResult<OpenedStore<golemdb_storage::MdbxStore>> {
    // Do not create a missing directory for ExistingOnly. Initialization itself
    // is always determined from the contents under the store's writer lock.
    if config.mode == crate::OpenMode::ExistingOnly
        && !path.try_exists().map_err(crate::OpenError::Io)?
    {
        return Err(crate::OpenError::NotInitialized);
    }
    let store = golemdb_storage::MdbxStore::open_with_options(path, options)?;
    open_store(store, config)
}
