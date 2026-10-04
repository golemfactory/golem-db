use golemdb_cells::CellValue;

use crate::{Details, RecordError, Result};

/// A user record's metadata, stored in its `#meta` cell: four counts over its
/// user cells (system cells such as `#key` and `#meta` are not counted), as
/// defined by the metering spec (D4).
///
/// Encoded as the four fields in this order, each `u64` big-endian: 32 bytes,
/// stored as a `bytes32` cell. The layout is under the state root, so it is
/// normative: every implementation must produce the same bytes.
///
/// `cells` lets a client prove it received every cell of a full read: with a
/// trusted root, `cells + 2` distinct cells with valid inclusion proofs are all
/// of them. `cells` and `indexed_cells` bound the record's deletion cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordMeta {
    /// Number of user cells.
    pub cells: u64,
    /// Σ over user cells of `(8 + |name|) + (1 + |value|)`.
    pub cell_bytes: u64,
    /// Number of user cells that are attributes.
    pub indexed_cells: u64,
    /// Σ over indexed cells of `|name| + 2 + |value|`.
    pub index_bytes: u64,
}

impl RecordMeta {
    /// The metadata after a write with these effects.
    pub(crate) fn apply(self, details: &Details) -> Result<Self> {
        let adjust = |count: u64, added: u64, removed: u64| {
            count
                .checked_add(added)
                .ok_or(RecordError::CorruptState("#meta count overflow"))?
                .checked_sub(removed)
                .ok_or(RecordError::CorruptState("#meta count below zero"))
        };
        Ok(Self {
            cells: adjust(self.cells, details.cells_created, details.cells_deleted)?,
            cell_bytes: adjust(
                self.cell_bytes,
                details.cell_bytes_written,
                details.cell_bytes_deleted,
            )?,
            indexed_cells: adjust(
                self.indexed_cells,
                details.index_joins,
                details.index_leaves,
            )?,
            index_bytes: adjust(
                self.index_bytes,
                details.index_bytes_written,
                details.index_bytes_deleted,
            )?,
        })
    }

    /// The `#meta` cell value.
    pub fn to_value(self) -> CellValue {
        let mut bytes = [0; 32];
        for (chunk, count) in bytes.chunks_exact_mut(8).zip([
            self.cells,
            self.cell_bytes,
            self.indexed_cells,
            self.index_bytes,
        ]) {
            chunk.copy_from_slice(&count.to_be_bytes());
        }
        CellValue::from_bytes32(bytes)
    }

    /// Decode a `#meta` cell value; `None` if it is not a non-indexed `bytes32`.
    pub fn from_value(value: &CellValue) -> Option<Self> {
        let bytes = value.as_bytes32().filter(|_| !value.is_indexable())?;
        let count =
            |index: usize| u64::from_be_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap());
        Some(Self {
            cells: count(0),
            cell_bytes: count(1),
            indexed_cells: count(2),
            index_bytes: count(3),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RecordMeta;
    use crate::{Details, RecordError};

    #[test]
    fn counts_that_leave_the_u64_range_are_corrupt_state() {
        let added = Details {
            cells_created: 1,
            ..Details::default()
        };
        let full = RecordMeta {
            cells: u64::MAX,
            ..RecordMeta::default()
        };
        assert!(matches!(
            full.apply(&added),
            Err(RecordError::CorruptState("#meta count overflow"))
        ));
        let removed = Details {
            cells_deleted: 1,
            ..Details::default()
        };
        assert!(matches!(
            RecordMeta::default().apply(&removed),
            Err(RecordError::CorruptState("#meta count below zero"))
        ));
        assert_eq!(RecordMeta::default().apply(&added).unwrap().cells, 1);
    }
}
