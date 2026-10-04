//! [`RecordOp`]: one operation on one record, for `create`, `patch`, `get` and
//! `delete`. The type parameter names the operation and decides which methods
//! exist, so a create cannot remove cells and a patch always has its key.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    marker::PhantomData,
};

use golemdb_cells::{CellKind, CellName, CellNameRef, CellValue, IntoCellValue};
use golemdb_record::{CellPatch, RecordCells, RecordKey, RecordPatch};

use crate::ApiError;

/// Marker for [`RecordOp<Create>`].
pub enum Create {}
/// Marker for [`RecordOp<Patch>`].
pub enum Patch {}
/// Marker for [`RecordOp<Get>`].
pub enum Get {}
/// Marker for [`RecordOp<Delete>`].
pub enum Delete {}

mod sealed {
    /// Operations that write cells: create and patch.
    pub trait Writes {}
    impl Writes for super::Create {}
    impl Writes for super::Patch {}
}

/// One operation on one record. Build it with the constructor for its
/// operation and pass it to the database method of the same name:
///
/// ```
/// use golemdb_api::{Api, Database, Genesis, ReadTarget, RecordKey, RecordOp};
///
/// let db = Database::open_memory(&Genesis::DEV)?;
/// let branch = db.begin()?;
/// let key = RecordKey([7; 32]);
/// db.create(branch, RecordOp::create().key(key)
///     .attribute("price", 50i32)
///     .field("description", "A product")).into_result()?;
/// db.patch(branch, RecordOp::patch(key)
///     .attribute("price", 75i32)
///     .remove("description")).into_result()?;
/// let record = db.get(ReadTarget::Branch(branch), RecordOp::get(key).only(["price"]))
///     .into_result()?;
/// assert_eq!(record.cells[b"price".as_slice()].as_i32(), Some(75));
/// db.delete(branch, RecordOp::delete(key)).into_result()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Every write declares its kind: `attribute` (indexed) or `field`. Builder
/// steps never fail; the first invalid name, duplicate name or value is kept
/// and reported by the database call as `InvalidArgument`.
///
/// A create cannot remove cells, and only a create takes `.key()`:
///
/// ```compile_fail
/// # use golemdb_api::RecordOp;
/// RecordOp::create().remove("description");
/// ```
///
/// ```compile_fail
/// # use golemdb_api::{RecordKey, RecordOp};
/// RecordOp::patch(RecordKey([7; 32])).key(RecordKey([8; 32]));
/// ```
#[derive(Debug)]
#[must_use = "a record operation does nothing until passed to the database"]
pub struct RecordOp<Op> {
    key: Option<RecordKey>,
    changes: RecordPatch,
    projection: Option<Vec<CellName>>,
    budget: Option<u64>,
    error: Option<ApiError>,
    _op: PhantomData<fn() -> Op>,
}

impl<Op> RecordOp<Op> {
    fn with_key(key: Option<RecordKey>) -> Self {
        Self {
            key,
            changes: BTreeMap::new(),
            projection: None,
            budget: None,
            error: None,
            _op: PhantomData,
        }
    }

    fn fail(&mut self, error: ApiError) {
        self.error.get_or_insert(error);
    }

    /// Cap the cost of this call.
    ///
    /// **Not enforced yet.** Until metering is implemented every call costs 0,
    /// so no budget is ever exceeded and `ApiError::OutOfBudget` is never
    /// returned. The budget is part of the API so that callers can already
    /// state it.
    pub fn budget(mut self, max_cost: u64) -> Self {
        self.budget = Some(max_cost);
        self
    }

    /// The budget given with [`RecordOp::budget`], if any.
    pub fn max_cost(&self) -> Option<u64> {
        self.budget
    }

    /// The record key: always set for patch, get and delete; for a create,
    /// only when the caller assigns the key.
    pub fn record_key(&self) -> Option<RecordKey> {
        self.key
    }
}

impl<Op: sealed::Writes> RecordOp<Op> {
    /// Write an attribute: an indexed cell, usable in filters.
    pub fn attribute(self, name: &str, value: impl IntoCellValue) -> Self {
        self.write(name, value, CellKind::Attribute)
    }

