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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
