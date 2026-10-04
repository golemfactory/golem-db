use golemdb_branch::{BranchId, Branches, CellRead, read_head};
use golemdb_cells::{
    CellKey, CellLimits, CellName, CellNameRef, CellReader, CellValue, reserved, system,
};
use golemdb_merkle::HashProvider;
use golemdb_storage::{ReadTransaction, Store};

use crate::{
    CellPatch, Details, ReadTarget, Record, RecordCells, RecordError, RecordKey, RecordMeta,
    RecordPatch, Result, state,
};

/// Cloneable record facade; clones share the supplied branch manager.
pub struct Records<S, H> {
    branches: Branches<S, H>,
}

impl<S, H> Clone for Records<S, H> {
    fn clone(&self) -> Self {
        Self {
            branches: self.branches.clone(),
        }
    }
}

impl<S: Store, H: HashProvider> Records<S, H> {
    pub fn new(branches: Branches<S, H>) -> Self {
        Self { branches }
    }

    /// Read a full record or an explicit projection. Missing projected cells
    /// are omitted; an empty projection returns only the record's key. Duplicate
    /// names collapse in the result. Names may be raw, including reserved cells.
    /// Record existence and identity are checked even for an empty projection.
    pub fn get(
        &self,
        target: ReadTarget,
        key: RecordKey,
        projection: Option<&[CellName]>,
    ) -> Result<Record> {
        match target {
            ReadTarget::Branch(branch) => self
                .branches
                .read(branch, |cell_reader| {
                    read_branch_record(cell_reader, key, projection)
                })
                .map_err(Into::into),
            ReadTarget::Head | ReadTarget::Commit(_) => {
                let tx = self.branches.store().begin_read()?;
                let head = read_head(&tx)?;
                if let ReadTarget::Commit(requested) = target
                    && requested != head
                {
                    return Err(RecordError::CommitUnavailable { requested, head });
                }
                read_committed_record(&CellReader::new(&tx), key, projection)
            }
        }
    }

    /// Create a record with zero or more user cells, plus its `#key` and
    /// `#meta` cells and its binding. Exact reserved keys are rejected before
    /// branch access; all other validation follows handle checks. Names and
    /// values are checked in byte order before identity/allocation reads.
    /// Returns the key and the write's effects on user cells.
    pub fn create(
        &self,
        branch: BranchId,
        key: RecordKey,
        values: RecordCells,
    ) -> Result<(RecordKey, Details)> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                let limits = state::limits(&cell_reader)?;
                for (name, value) in &values {
                    validate(&limits, name, Some(value))?;
                }
                if state::resolve(&cell_reader, key)?.is_some() {
                    return Err(RecordError::AlreadyExists);
                }
                let (id, next) = state::next_id(&cell_reader)?;
                if cell_writer
                    .scan_prefix(&id.to_be_bytes())?
                    .next()
                    .transpose()?
                    .is_some()
                {
                    return Err(RecordError::CorruptState(
                        "allocator points to occupied record",
                    ));
                }
                let mut details = Details::default();
                for (name, value) in values {
                    details.created(name.as_bytes(), &value);
                    cell_writer.put(CellKey::new(id, name), value);
                }
                cell_writer.put(
                    CellKey::new(id, reserved::KEY),
                    CellValue::from_bytes32(key.0),
                );
                let meta = RecordMeta::default().apply(&details)?;
                cell_writer.put(state::meta_key(id), meta.to_value());
                cell_writer.put(state::binding(key), CellValue::from_u64(id));
                cell_writer.put(state::allocator_key(), CellValue::from_u64(next));
                Ok((key, details))
            })
            .map_err(Into::into)
    }

    /// Apply a partial mutation. Empty patches and removing absent cells are
    /// no-ops on an existing record. Removing the last user cell leaves an empty
    /// record, which exists until deleted. `#meta` is updated from the patch's
    /// effects, which are returned.
    pub fn patch(&self, branch: BranchId, key: RecordKey, changes: RecordPatch) -> Result<Details> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                let id = state::resolve(&cell_reader, key)?.ok_or(RecordError::NotFound)?;
                let limits = state::limits(&cell_reader)?;
                for (name, change) in &changes {
                    validate(
                        &limits,
                        name,
                        match change {
                            CellPatch::Set(value) => Some(value),
                            CellPatch::Remove => None,
                        },
                    )?;
                }
                let meta = state::meta(&cell_reader, id)?;
                let mut details = Details::default();
                for (name, change) in changes {
                    let address = CellKey::new(id, name);
                    let old = cell_writer.get(&address)?;
                    let name = address.name();
                    let name = name.as_bytes();
                    match change {
                        CellPatch::Set(value) => match &old {
                            Some(old) if *old == value => {}
                            Some(old) => {
                                details.updated(name, old, &value);
                                cell_writer.put(address, value);
                            }
                            None => {
                                details.created(name, &value);
                                cell_writer.put(address, value);
                            }
                        },
                        CellPatch::Remove => {
                            if let Some(old) = &old {
                                details.deleted(name, old);
                                cell_writer.delete(address);
                            }
                        }
                    }
                }
                let updated = meta.apply(&details)?;
                if updated != meta {
                    cell_writer.put(state::meta_key(id), updated.to_value());
                }
                Ok(details)
            })
            .map_err(Into::into)
    }

    /// Remove the binding and every live cell, including #key. Successful
    /// deletion never rewinds the allocator; recreation receives a fresh ID.
    /// Returns the deletion's effects on user cells.
    pub fn delete(&self, branch: BranchId, key: RecordKey) -> Result<Details> {
        reject_reserved(key)?;
        self.branches
            .write(branch, |cell_writer| {
                let cell_reader = cell_writer.as_read();
                let id = state::resolve(&cell_reader, key)?.ok_or(RecordError::NotFound)?;
                let cells = cell_reader
                    .scan_prefix(&id.to_be_bytes())?
                    .collect::<golemdb_branch::Result<Vec<_>>>()?;
                let mut details = Details::default();
                for (address, value) in cells {
                    let name = address.name();
                    // System cells such as #key are not counted.
                    if CellNameRef::parse_user(name.as_bytes(), usize::MAX).is_ok() {
                        details.deleted(name.as_bytes(), &value);
                    }
                    cell_writer.delete(address);
                }
                cell_writer.delete(state::binding(key));
                Ok(details)
            })
            .map_err(Into::into)
    }
}

