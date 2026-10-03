//! Transactional two-tier index, ordered terms, and canonical bitmap chunks.
//!
//! A record ID splits into a 48-bit path and a 16-bit offset. Persisted chunks
//! encode their six-byte big-endian path followed by canonical portable Roaring
//! bytes. Empty chunks are working state only; persistence removes their leaf.
//! [`Bitmap`] assembles chunks for queries, without owning storage or hashing.
//! [`IndexTerm`] supplies ordered table keys and term leaf hashes. All hashing
//! receives the same configured [`golemdb_merkle::HashProvider`] from the caller.
//! [`Index`] coordinates the flat term table, BitmapTrie, and IndexTrie inside
//! the caller's transaction. Trie adapters remain private implementation details.
//!
//! ```
//! use golemdb_cells::CellType;
//! use golemdb_index::{Index, IndexTerm, PostingChange};
//! use golemdb_merkle::{Keccak256Hasher, RootRef};
//! use golemdb_storage::{Store, MemoryStore, WriteTransaction};
//!
//! let db = MemoryStore::new();
//! let hasher = Keccak256Hasher;
//! let index = Index::new(&hasher);
//! let term = IndexTerm::new("color", CellType::Str, b"blue")?;
//! let mut tx = db.begin_write()?;
//! let update = index.apply(&mut tx, RootRef::Empty, [
//!     PostingChange::Add { term: term.clone(), record_id: 42 },
//! ])?;
//! // The caller can write its head and history in this same transaction.
//! tx.commit()?;
//! let read = db.begin_read()?;
//! assert!(index.bitmap(&read, &term)?.unwrap().treemap().contains(42));
//! assert_eq!(index.reopen(&read, update.root.hash(&hasher))?, update.root);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```
//! use golemdb_cells::{CellType, Width};
//! use golemdb_index::{BitmapContainer, IndexTerm};
//! use golemdb_merkle::Keccak256Hasher;
//! let hasher = Keccak256Hasher;
//! let term = IndexTerm::new("Price", CellType::Int(Width::W4), &100i32.to_be_bytes())?;
//! let chunk = BitmapContainer::from_values(0, [64, 65])?;
//! // A singleton BitmapTrie's root is its only container's leaf hash.
//! let bitmap_root = chunk.leaf_hash(&hasher)?;
//! let index_leaf = term.leaf_hash(&bitmap_root, &hasher);
//! assert_ne!(bitmap_root, index_leaf);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod bitmap;
mod bitmap_trie;
pub mod container;
pub mod error;
mod index;
mod index_trie;
pub mod path;
pub mod tables;
mod term;

pub use bitmap::Bitmap;
pub use container::BitmapContainer;
pub use error::{BitmapError, IndexError, Result};
pub use index::{Index, IndexUpdate, PostingChange, TermRootChange, TermScan};
pub use term::{IndexTerm, TermError};

/// IndexTrie routes by a complete term digest (32 bytes).
/// This is a byte width, not the compressed trie's depth.
pub const INDEX_TRIE_PATH_BYTES: usize = size_of::<golemdb_merkle::Hash>();

/// BitmapTrie routes by the high 48 bits of a record ID (6 bytes).
/// A hexary path has two digits per byte, so its maximum depth is 12.
pub const BITMAP_TRIE_PATH_BYTES: usize = PATH_BITS as usize / 8;

/// Domain byte for index term leaf hashes.
pub const INDEX_LEAF_DOMAIN: u8 = 0x02;
/// Domain byte for IndexTrie branch hashes.
pub const INDEX_BRANCH_DOMAIN: u8 = 0x03;
/// Domain byte for bitmap container leaf hashes.
pub const BITMAP_LEAF_DOMAIN: u8 = 0x04;
/// Domain byte for BitmapTrie branch hashes.
pub const BITMAP_BRANCH_DOMAIN: u8 = 0x05;

pub(crate) const PATH_BITS: u8 = 48;
pub(crate) const PATH_BYTE_OFFSET: usize = 8 - BITMAP_TRIE_PATH_BYTES;
pub(crate) const MAX_PATH: u64 = (1 << PATH_BITS) - 1;
pub(crate) const CHUNK_BITS: u8 = 16;
