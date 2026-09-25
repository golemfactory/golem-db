//! Physical table names shared with the engine. Writes belong to one transaction.
use golemdb_storage::Table;

pub const INDEX: Table = Table("Index");
pub const BITMAP_CONTAINER: Table = Table("BitmapContainer");
pub const BITMAP_TRIE: Table = Table("BitmapTrie");
pub const INDEX_TRIE: Table = Table("IndexTrie");
