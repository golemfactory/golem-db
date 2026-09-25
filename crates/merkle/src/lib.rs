//! Shared branch-only Merkle tries over transactional key/value storage.
//!
//! Leaf payloads and their preimages belong to the owning crate. This crate
//! supplies canonical compact branches, immutable trie mutation, traversal,
//! and configurable hashing. Empty roots hash the empty byte string; singleton
//! roots use their leaf digest directly. Owners retain [`RootRef`] metadata.
//!
//! ```
//! use golemdb_merkle::{HashConfig, HashProvider};
//! let config = HashConfig::from_yaml("hash_function: keccak-256")?;
//! let hash = config.hash_function;
//! assert_eq!(hash.hash(b"abc"), hash.hash_parts(&[b"a", b"bc"]));
//! # Ok::<(), golemdb_merkle::ConfigError>(())
//! ```
//!
//! The caller owns the transaction and leaf hashing. This illustrative leaf
//! preimage includes the full path; production owners supply their own codecs.
//!
//! ```
//! use golemdb_merkle::{BranchDomain, HashAlgorithm, HashProvider, LeafRef, RootRef, Trie};
//! use golemdb_storage::{Database, MemoryDatabase, Table, WriteTransaction};
//!
//! let db = MemoryDatabase::new();
//! let hash = HashAlgorithm::Keccak256;
//! let trie = Trie::<_, 6>::new(Table("ExampleBranches"), BranchDomain::Bitmap, &hash)?;
//! let mut tx = db.begin_write()?;
//! let mut root = RootRef::Empty;
//! for path in [[0; 6], [1; 6]] {
//!     let leaf = LeafRef { path, hash: hash.hash_parts(&[&[0x04], &path, b"payload"]) };
//!     root = trie.insert(&mut tx, root, leaf)?;
//! }
//! tx.commit()?;
//! let read = db.begin_read()?;
//! assert_eq!(trie.walk(&read, root).collect::<golemdb_merkle::Result<Vec<_>>>()?.len(), 2);
//! assert!(trie.get(&read, root, &[1; 6])?.is_some());
//! # Ok::<(), golemdb_merkle::MerkleError>(())
//! ```

mod config;
mod error;
mod hash;
mod node;
mod path;
mod trie;

pub use config::{ConfigError, HashConfig};
pub use error::{MerkleError, Result};
pub use hash::{Hash, HashAlgorithm, HashProvider};
pub use node::{BranchDomain, BranchNodeCompact};
pub use trie::{LeafRef, RootRef, Trie, Walk};
