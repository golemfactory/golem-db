use golemdb_cells::CellValue;

/// The effects of one write on a record's live user cells, counted as in the
/// metering spec (D4). System cells (`#key`, the binding) are not counted.
///
/// These are effects, not charges: setting a cell to its current value changes
/// nothing and counts nothing, and a failed write applied nothing. A cell's
/// size is `(8 + |name|) + (1 + |value|)`, an index entry's `|name| + 2 + |value|`.
///
/// Non-exhaustive: further counts, such as index terms created, can be added.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Details {
    pub cells_created: u64,
    pub cells_updated: u64,
    pub cells_deleted: u64,
    pub index_joins: u64,
    pub index_leaves: u64,
    pub cell_bytes_written: u64,
    pub cell_bytes_deleted: u64,
    pub index_bytes_written: u64,
    pub index_bytes_deleted: u64,
}

impl Details {
    pub(crate) fn created(&mut self, name: &[u8], value: &CellValue) {
        self.cells_created += 1;
        self.cell_bytes_written += cell_bytes(name, value);
        self.join(name, value);
    }

    pub(crate) fn updated(&mut self, name: &[u8], old: &CellValue, new: &CellValue) {
        self.cells_updated += 1;
        self.cell_bytes_deleted += cell_bytes(name, old);
        self.cell_bytes_written += cell_bytes(name, new);
        self.leave(name, old);
        self.join(name, new);
    }

    pub(crate) fn deleted(&mut self, name: &[u8], old: &CellValue) {
        self.cells_deleted += 1;
        self.cell_bytes_deleted += cell_bytes(name, old);
        self.leave(name, old);
    }

    fn join(&mut self, name: &[u8], value: &CellValue) {
        if value.is_indexable() {
            self.index_joins += 1;
            self.index_bytes_written += index_bytes(name, value);
        }
    }

    fn leave(&mut self, name: &[u8], value: &CellValue) {
        if value.is_indexable() {
            self.index_leaves += 1;
            self.index_bytes_deleted += index_bytes(name, value);
        }
    }
}

/// `(8 + |name|)` for the key (record ID and name), `(1 + |value|)` for the
/// value (type tag and payload).
fn cell_bytes(name: &[u8], value: &CellValue) -> u64 {
    (8 + name.len() + value.encoded_bytes().len()) as u64
}

/// The index term key: name, `0x00` separator, type tag, payload.
fn index_bytes(name: &[u8], value: &CellValue) -> u64 {
    (name.len() + 2 + value.value().len()) as u64
}
