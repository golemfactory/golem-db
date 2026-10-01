use crate::*;

#[test]
fn record_kind_helpers_encode_explicit_types_and_insert_preserves_kind() {
    let indexed = CellValue::from_str("kept")
        .with_kind(CellKind::Attribute)
        .unwrap();
    let input = RecordInput::new()
        .attribute("price", CellValue::from_i32(50))
        .unwrap()
        .field("description", indexed.clone())
        .unwrap()
        .insert("tag", indexed.clone())
        .unwrap()
        .field("payload", CellValue::from_bytes(&[0, 255]))
        .unwrap();
    let cells = input.into_cells();
    assert_eq!(
        cells[b"price".as_slice()].encoded_bytes(),
        &[0x90, 0x80, 0, 0, 50]
    );
    assert_eq!(cells[b"description".as_slice()].kind(), CellKind::Field);
    assert_eq!(cells[b"tag".as_slice()], indexed);
    assert_eq!(
        cells[b"payload".as_slice()].as_bytes(),
        Some([0, 255].as_slice())
    );
    assert!(matches!(
        RecordInput::new().attribute("payload", CellValue::from_bytes(b"")),
        Err(ApiError::InvalidArgument { .. })
    ));
}

#[test]
fn patch_supports_kind_overrides_preserved_sets_and_removals() {
    let indexed = CellValue::from_i32(1)
        .with_kind(CellKind::Attribute)
        .unwrap();
    let patch = PatchInput::new()
        .attribute("price", CellValue::from_i32(75))
        .unwrap()
        .field("plain", indexed.clone())
        .unwrap()
        .set("preserve", indexed.clone())
        .unwrap()
        .remove("description")
        .unwrap()
        .into_patch();
    assert_eq!(
        patch[b"price".as_slice()],
        CellPatch::Set(
            CellValue::from_i32(75)
                .with_kind(CellKind::Attribute)
                .unwrap()
        )
    );
    assert_eq!(
        patch[b"plain".as_slice()],
        CellPatch::Set(CellValue::from_i32(1))
    );
    assert_eq!(patch[b"preserve".as_slice()], CellPatch::Set(indexed));
    assert_eq!(patch[b"description".as_slice()], CellPatch::Remove);
    assert!(matches!(
        PatchInput::new().attribute("payload", CellValue::from_bytes(b"")),
        Err(ApiError::InvalidArgument { .. })
    ));
}

#[test]
fn duplicate_names_fail_across_helper_kinds_and_patch_operations() {
    let input = RecordInput::new()
        .field("price", CellValue::from_i32(1))
        .unwrap();
    assert!(matches!(
        input.clone().insert("price", CellValue::from_i32(2)),
        Err(ApiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        input.clone().attribute("price", CellValue::from_i32(2)),
        Err(ApiError::InvalidArgument { .. })
    ));
    assert!(matches!(
        input.field("price", CellValue::from_i32(2)),
        Err(ApiError::InvalidArgument { .. })
    ));
    for patch in [
        PatchInput::new()
            .set("price", CellValue::from_i32(1))
            .unwrap(),
        PatchInput::new().remove("price").unwrap(),
    ] {
        assert!(matches!(
            patch.clone().remove("price"),
            Err(ApiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            patch.set("price", CellValue::from_i32(2)),
            Err(ApiError::InvalidArgument { .. })
        ));
    }
    assert!(
        RecordInput::new()
            .field("Price", CellValue::from_i32(1))
            .unwrap()
            .field("price", CellValue::from_i32(2))
            .is_ok()
    );
}

#[test]
fn builders_validate_grammar_but_leave_deployment_limits_and_record_shape_to_crud() {
    for invalid in ["", "#key", "@admin", "$", "1name", "a\0b", "a b", "é"] {
        assert!(matches!(
            RecordInput::new().field(invalid, CellValue::from_bool(true)),
            Err(ApiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            PatchInput::new().remove(invalid),
            Err(ApiError::InvalidArgument { .. })
        ));
        // Pre-built maps must not bypass syntax checks through raw CellName.
        let name: CellName = CellNameRef::raw(invalid.as_bytes()).into();
        assert!(
            RecordInput::try_from(RecordCells::from([(
                name.clone(),
                CellValue::from_bool(true)
            )]))
            .is_err()
        );
        assert!(PatchInput::try_from(RecordPatch::from([(name, CellPatch::Remove)])).is_err());
    }
    assert!(
        RecordInput::new()
            .field("$owner", CellValue::from_u64(1))
            .is_ok()
    );
    assert!(
        RecordInput::new()
            .field(&"a".repeat(1024), CellValue::from_str(&"v".repeat(2048)))
            .is_ok()
    );
    assert!(RecordInput::new().as_cells().is_empty());
    assert!(PatchInput::new().as_patch().is_empty());
    let cells = RecordInput::new()
        .attribute("price", CellValue::from_i32(50))
        .unwrap()
        .into_cells();
    assert_eq!(
        RecordInput::try_from(cells.clone()).unwrap().as_cells(),
        &cells
    );
    let patch = PatchInput::new().remove("price").unwrap().into_patch();
    assert_eq!(
        PatchInput::try_from(patch.clone()).unwrap().as_patch(),
        &patch
    );
}

#[test]
fn projections_preserve_reserved_binary_names_and_distinguish_empty_from_all() {
    assert_eq!(Projection::default().as_names(), None);
    let empty = Projection::only(std::iter::empty::<&str>());
    assert_eq!(empty.as_names(), Some([].as_slice()));
    let raw = [0, 255, 0, 1];
    let projection = Projection::only([b"#key".as_slice(), b"price", &raw, b"price", b"@version"]);
    let names = projection.as_names().unwrap();
    assert_eq!(names.len(), 4);
    assert_eq!(names[0].as_bytes(), &raw);
    assert_eq!(names[1].as_bytes(), b"#key");
    assert_eq!(names[2].as_bytes(), b"@version");
    assert_eq!(names[3].as_bytes(), b"price");
}
