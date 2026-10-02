use proptest::prelude::*;

use crate::*;

#[test]
fn cell_batch_keeps_last_operation_and_reports_changes_in_key_order() {
    use golemdb_merkle::{Keccak256Hasher, RootRef};
    use golemdb_storage::{Database, MemoryDatabase};

    let db = MemoryDatabase::new();
    let cells = Cells::new(&Keccak256Hasher);
    let mut tx = db.begin_write().unwrap();
    let key = |id| CellKey::new(id, CellNameRef::raw(b"value"));
    let value = |byte| CellValue::parse(vec![0x01, byte]).unwrap();
    let put = |id, byte| CellChange::Put {
        key: key(id),
        value: value(byte),
    };
    let delete = |id| CellChange::Delete { key: key(id) };
    let initial = cells
        .apply(&mut tx, RootRef::Empty, [put(2, 0), put(3, 1)])
        .unwrap();
    let update = cells
        .apply(
            &mut tx,
            initial.root,
            [
                put(3, 0),
                put(1, 1),
                delete(2),
                put(4, 1),
                delete(1),
                put(2, 1),
                delete(3),
                delete(4),
                put(1, 0),
                put(2, 0),
                delete(3),
            ],
        )
        .unwrap();

    assert_eq!(
        update.changed_cells,
        vec![
            CellValueChange {
                key: key(1),
                before: None,
                after: Some(value(0))
            },
            CellValueChange {
                key: key(3),
                before: Some(value(1)),
                after: None
            },
        ]
    );
    for (id, expected) in [
        (1, Some(value(0))),
        (2, Some(value(0))),
        (3, None),
        (4, None),
    ] {
        assert_eq!(cells.get(&tx, &key(id)).unwrap(), expected);
    }
    let fresh_db = MemoryDatabase::new();
    let mut fresh_tx = fresh_db.begin_write().unwrap();
    let fresh = cells
        .apply(&mut fresh_tx, RootRef::Empty, [put(1, 0), put(2, 0)])
        .unwrap();
    assert_eq!(update.root, fresh.root);
}

#[test]
fn owned_values_preserve_vectors_without_copying_storage_buffers() {
    for (name, bytes, ..) in super::vectors::VECTORS {
        let buffer = bytes.to_vec();
        let pointer = buffer.as_ptr();
        let owned = CellValue::parse(buffer).unwrap();
        assert_eq!(owned.encoded_bytes().as_ptr(), pointer, "{name}");
        assert_eq!(owned.metadata(), bytes[0], "{name}");
        assert_eq!(owned.is_indexable(), bytes[0] & 0x80 != 0, "{name}");
        assert_eq!(owned.cell_type().id(), bytes[0] & 0x7f, "{name}");
        assert_eq!(owned.value(), &bytes[1..], "{name}");
        assert_eq!(
            owned.as_bytes(),
            (bytes[0] & 0x7f == CellType::Bytes.id()).then_some(&bytes[1..]),
            "{name}"
        );
        assert_eq!(
            owned.as_view(),
            CellValueRef::parse(bytes).unwrap(),
            "{name}"
        );
        assert_eq!(
            owned.as_view().value().as_ptr(),
            owned.encoded_bytes()[1..].as_ptr()
        );
        assert_eq!(CellValue::from(owned.as_view()), owned);
        let buffer = owned.into_bytes();
        assert_eq!(buffer.as_ptr(), pointer, "{name}");
        assert_eq!(buffer, *bytes);
    }
    for (name, bytes, error) in super::vectors::BAD_VECTORS {
        assert_eq!(
            CellValue::parse(bytes.to_vec()).err(),
            Some(*error),
            "{name}"
        );
    }
}

#[test]
fn owned_value_outlives_source_and_moves_into_a_response() {
    let owned = {
        let source = String::from("hello");
        CellValue::from(CellValueRef::new(CellType::Str, source.as_bytes(), false).unwrap())
    };
    let response = [owned];
    assert_eq!(response[0].as_str(), Some("hello"));
}

#[test]
fn variable_values_have_no_length_or_implicit_boundary() {
    assert_eq!(CellValueRef::parse(&[2]).unwrap().as_str(), Some(""));
    assert_eq!(
        CellValueRef::parse(&[3]).unwrap().as_bytes(),
        Some(&b""[..])
    );
    let bytes = [3, 0, 0, 0, 1, 255, 0, 128];
    assert_eq!(CellValueRef::parse(&bytes).unwrap().value(), &bytes[1..]);
    assert_eq!(
        CellValueRef::new(CellType::Str, b"hi", true)
            .unwrap()
            .encode(),
        b"\x82hi"
    );
    // There is no sniffing for the old framed format: every byte is payload.
    assert_eq!(
        CellValueRef::parse(b"\x02\0\0\0\x02hi").unwrap().value(),
        b"\0\0\0\x02hi"
    );
}

