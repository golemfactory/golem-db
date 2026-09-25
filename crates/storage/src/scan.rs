use std::{iter::FusedIterator, ops::Bound};

use crate::{Entry, ReadCursor, ReadTransaction, Result, StorageError, Table};

/// Lazy forward scan. Errors are yielded once, then the iterator is exhausted.
pub struct Scan<C> {
    cursor: C,
    excluded_start: Option<Vec<u8>>,
    upper: Bound<Vec<u8>>,
    prefix: Option<Vec<u8>>,
    done: bool,
}

/// Bytewise range scan. Reversed bounds are rejected; equal bounds with an
/// excluded endpoint describe an empty range.
pub fn scan<T: ReadTransaction>(
    tx: &T,
    table: Table,
    lower: Bound<Vec<u8>>,
    upper: Bound<Vec<u8>>,
) -> Result<Scan<T::Cursor<'_>>> {
    let mut empty = false;
    if let (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) =
        (&lower, &upper)
    {
        if a > b {
            return Err(StorageError::InvalidRange);
        }
        empty = a == b
            && (!matches!(lower, Bound::Included(_)) || !matches!(upper, Bound::Included(_)));
    }
    let start = match &lower {
        Bound::Unbounded => &b""[..],
        Bound::Included(key) | Bound::Excluded(key) => key.as_slice(),
    };
    let cursor = tx.cursor(table, start)?;
    Ok(Scan {
        cursor,
        excluded_start: match lower {
            Bound::Excluded(key) => Some(key),
            _ => None,
        },
        upper,
        prefix: None,
        done: empty,
    })
}

/// Prefix scan, including empty and all-0xff prefixes, without a successor key.
pub fn scan_prefix<T: ReadTransaction>(
    tx: &T,
    table: Table,
    prefix: Vec<u8>,
) -> Result<Scan<T::Cursor<'_>>> {
    Ok(Scan {
        cursor: tx.cursor(table, &prefix)?,
        excluded_start: None,
        upper: Bound::Unbounded,
        prefix: Some(prefix),
        done: false,
    })
}

impl<C: ReadCursor> Scan<C> {
    fn advance(&mut self) -> Result<Option<Entry>> {
        let row = self.cursor.next()?;
        if let Some(excluded) = self.excluded_start.take()
            && row.as_ref().is_some_and(|(key, _)| *key == excluded)
        {
            self.cursor.next()
        } else {
            Ok(row)
        }
    }
}

impl<C: ReadCursor> Iterator for Scan<C> {
    type Item = Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.advance() {
            Ok(Some((key, value))) => {
                let within = match &self.upper {
                    Bound::Unbounded => true,
                    Bound::Included(end) => key <= *end,
                    Bound::Excluded(end) => key < *end,
                } && self.prefix.as_ref().is_none_or(|p| key.starts_with(p));
                if within {
                    Some(Ok((key, value)))
                } else {
                    self.done = true;
                    None
                }
            }
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

impl<C: ReadCursor> FusedIterator for Scan<C> {}
