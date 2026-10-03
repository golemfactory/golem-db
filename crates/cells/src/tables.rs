//! Physical tables shared with the layers above, updated in their transaction.

use golemdb_storage::Table;

pub const CELL: Table = Table("Cell");
pub const CELL_TRIE: Table = Table("CellTrie");
