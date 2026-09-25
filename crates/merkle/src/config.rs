use crate::HashAlgorithm;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, path::Path};

/// Initial startup configuration. Later the engine will obtain the selected
/// algorithm from genesis/persisted protocol metadata instead of this file.
/// Deserialization requires the field explicitly, rejecting typos and duplicates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HashConfig {
    pub hash_function: HashAlgorithm,
}

impl HashConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        serde_saphyr::from_str(yaml).map_err(ConfigError::Yaml)
    }

    /// Load a caller-selected file. No implicit search path or environment override.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let yaml = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_yaml(&yaml)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(serde_saphyr::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot read hash configuration: {error}"),
            Self::Yaml(error) => write!(f, "invalid hash configuration: {error}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Yaml(e) => Some(e),
        }
    }
}
