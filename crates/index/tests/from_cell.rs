//! Regression tests: `IndexTerm::from_cell` must invert the cell codec's
//! stored order forms (sign-flipped integers, sortable IEEE floats) before
//! constructing terms, so a cell-derived term equals the term built from the
//! raw value.

use golemdb_cells::{CellType as T, CellValue, FloatWidth as F, Width as W};
use golemdb_index::IndexTerm;

#[test]
fn from_cell_int_inverts_sign_flip() {
    // -1i32: raw FF FF FF FF, stored 7F FF FF FF (sign bit flipped).
    let cell = CellValue::new(T::Int(W::W4), &[0x7f, 0xff, 0xff, 0xff], true).unwrap();
    let via_cell = IndexTerm::from_cell("I", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("I", T::Int(W::W4), &(-1i32).to_be_bytes()).unwrap();
    assert_eq!(via_cell, via_raw);
}

#[test]
fn from_cell_int_round_trip_through_wire() {
    // Parse the cell from wire bytes (tag 0x10 + stored form), then term it.
    let wire = [0x90u8, 0x7f, 0xff, 0xff, 0xff]; // indexable i32 -1
    let cell = CellValue::parse_prefix(&wire).unwrap().0;
    let via_cell = IndexTerm::from_cell("I", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("I", T::Int(W::W4), &(-1i32).to_be_bytes()).unwrap();
    assert_eq!(via_cell, via_raw);
}

#[test]
fn from_cell_float_inverts_sortable_transform() {
    // -1f32: raw BF 80 00 00, stored 40 7F FF FF (complement).
    let cell = CellValue::new(T::Float(F::F32), &[0x40, 0x7f, 0xff, 0xff], true).unwrap();
    let via_cell = IndexTerm::from_cell("F", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("F", T::Float(F::F32), &(-1f32).to_be_bytes()).unwrap();
    assert_eq!(via_cell, via_raw);

    // +1f32: raw 3F 80 00 00, stored BF 80 00 00 (sign bit set).
    let cell = CellValue::new(T::Float(F::F32), &[0xbf, 0x80, 0x00, 0x00], true).unwrap();
    let via_cell = IndexTerm::from_cell("F", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("F", T::Float(F::F32), &1f32.to_be_bytes()).unwrap();
    assert_eq!(via_cell, via_raw);

    // f64 negative: raw BF F0 00 00 00 00 00 00, stored 40 0F FF FF FF FF FF FF.
    let cell = CellValue::new(
        T::Float(F::F64),
        &[0x40, 0x0f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        true,
    )
    .unwrap();
    let via_cell = IndexTerm::from_cell("F", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("F", T::Float(F::F64), &(-1f64).to_be_bytes()).unwrap();
    assert_eq!(via_cell, via_raw);
}

#[test]
fn from_cell_uint_and_bool_are_passthrough() {
    let one = 1u32.to_be_bytes();
    let cell = CellValue::new(T::Uint(W::W4), &one, true).unwrap();
    let via_cell = IndexTerm::from_cell("U", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("U", T::Uint(W::W4), &one).unwrap();
    assert_eq!(via_cell, via_raw);

    let cell = CellValue::new(T::Bool, &[1], true).unwrap();
    let via_cell = IndexTerm::from_cell("B", cell).unwrap().unwrap();
    let via_raw = IndexTerm::new("B", T::Bool, &[1]).unwrap();
    assert_eq!(via_cell, via_raw);
}
