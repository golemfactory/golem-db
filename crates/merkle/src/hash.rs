use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

pub type Hash = [u8; 32];

/// Hash the concatenation of byte slices, with no implicit framing or domain.
/// Callers define canonical preimages and supply domain bytes explicitly.
/// Use the same provider for routing, leaf hashes, and branch hashes.
pub trait HashProvider {
    fn hash_parts(&self, parts: &[&[u8]]) -> Hash;

    fn hash(&self, bytes: &[u8]) -> Hash {
        self.hash_parts(&[bytes])
    }
}

/// Supported protocol hash choices. Additional algorithms require explicit
/// implementations and identifiers; unknown names never fall back to Keccak.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum HashAlgorithm {
    #[default]
    #[serde(rename = "keccak-256")]
    Keccak256,
}

impl HashProvider for HashAlgorithm {
    fn hash_parts(&self, parts: &[&[u8]]) -> Hash {
        match self {
            Self::Keccak256 => {
                let mut digest = Keccak256::new();
                for part in parts {
                    digest.update(part);
                }
                digest.finalize().into()
            }
        }
    }
}
