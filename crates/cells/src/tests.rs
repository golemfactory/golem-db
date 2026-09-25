//! Unit tests: [`vectors`] holds the accept/reject tables, [`properties`] the
//! generated-input checks, [`order`] stored-form ordering, [`keys`] cell names.

mod keys;
mod order;
mod properties;
mod vectors;

use crate::*;
use vectors::*;

#[test]
fn vectors_decode_and_re_encode() {
    for (name, bytes, indexable, ty, value) in VECTORS {
        let cell = CellValue::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(cell.cell_type(), *ty, "{name}");
        assert_eq!(cell.value(), *value, "{name}");
        assert_eq!(cell.is_indexable(), *indexable, "{name}");
        assert_eq!(cell.encode(), *bytes, "{name}");
    }
}

#[test]
fn vectors_cover_every_type() {
    for id in 0..128 {
        if CellType::from_id(id).is_ok() {
            assert!(
                VECTORS.iter().any(|(_, b, ..)| b[0] & !INDEXABLE_BIT == id),
                "type id {id} has no vector"
            );
        }
    }
}

#[test]
fn bad_vectors_are_rejected_for_the_stated_reason() {
    for (name, bytes, expected) in BAD_VECTORS {
        assert_eq!(CellValue::parse(bytes).err(), Some(*expected), "{name}");
    }
}

#[test]
fn id_space_matches_the_spec() {
    for id in 0..128 {
        match (id, CellType::from_id(id)) {
            (0, got) => assert_eq!(got, Err(CellParseError::AbsentTag)),
            (5..=7 | 26 | 27 | 30.., got) => {
                assert_eq!(got, Err(CellParseError::ReservedType(id)))
            }
            (_, got) => assert_eq!(got.map(CellType::id), Ok(id), "id {id}"),
        }
    }
}

/// Every metadata byte against every payload length: parsing never panics,
/// and whatever parses re-encodes to the same bytes.
#[test]
fn every_metadata_byte_and_length() {
    // `00 00 00 01` is a length prefix of 1 for `str`/`bytes`, followed by
    // `01`s, which are valid content for every type but floats.
    let mut payload = vec![0x00, 0x00, 0x00, 0x01];
    payload.extend([0x01; 36]);
    for metadata in 0..=u8::MAX {
        for len in 0..=payload.len() {
            let bytes = [&[metadata][..], &payload[..len]].concat();
            let accepted = match CellType::from_id(metadata & !INDEXABLE_BIT) {
                Err(_) => false,
                Ok(CellType::Bytes) if metadata & INDEXABLE_BIT != 0 => false,
                Ok(ty) => match ty.width() {
                    Some(n) => len == n && ty.validate(&payload[..n]).is_ok(),
                    None => len == 5,
                },
            };
            match CellValue::parse(&bytes) {
                Ok(cell) => {
                    assert!(accepted, "{metadata:#04x} len {len}: should fail");
                    assert_eq!(cell.encode(), bytes, "{metadata:#04x} len {len}");
                }
                Err(_) => assert!(!accepted, "{metadata:#04x} len {len}: should parse"),
            }
        }
    }
}

/// `0x00` is not a type, so a tombstone is `Option<CellValue>::None`, which
/// costs no space.
#[test]
fn absence_is_a_free_option() {
    assert_eq!(
        size_of::<Option<CellValue<'_>>>(),
        size_of::<CellValue<'_>>()
    );
}

