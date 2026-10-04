//! [`RecordOp`]: one operation on one record, for `create`, `patch`, `get` and
//! `delete`. The type parameter names the operation and decides which methods
//! exist and what the operation holds, so a create cannot remove cells and a
//! patch always has its key.

use std::collections::{BTreeMap, btree_map::Entry};

use golemdb_cells::{
    CellKind, CellName, CellNameError, CellNameRef, CellParseError, CellValue, IntoCellValue,
};
use golemdb_record::{CellPatch, RecordCells, RecordKey, RecordPatch};

use crate::ApiError;

/// Marker for [`RecordOp<Create>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Create {}
/// Marker for [`RecordOp<Patch>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patch {}
/// Marker for [`RecordOp<Get>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Get {}
/// Marker for [`RecordOp<Delete>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delete {}

mod sealed {
    use std::fmt::Debug;

    use golemdb_cells::{CellName, CellValue};
    use golemdb_record::{CellPatch, RecordCells, RecordKey, RecordPatch};

    /// What each operation holds.
    pub trait Kind {
        type Data: Debug + Clone + PartialEq + Eq;
        fn key(data: &Self::Data) -> Option<RecordKey>;
    }

    /// Operations that write cells: create and patch.
    pub trait Writes: Kind {
        /// Stage a cell; `false` if this operation already writes or removes it.
        fn set(data: &mut Self::Data, name: CellName, value: CellValue) -> bool;
        fn value<'a>(data: &'a Self::Data, name: &[u8]) -> Option<&'a CellValue>;
    }

    impl Kind for super::Create {
        type Data = (Option<RecordKey>, RecordCells);
        fn key(data: &Self::Data) -> Option<RecordKey> {
            data.0
        }
    }

    impl Kind for super::Patch {
        type Data = (RecordKey, RecordPatch);
        fn key(data: &Self::Data) -> Option<RecordKey> {
            Some(data.0)
        }
    }

    impl Kind for super::Get {
        type Data = (RecordKey, Option<Vec<CellName>>);
        fn key(data: &Self::Data) -> Option<RecordKey> {
            Some(data.0)
        }
    }

    impl Kind for super::Delete {
        type Data = RecordKey;
        fn key(data: &Self::Data) -> Option<RecordKey> {
            Some(*data)
        }
    }

    impl Writes for super::Create {
        fn set(data: &mut Self::Data, name: CellName, value: CellValue) -> bool {
            super::insert(&mut data.1, name, value)
        }
        fn value<'a>(data: &'a Self::Data, name: &[u8]) -> Option<&'a CellValue> {
            data.1.get(name)
        }
    }

    impl Writes for super::Patch {
        fn set(data: &mut Self::Data, name: CellName, value: CellValue) -> bool {
            super::insert(&mut data.1, name, CellPatch::Set(value))
        }
        fn value<'a>(data: &'a Self::Data, name: &[u8]) -> Option<&'a CellValue> {
            match data.1.get(name) {
                Some(CellPatch::Set(value)) => Some(value),
                _ => None,
            }
        }
    }
}

fn insert<V>(cells: &mut BTreeMap<CellName, V>, name: CellName, value: V) -> bool {
    match cells.entry(name) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            true
        }
        Entry::Occupied(_) => false,
    }
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
/// and reported by the database call as `InvalidArgument`, naming the step
/// (`field("price")`). [`RecordOp::validate`] reports it early.
///
/// Operations are plain values: they can be cloned, for example to repeat a
/// call after a `Conflict`, and compared, for example in a mock.
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
///
/// Reads and deletes write no cells:
///
/// ```compile_fail
/// # use golemdb_api::{RecordKey, RecordOp};
/// RecordOp::get(RecordKey([7; 32])).field("price", 1i32);
/// ```
///
/// ```compile_fail
/// # use golemdb_api::{RecordKey, RecordOp};
/// RecordOp::delete(RecordKey([7; 32])).remove("price");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a record operation does nothing until passed to the database"]
pub struct RecordOp<Op: sealed::Kind> {
    data: Op::Data,
    budget: Option<u64>,
    error: Option<BuildError>,
}

impl<Op: sealed::Kind> RecordOp<Op> {
    fn new(data: Op::Data) -> Self {
        Self {
            data,
            budget: None,
            error: None,
        }
    }

    fn fail(&mut self, step: &'static str, name: Option<&str>, problem: Problem) {
        self.error.get_or_insert_with(|| BuildError {
            step,
            name: name.map(str::to_owned),
            problem,
        });
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
        Op::key(&self.data)
    }

    /// The error the database call would report for how this operation was
    /// built, if any: the first invalid step. Checks the name grammar only;
    /// the deployment's limits, such as `#maxCellNameLen`, are checked by the
    /// call.
    pub fn validate(&self) -> Result<(), ApiError> {
        match &self.error {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }

    /// The operation's data, or the first build error.
    fn into_data(self) -> Result<Op::Data, ApiError> {
        match self.error {
            Some(error) => Err(error.into()),
            None => Ok(self.data),
        }
    }
}

impl<Op: sealed::Writes> RecordOp<Op> {
    /// Write an attribute: an indexed cell, usable in filters.
    ///
    /// The Rust type decides the cell type. An unsuffixed integer literal is
    /// an `i32`; write `50i64` or `50u64` for other widths. Byte strings are
    /// slices or fixed-width arrays: `&b"abc"[..]`, or `*b"abcd"` for `bytes4`.
    pub fn attribute(self, name: &str, value: impl IntoCellValue) -> Self {
        self.write("attribute", name, value, CellKind::Attribute)
    }