fn reject_reserved(key: RecordKey) -> Result<()> {
    if system::by_key(&key.0).is_some() {
        Err(RecordError::Reserved)
    } else {
        Ok(())
    }
}

fn validate(limits: &CellLimits, name: &CellName, value: Option<&CellValue>) -> Result<()> {
    limits
        .parse_user_name(name.as_bytes())
        .map_err(|error| RecordError::InvalidArgument(error.to_string()))?;
    if let Some(value) = value {
        limits
            .validate_value(value.as_view())
            .map_err(|error| RecordError::InvalidArgument(error.to_string()))?;
    }
    Ok(())
}

fn read_branch_record<R: ReadTransaction>(
    cell_reader: &CellRead<'_, R>,
    key: RecordKey,
    projection: Option<&[CellName]>,
) -> Result<Record> {
    let id = state::resolve(cell_reader, key)?.ok_or(RecordError::NotFound)?;
    let mut values = RecordCells::new();
    match projection {
        Some(names) => {
            for name in names {
                if let Some(value) = cell_reader.get(&CellKey::new(id, name.clone()))? {
                    values.insert(name.clone(), value);
                }
            }
        }
        None => {
            for row in cell_reader.scan_prefix(&id.to_be_bytes())? {
                let (address, value) = row?;
                values.insert(address.name().into(), value);
            }
        }
    }
    Ok(Record { key, cells: values })
}

// Explicit storage path: cell storage owns decoding and scanning; record owns
// the binding/identity checks. No branch or empty overlay is created for reads.
fn read_committed_record<R: ReadTransaction>(
    cell_reader: &CellReader<'_, R>,
    key: RecordKey,
    projection: Option<&[CellName]>,
) -> Result<Record> {
    let id = state::record_id(key, cell_reader.get(&state::binding(key))?)?
        .ok_or(RecordError::NotFound)?;
    state::check_identity(key, cell_reader.get(&CellKey::new(id, reserved::KEY))?)?;
    let mut values = RecordCells::new();
    match projection {
        Some(names) => {
            for name in names {
                if let Some(value) = cell_reader.get(&CellKey::new(id, name.clone()))? {
                    values.insert(name.clone(), value);
                }
            }
        }
        None => {
            for row in cell_reader.scan_record(id)? {
                let (address, value) = row?;
                values.insert(address.name().into(), value);
            }
        }
    }
    Ok(Record { key, cells: values })
}