#[test]
fn accessors_decode_their_rust_equivalents() {
    fn parse(bytes: &[u8]) -> CellValue<'_> {
        CellValue::parse(bytes).unwrap()
    }
    fn cell(id: u8, value: &[u8]) -> Vec<u8> {
        [&[id][..], value].concat()
    }

    assert_eq!(parse(&[0x01, 1]).as_bool(), Some(true));
    assert_eq!(parse(&[0x02, 0, 0, 0, 2, b'h', b'i']).as_str(), Some("hi"));
    assert_eq!(
        parse(&[0x03, 0, 0, 0, 2, 0xDE, 0xAD]).as_bytes(),
        Some(&[0xDE, 0xAD][..])
    );
    assert_eq!(parse(&[0x08, 1, 2, 3, 4]).as_bytes4(), Some([1, 2, 3, 4]));
    assert_eq!(parse(&cell(0x04, &[7; 20])).as_bytes20(), Some([7; 20]));
    assert_eq!(
        parse(&cell(0x0D, &u64::MAX.to_be_bytes())).as_u64(),
        Some(u64::MAX)
    );

    // Signed values and floats are stored in order form; the accessors decode.
    assert_eq!(parse(&[0x10, 0x7F, 0xFF, 0xFF, 0xFF]).as_i32(), Some(-1));
    assert_eq!(
        parse(&cell(0x11, &flip_sign(i64::MIN.to_be_bytes()))).as_i64(),
        Some(i64::MIN)
    );
    assert_eq!(
        parse(&cell(0x13, &flip_sign([0xFF; 32]))).as_i256_be(),
        Some([0xFF; 32])
    );
    assert_eq!(
        parse(&cell(0x14, &flip_sign((-5i32).to_be_bytes()))).as_dec32_unscaled(),
        Some(-5)
    );
    assert_eq!(parse(&[0x18, 0xBF, 0xC0, 0, 0]).as_f32(), Some(1.5));
    assert_eq!(
        parse(&cell(0x19, &encode_float((-0.25f64).to_be_bytes()))).as_f64(),
        Some(-0.25)
    );
    assert_eq!(
        parse(&cell(0x1C, &flip_sign(20_000i32.to_be_bytes()))).as_date32(),
        Some(20_000)
    );
    assert_eq!(
        parse(&cell(0x1D, &flip_sign((-1i64).to_be_bytes()))).as_timestamp64(),
        Some(-1)
    );
}

/// The same eight bytes under `bytes8`, `u64`, `i64`, `dec64`, `f64` and
/// `timestamp64` read out under exactly one accessor each.
#[test]
fn accessors_are_exclusive() {
    let payload = [0x80, 0, 0, 0, 0, 0, 0, 1];
    for id in [0x09, 0x0D, 0x11, 0x15, 0x19, 0x1D] {
        let bytes = [&[id][..], &payload].concat();
        let cell = CellValue::parse(&bytes).unwrap();
        let hits = [
            cell.as_bytes8().is_some(),
            cell.as_u64().is_some(),
            cell.as_i64().is_some(),
            cell.as_dec64_unscaled().is_some(),
            cell.as_f64().is_some(),
            cell.as_timestamp64().is_some(),
        ];
        assert_eq!(hits.iter().filter(|h| **h).count(), 1, "id {id:#04x}");
        assert_eq!(cell.as_u32(), None, "id {id:#04x}");
    }
}

/// `bytes` accepts what `str` rejects: UTF-8 is the type's rule, not the
/// format's.
#[test]
fn bytes_accepts_what_str_rejects() {
    for (name, bytes, expected) in BAD_VECTORS {
        if let CellParseError::InvalidUtf8 { .. } = expected {
            let as_bytes = [&[CellType::Bytes.id()][..], &bytes[1..]].concat();
            let (cell, _) =
                CellValue::parse_prefix(&as_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(cell.cell_type(), CellType::Bytes, "{name}");
        }
    }
}

#[test]
fn over_long_values_cannot_be_built() {
    let too_long = [b'a'; MAX_VALUE_LEN + 1];
    for ty in [CellType::Str, CellType::Bytes] {
        assert_eq!(
            CellValue::new(ty, &too_long, false),
            Err(CellParseError::TooLong {
                actual: MAX_VALUE_LEN + 1
            })
        );
    }
    assert!(CellValue::new(CellType::Bytes, &[0; MAX_VALUE_LEN], false).is_ok());
}

/// Packed cells walk back unchanged, and cutting the run anywhere is caught
/// rather than read as a shorter valid run.
#[test]
fn packed_cells_walk_and_truncation_is_caught() {
    let seven = 7u64.to_be_bytes();
    let cells = [
        CellValue::new(CellType::Str, b"hi", true).unwrap(),
        CellValue::new(CellType::Uint(Width::W8), &seven, false).unwrap(),
        CellValue::new(CellType::Bytes, &[0xDE, 0xAD], false).unwrap(),
        CellValue::new(CellType::Bool, &[1], false).unwrap(),
    ];
    let mut packed = Vec::new();
    for cell in &cells {
        cell.encode_into(&mut packed);
    }

    let walk = |mut rest: &[u8]| {
        let mut walked = 0;
        while let Ok((_, tail)) = CellValue::parse_prefix(rest) {
            walked += 1;
            rest = tail;
        }
        (walked, rest.is_empty())
    };
    assert_eq!(walk(&packed), (cells.len(), true));
    for cut in 1..packed.len() {
        assert_ne!(walk(&packed[..cut]), (cells.len(), true), "cut at {cut}");
    }
}