    /// Write a field: a cell that is stored but not indexed.
    pub fn field(self, name: &str, value: impl IntoCellValue) -> Self {
        self.write(name, value, CellKind::Field)
    }

    /// The value this operation writes to `name`, if any. For inspecting an
    /// operation, for example in a mock implementation of `Api`.
    pub fn value(&self, name: &str) -> Option<&CellValue> {
        match self.changes.get(name.as_bytes()) {
            Some(CellPatch::Set(value)) => Some(value),
            _ => None,
        }
    }

    fn write(mut self, name: &str, value: impl IntoCellValue, kind: CellKind) -> Self {
        match value
            .into_cell_value()
            .and_then(|value| value.with_kind(kind))
        {
            Ok(value) => self.change(name, CellPatch::Set(value)),
            Err(error) => {
                self.fail(error.into());
                self
            }
        }
    }

    fn change(mut self, name: &str, change: CellPatch) -> Self {
        // Grammar only: the deployment's #maxCellNameLen is checked by the call.
        match CellNameRef::parse_user(name.as_bytes(), usize::MAX) {
            Ok(name) => match self.changes.entry(name.into()) {
                Entry::Vacant(entry) => {
                    entry.insert(change);
                }
                Entry::Occupied(entry) => {
                    let message = format!("duplicate cell name: {}", entry.key());
                    self.fail(ApiError::invalid_argument(message));
                }
            },
            Err(error) => self.fail(error.into()),
        }
        self
    }
}

impl RecordOp<Create> {
    /// A new record. Name its key with [`RecordOp::key`] when the database
    /// uses caller-assigned keys.
    pub fn create() -> Self {
        Self::with_key(None)
    }

    /// The new record's key, for databases with caller-assigned keys.
    pub fn key(mut self, key: RecordKey) -> Self {
        if self.key.is_some() {
            self.fail(ApiError::invalid_argument("record key given twice"));
        } else {
            self.key = Some(key);
        }
        self
    }

    pub(crate) fn into_create(self) -> Result<(Option<RecordKey>, RecordCells), ApiError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let cells = self
            .changes
            .into_iter()
            .map(|(name, change)| match change {
                CellPatch::Set(value) => (name, value),
                CellPatch::Remove => unreachable!("a create has no remove method"),
            })
            .collect();
        Ok((self.key, cells))
    }
}

impl RecordOp<Patch> {
    /// Changes to the existing record `key`.
    pub fn patch(key: RecordKey) -> Self {
        Self::with_key(Some(key))
    }

    /// Remove a cell. Removing a cell the record does not have is a no-op.
    pub fn remove(self, name: &str) -> Self {
        self.change(name, CellPatch::Remove)
    }

    /// Whether this patch removes `name`.
    pub fn removes(&self, name: &str) -> bool {
        matches!(self.changes.get(name.as_bytes()), Some(CellPatch::Remove))
    }

    pub(crate) fn into_patch(self) -> Result<(RecordKey, RecordPatch), ApiError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok((self.key.expect("a patch always has its key"), self.changes)),
        }
    }
}

impl RecordOp<Get> {
    /// Read the record `key`: all cells unless narrowed with [`RecordOp::only`].
    pub fn get(key: RecordKey) -> Self {
        Self::with_key(Some(key))
    }

    /// Read only these cells. Names are taken as given, so reserved cells such
    /// as `#key` can be named; missing cells are omitted from the result. An
    /// empty list returns no cells but still checks that the record exists.
    pub fn only<I, N>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: AsRef<[u8]>,
    {
        let mut names: Vec<CellName> = names
            .into_iter()
            .map(|name| CellNameRef::raw(name.as_ref()).into())
            .collect();
        names.sort();
        names.dedup();
        self.projection = Some(names);
        self
    }

    pub(crate) fn into_get(self) -> (RecordKey, Option<Vec<CellName>>) {
        (self.key.expect("a get always has its key"), self.projection)
    }
}

impl RecordOp<Delete> {
    /// Delete the record `key`.
    pub fn delete(key: RecordKey) -> Self {
        Self::with_key(Some(key))
    }

    pub(crate) fn into_delete(self) -> RecordKey {
        self.key.expect("a delete always has its key")
    }
}