    /// Write a field: a cell that is stored but not indexed. Values convert
    /// as for [`RecordOp::attribute`].
    pub fn field(self, name: &str, value: impl IntoCellValue) -> Self {
        self.write("field", name, value, CellKind::Field)
    }

    /// The value this operation writes to `name`, if any. For inspecting an
    /// operation, for example in a mock implementation of `Api`.
    pub fn value(&self, name: &str) -> Option<&CellValue> {
        Op::value(&self.data, name.as_bytes())
    }

    fn write(
        mut self,
        step: &'static str,
        name: &str,
        value: impl IntoCellValue,
        kind: CellKind,
    ) -> Self {
        let Some(cell_name) = self.user_name(step, name) else {
            return self;
        };
        match value
            .into_cell_value()
            .and_then(|value| value.with_kind(kind))
        {
            Ok(value) => {
                if !Op::set(&mut self.data, cell_name, value) {
                    self.fail(step, Some(name), Problem::Duplicate);
                }
            }
            Err(error) => self.fail(step, Some(name), Problem::Value(error)),
        }
        self
    }

    /// `name` as a user cell name, or `None` after recording why not.
    fn user_name(&mut self, step: &'static str, name: &str) -> Option<CellName> {
        // Grammar only: the deployment's #maxCellNameLen is checked by the call.
        match CellNameRef::parse_user(name.as_bytes(), usize::MAX) {
            Ok(cell_name) => Some(cell_name.into()),
            Err(error) => {
                self.fail(step, Some(name), Problem::Name(error));
                None
            }
        }
    }
}

impl RecordOp<Create> {
    /// A new record. Name its key with [`RecordOp::key`] when the database
    /// uses caller-assigned keys.
    pub fn create() -> Self {
        Self::new((None, BTreeMap::new()))
    }

    /// The new record's key, for databases with caller-assigned keys.
    pub fn key(mut self, key: RecordKey) -> Self {
        if self.data.0.is_some() {
            self.fail("key", None, Problem::KeyTwice);
        } else {
            self.data.0 = Some(key);
        }
        self
    }

    pub(crate) fn into_create(self) -> Result<(Option<RecordKey>, RecordCells), ApiError> {
        self.into_data()
    }
}

impl RecordOp<Patch> {
    /// Changes to the existing record `key`.
    pub fn patch(key: RecordKey) -> Self {
        Self::new((key, BTreeMap::new()))
    }

    /// Remove a cell. Removing a cell the record does not have is a no-op.
    pub fn remove(mut self, name: &str) -> Self {
        if let Some(cell_name) = self.user_name("remove", name)
            && !insert(&mut self.data.1, cell_name, CellPatch::Remove)
        {
            self.fail("remove", Some(name), Problem::Duplicate);
        }
        self
    }

    /// Whether this patch removes `name`.
    pub fn removes(&self, name: &str) -> bool {
        matches!(self.data.1.get(name.as_bytes()), Some(CellPatch::Remove))
    }

    pub(crate) fn into_patch(self) -> Result<(RecordKey, RecordPatch), ApiError> {
        self.into_data()
    }
}

impl RecordOp<Get> {
    /// Read the record `key`: all cells unless narrowed with [`RecordOp::only`].
    pub fn get(key: RecordKey) -> Self {
        Self::new((key, None))
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
        self.data.1 = Some(names);
        self
    }

    /// A get has no failing step.
    pub(crate) fn into_get(self) -> (RecordKey, Option<Vec<CellName>>) {
        self.data
    }
}

impl RecordOp<Delete> {
    /// Delete the record `key`.
    pub fn delete(key: RecordKey) -> Self {
        Self::new(key)
    }

    /// A delete has no failing step.
    pub(crate) fn into_delete(self) -> RecordKey {
        self.data
    }
}

/// The first invalid builder step, kept until the call reports it. Cloneable,
/// unlike `ApiError`, so that operations stay plain values.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildError {
    step: &'static str,
    name: Option<String>,
    problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Problem {
    Name(CellNameError),
    Value(CellParseError),
    Duplicate,
    KeyTwice,
}

impl From<BuildError> for ApiError {
    fn from(error: BuildError) -> Self {
        let step = match &error.name {
            Some(name) => format!("{}({name:?})", error.step),
            None => format!("{}()", error.step),
        };
        // The message names the step; a typed cause stays the source, so error
        // reports print each piece of text once.
        let (what, source): (_, Option<Box<dyn std::error::Error + Send + Sync>>) =
            match error.problem {
                Problem::Name(cause) => ("invalid cell name", Some(Box::new(cause))),
                Problem::Value(cause) => ("invalid cell value", Some(Box::new(cause))),
                Problem::Duplicate => ("the operation already writes or removes this cell", None),
                Problem::KeyTwice => ("the record key is given twice", None),
            };
        ApiError::InvalidArgument {
            message: format!("{step}: {what}"),
            source,
        }
    }
}
