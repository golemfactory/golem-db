use std::path::Path;

use golemdb_cells::CellLimits;
use golemdb_merkle::HashAlgorithm;
use serde::{Deserialize, Serialize};

use crate::{OpenError, OpenResult};

/// Immutable deployment settings. Every YAML field is required; no admission
/// limit or hash algorithm is silently selected by the opener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Genesis {
    pub hash_function: HashAlgorithm,
    pub cell_limits: CellLimits,
}

impl Genesis {
    /// A genesis for development, tests and examples; never for a deployment.
    /// Its values may change between releases, so databases created with it
    /// are disposable. Deployments define their own genesis, usually as a
    /// YAML file loaded with [`Genesis::load`].
    pub const DEV: Genesis = Genesis {
        hash_function: HashAlgorithm::Keccak256,
        cell_limits: CellLimits {
            max_cell_name_len: 32,
            max_str_len: 64,
            max_bytes_len: 128,
        },
    };

    pub fn from_yaml(yaml: &str) -> OpenResult<Self> {
        serde_saphyr::from_str(yaml).map_err(OpenError::Yaml)
    }

    /// Read only this explicit path; no environment or implicit file search.
    pub fn load(path: impl AsRef<Path>) -> OpenResult<Self> {
        Self::from_yaml(&std::fs::read_to_string(path).map_err(OpenError::Io)?)
    }

    pub(crate) fn validate(&self, max_key: usize, max_value: usize) -> OpenResult<()> {
        let limits = self.cell_limits;
        if limits.max_cell_name_len == 0 {
            return Err(OpenError::InvalidConfig(
                "max_cell_name_len must be positive".into(),
            ));
        }
        // Complete cell keys include recordID. Index keys include a separator
        // and tag; fixed-width attributes may be wider than the string limit.
        let name = u64::from(limits.max_cell_name_len);
        let key = (name + 8)
            .max(name + 2 + u64::from(limits.max_str_len).max(32))
            .max(40);
        // Format 1 reserves 16 KiB for fixed internal values: trie nodes, metadata,
        // root pairs, and canonical single-u16-domain Roaring containers.
        let value = (1 + u64::from(limits.max_bytes_len.max(limits.max_str_len))).max(16 * 1024);
        if key > max_key as u64 {
            return Err(OpenError::InvalidConfig(format!(
                "configured cells/index terms need {key}-byte keys; store supports {max_key}"
            )));
        }
        if value > max_value as u64 {
            return Err(OpenError::InvalidConfig(format!(
                "configured cells and internal rows need {value}-byte values; store supports {max_value}"
            )));
        }
        Ok(())
    }
}

/// How opening treats existing storage. Non-exhaustive so that further modes,
/// such as a read-only open, can be added without breaking callers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenMode {
    /// Initialize pristine storage, otherwise validate and reopen.
    #[default]
    CreateIfMissing,
    /// Require a complete initialized database. Never write genesis.
    ExistingOnly,
    /// Require pristine storage. Existing directories alone do not imply a database.
    CreateNew,
}

/// What opening needs regardless of the store: the deployment's genesis, which
/// is identical on every node, and how to treat existing storage.
///
/// Non-exhaustive: build it with `Config::new` and set fields (or use
/// `with_mode`), so store-neutral options can be added without breaking callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    pub genesis: Genesis,
    pub mode: OpenMode,
}

impl Config {
    pub fn new(genesis: Genesis) -> Self {
        Self {
            genesis,
            mode: OpenMode::CreateIfMissing,
        }
    }

