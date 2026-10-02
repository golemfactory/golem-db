//! Physical tables shared with the engine, updated in its transaction.

use golemdb_storage::Table;

pub const CELL: Table = Table("Cell");
pub const CELL_TRIE: Table = Table("CellTrie");
