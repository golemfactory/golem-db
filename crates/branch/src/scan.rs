use std::{
    cmp::Ordering,
    collections::btree_map,
    iter::{FusedIterator, Peekable},
    ops::Bound::{Excluded, Included, Unbounded},
};

use golemdb_cells::{CellKey, CellValue, tables};
use golemdb_storage::{ReadCursor, ReadTransaction, Scan, scan_prefix};

use crate::{Result, overlay::Entries};

/// Lazy forward merge of cells matching an encoded cell-key prefix. Values are owned; errors are
/// yielded once and exhaust the iterator. No index is consulted or maintained.
pub struct CellScan<'a, C: ReadCursor> {
    origin: Peekable<Scan<C>>,
    overlay: Peekable<btree_map::Range<'a, CellKey, Option<CellValue>>>,
    done: bool,
}

impl<'a, C: ReadCursor> CellScan<'a, C> {
    pub(crate) fn new<R>(origin: &'a R, entries: &'a Entries, prefix: &[u8]) -> Result<Self>
    where
        R: ReadTransaction<Cursor<'a> = C>,
    {
        // Cell keys have an eight-byte minimum. Padding short byte bounds with
        // zeros finds the first representable key at or above each bound.
        let lower_key = |bytes: &[u8]| {
            let mut bytes = bytes.to_vec();
            bytes.resize(bytes.len().max(8), 0);
            CellKey::decode(&bytes).expect("padded cell-key bound")
        };
        let first = lower_key(prefix);
        let mut successor = prefix.to_vec();
        let end = match successor.iter().rposition(|byte| *byte != u8::MAX) {
            Some(index) => {
                successor[index] += 1;
                successor.truncate(index + 1);
                Excluded(lower_key(&successor))
            }
            None => Unbounded,
        };
        Ok(Self {
            origin: scan_prefix(origin, tables::CELL, prefix.to_vec())?.peekable(),
            overlay: entries.range((Included(first), end)).peekable(),
            done: false,
        })
    }

    fn advance(&mut self) -> Result<Option<(CellKey, CellValue)>> {
        loop {
            let order = match (self.origin.peek(), self.overlay.peek()) {
                (Some(Err(_)), _) => return Err(self.origin.next().unwrap().unwrap_err().into()),
                (Some(Ok((key, _))), Some((overlay_key, _))) => key.cmp(&overlay_key.encode()),
                (Some(Ok(_)), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => return Ok(None),
            };
            if order == Ordering::Less {
                let (key, value) = self.origin.next().unwrap()?;
                return Ok(Some((CellKey::decode(&key)?, CellValue::parse(value)?)));
            }
            if order == Ordering::Equal {
                // Do not decode an origin value shadowed by a put or tombstone.
                self.origin.next();
            }
            let (key, value) = self.overlay.next().unwrap();
            if let Some(value) = value {
                return Ok(Some((key.clone(), value.clone())));
            }
        }
    }
}

impl<C: ReadCursor> Iterator for CellScan<'_, C> {
    type Item = Result<(CellKey, CellValue)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.advance() {
            Ok(Some(row)) => Some(Ok(row)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

impl<C: ReadCursor> FusedIterator for CellScan<'_, C> {}