    pub fn with_mode(mut self, mode: OpenMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Which built-in store a database opens, and its local tuning. Store settings
/// are node-local and never part of the genesis identity.
///
/// Non-exhaustive so that further stores can be added without breaking
/// callers. A custom store is opened with `Database::from_store` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreConfig {
    /// A fresh in-memory store; nothing survives the last database handle.
    Memory,
    /// An MDBX environment in the directory `path`.
    #[cfg(feature = "mdbx")]
    Mdbx {
        path: std::path::PathBuf,
        options: golemdb_storage::MdbxOptions,
    },
}

impl StoreConfig {
    /// MDBX at `path` with default options (note the 1 GiB size cap of
    /// `MdbxOptions::default()`).
    #[cfg(feature = "mdbx")]
    pub fn mdbx(path: impl Into<std::path::PathBuf>) -> Self {
        Self::Mdbx {
            path: path.into(),
            options: golemdb_storage::MdbxOptions::default(),
        }
    }
}

/// Lets a store config be reused: `Database::open(&store, &config)`.
impl From<&StoreConfig> for StoreConfig {
    fn from(store: &StoreConfig) -> Self {
        store.clone()
    }
}

// A bare path means MDBX with default options, the standard store:
// `Database::open("./data", &config)`.
#[cfg(feature = "mdbx")]
impl From<&str> for StoreConfig {
    fn from(path: &str) -> Self {
        Self::mdbx(path)
    }
}

#[cfg(feature = "mdbx")]
impl From<String> for StoreConfig {
    fn from(path: String) -> Self {
        Self::mdbx(path)
    }
}

#[cfg(feature = "mdbx")]
impl From<&std::path::Path> for StoreConfig {
    fn from(path: &std::path::Path) -> Self {
        Self::mdbx(path)
    }
}

#[cfg(feature = "mdbx")]
impl From<std::path::PathBuf> for StoreConfig {
    fn from(path: std::path::PathBuf) -> Self {
        Self::mdbx(path)
    }
}

impl StoreConfig {
    /// Parse a store file. Relative MDBX paths stay relative to the current
    /// directory; use [`StoreConfig::load`] to resolve them against the file.
    ///
    /// ```yaml
    /// mdbx:
    ///   path: ./data
    ///   options:             # optional; omitted options keep their defaults
    ///     max_map_size: 68719476736
    /// ```
    ///
    /// `memory` selects the in-memory store. Unknown fields are rejected.
    pub fn from_yaml(yaml: &str) -> OpenResult<Self> {
        StoreFile::parse(yaml)?.into_config(None)
    }

    /// Read a store file from this explicit path. A relative MDBX path in the
    /// file is resolved against the file's directory, not the current one.
    /// Keep the store file separate from the genesis file: store settings are
    /// local to a node and may change between restarts; genesis may not.
    pub fn load(path: impl AsRef<Path>) -> OpenResult<Self> {
        let path = path.as_ref();
        let yaml = std::fs::read_to_string(path).map_err(OpenError::Io)?;
        StoreFile::parse(&yaml)?.into_config(path.parent())
    }
}

/// The store file's YAML shape. Kept apart from `StoreConfig` so that
/// golemdb-storage needs no serde, and omitted options fall back to the
/// store implementation's defaults.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(feature = "mdbx"), allow(dead_code))]
enum StoreFile {
    Memory,
    Mdbx(MdbxFile),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "mdbx"), allow(dead_code))]
struct MdbxFile {
    path: std::path::PathBuf,
    #[serde(default)]
    options: MdbxOptionsFile,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "mdbx"), allow(dead_code))]
struct MdbxOptionsFile {
    max_tables: Option<u64>,
    max_map_size: Option<usize>,
    growth_step: Option<usize>,
}

impl StoreFile {
    fn parse(yaml: &str) -> OpenResult<Self> {
        serde_saphyr::from_str(yaml).map_err(OpenError::StoreYaml)
    }

    fn into_config(self, base: Option<&Path>) -> OpenResult<StoreConfig> {
        match self {
            StoreFile::Memory => Ok(StoreConfig::Memory),
            #[cfg(feature = "mdbx")]
            StoreFile::Mdbx(file) => {
                let mut options = golemdb_storage::MdbxOptions::default();
                if let Some(value) = file.options.max_tables {
                    options.max_tables = value;
                }
                if let Some(value) = file.options.max_map_size {
                    options.max_map_size = value;
                }
                if let Some(value) = file.options.growth_step {
                    options.growth_step = value;
                }
                let path = match base {
                    Some(base) if file.path.is_relative() => base.join(&file.path),
                    _ => file.path,
                };
                Ok(StoreConfig::Mdbx { path, options })
            }
            #[cfg(not(feature = "mdbx"))]
            StoreFile::Mdbx(_) => {
                let _ = base;
                Err(OpenError::InvalidConfig(
                    "the store file selects MDBX, but golemdb-api was built without the mdbx feature"
                        .into(),
                ))
            }
        }
    }
}