#[test]
fn limits_are_independent_and_count_payload_bytes() {
    let limits = CellLimits {
        max_cell_name_len: 6,
        max_str_len: 2,
        max_bytes_len: 4,
    };
    assert!(limits.parse_user_name(b"$owner").is_ok());
    assert_eq!(
        limits.parse_user_name(b"$owners"),
        Err(CellNameError::TooLong { max: 6, actual: 7 })
    );
    for (ty, max) in [(CellType::Str, 2), (CellType::Bytes, 4)] {
        for len in [0, max, max + 1] {
            let value = vec![b'a'; len];
            let cell = CellValueRef::new(ty, &value, false).unwrap();
            let expected = if len <= max {
                Ok(())
            } else {
                Err(CellLimitError {
                    ty,
                    max: max as u32,
                    actual: len,
                })
            };
            assert_eq!(limits.validate_value(cell), expected);
        }
    }
    assert!(
        limits
            .validate_value(CellValueRef::new(CellType::Str, "é".as_bytes(), true).unwrap())
            .is_ok()
    );
    assert!(
        limits
            .validate_value(CellValueRef::new(CellType::Str, "éa".as_bytes(), false).unwrap())
            .is_err()
    );
    let zero = CellLimits {
        max_cell_name_len: 0,
        max_str_len: 0,
        max_bytes_len: 0,
    };
    assert!(zero.parse_user_name(b"a").is_err());
    assert!(
        zero.validate_value(CellValueRef::parse(&[2]).unwrap())
            .is_ok()
    );
    assert!(
        zero.validate_value(CellValueRef::parse(&[3]).unwrap())
            .is_ok()
    );
    assert!(
        zero.validate_value(CellValueRef::parse(&[1, 1]).unwrap())
            .is_ok()
    );
}

#[test]
fn codec_does_not_impose_the_old_u16_cap() {
    let payload = vec![b'a'; 65536];
    for ty in [CellType::Str, CellType::Bytes] {
        let cell = CellValueRef::new(ty, &payload, false).unwrap();
        let bytes = cell.encode();
        assert_eq!(bytes.len(), payload.len() + 1);
        assert_eq!(CellValueRef::parse(&bytes).unwrap(), cell);
        let limits = CellLimits {
            max_cell_name_len: 64,
            max_str_len: 65536,
            max_bytes_len: 65536,
        };
        assert!(limits.validate_value(cell).is_ok());
    }
}

#[test]
fn full_keys_encode_record_id_then_raw_name() {
    let key = CellKey::new(42, CellNameRef::parse_user(b"Price", 64).unwrap());
    assert_eq!(key.encode(), b"\0\0\0\0\0\0\0\x2aPrice");
    assert_eq!(key.record_id(), 42);
    assert_eq!(key.name().as_bytes(), b"Price");
    assert_eq!(CellKey::decode(&key.encode()).unwrap(), key);
    for name in [&b""[..], b"#key", b"@weight", b"\0\xff\x80"] {
        let key = CellKey::new(u64::MAX, CellNameRef::raw(name));
        assert_eq!(CellKey::decode(&key.encode()).unwrap(), key);
    }
    for len in 0..8 {
        assert_eq!(
            CellKey::decode(&[0; 8][..len]),
            Err(CellKeyError::MissingRecordId { actual: len })
        );
    }
}

#[test]
fn float_input_normalizes_zero_but_decoding_rejects_noncanonical_zero() {
    assert_eq!(
        encode_float((-0.0f32).to_be_bytes()),
        encode_float(0.0f32.to_be_bytes())
    );
    assert_eq!(
        encode_float((-0.0f64).to_be_bytes()),
        encode_float(0.0f64.to_be_bytes())
    );
    assert_eq!(
        CellValueRef::new(
            CellType::Float(FloatWidth::F32),
            &encode_float((-0.0f32).to_be_bytes()),
            false
        )
        .unwrap()
        .as_f32(),
        Some(0.0)
    );
    assert_eq!(
        CellValueRef::parse(&[0x18, 0x7f, 0xff, 0xff, 0xff]),
        Err(CellParseError::NegativeZero)
    );
    assert_eq!(
        CellValueRef::new(
            CellType::Float(FloatWidth::F64),
            &encode_float(f64::NAN.to_be_bytes()),
            false
        ),
        Err(CellParseError::FloatNaN)
    );
}

proptest! {
    #[test]
    fn key_order_matches_storage_bytes(
        a in any::<u64>(), b in any::<u64>(),
        x in prop::collection::vec(any::<u8>(), 0..80),
        y in prop::collection::vec(any::<u8>(), 0..80),
    ) {
        let a = CellKey::new(a, CellNameRef::raw(&x));
        let b = CellKey::new(b, CellNameRef::raw(&y));
        prop_assert_eq!(a.cmp(&b), a.encode().cmp(&b.encode()));
        prop_assert_eq!(CellKey::decode(&a.encode()).unwrap(), a);
    }
}
