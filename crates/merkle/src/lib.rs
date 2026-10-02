//! Shared branch-only Merkle tries over transactional key/value storage.
//!
//! Hash domain assignments, leaf payloads and their preimages belong to the owning crate. This crate
//! supplies canonical compact branches, immutable trie mutation, traversal,
//! and configurable hashing. Empty roots hash the empty byte string; singleton
//! roots use their leaf digest directly. Owners retain [`RootRef`] metadata.
//!
//! Select a concrete provider once from [`HashConfig`] at startup. The enum is
//! only a configuration identifier; it cannot be passed as a hash provider.
//!
//! The caller owns the transaction and leaf hashing. This illustrative leaf
//! preimage includes the full path; production owners supply their own codecs.
//!
//! ```
//! use golemdb_merkle::{Keccak256Hasher, HashProvider, LeafRef, RootRef, Trie};
//! use golemdb_storage::{Database, MemoryDatabase, Table, WriteTransaction};
//!
//! let db = MemoryDatabase::new();
//! let hash = Keccak256Hasher;
//! let trie = Trie::<_, 6>::new(Table("ExampleBranches"), 0x05, &hash);
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
pub use hash::{Blake3Hasher, Hash, HashAlgorithm, HashProvider, Keccak256Hasher};
pub use node::BranchNodeCompact;
pub use trie::{LeafRef, RootRef, Trie, Walk};
