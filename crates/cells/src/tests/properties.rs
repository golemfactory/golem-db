//! Properties over generated input: arbitrary content, and lengths past
//! [`MAX_VALUE_LEN`].

use proptest::prelude::*;

use crate::*;

/// A type with a value valid for it. `str`/`bytes` lengths straddle
/// [`MAX_VALUE_LEN`], so `TooLong` is the one error building may hit.
fn any_valid_cell() -> impl Strategy<Value = (CellType, Vec<u8>)> {
    (0u8..128)
        .prop_filter_map("reserved id", |id| CellType::from_id(id).ok())
        .prop_flat_map(|ty| {
            let value = match (ty, ty.width()) {
                (CellType::Bool, _) => prop::collection::vec(0u8..=1, 1).boxed(),
                (CellType::Str, _) => prop::string::string_regex(".{0,120}")
                    .unwrap()
                    .prop_map(String::into_bytes)
                    .boxed(),
                // Only floats reject some bit patterns: NaN and -0.0.
                (_, Some(n)) => prop::collection::vec(any::<u8>(), n)
                    .prop_filter("NaN or -0.0", move |v| ty.validate(v).is_ok())
                    .boxed(),
                (_, None) => prop::collection::vec(any::<u8>(), 0..=MAX_VALUE_LEN + 8).boxed(),
            };
            (Just(ty), value)
        })
}

proptest! {
    /// Parsing arbitrary bytes never panics, and whatever parses re-encodes
    /// to the bytes it came from.
    #[test]
    fn parse_is_total_and_encode_inverts_it(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        if let Ok(cell) = CellValue::parse(&bytes) {
            prop_assert_eq!(cell.encode(), bytes);
        }
    }

    #[test]
    fn build_encode_parse_round_trips((ty, value) in any_valid_cell(), indexable: bool) {
        let indexable = indexable && ty != CellType::Bytes;
        let built = match CellValue::new(ty, &value, indexable) {
            Ok(cell) => cell,
            Err(e) => {
                prop_assert_eq!(e, CellParseError::TooLong { actual: value.len() });
                return Ok(());
            }
        };
        let encoded = built.encode();
        prop_assert_eq!(CellValue::parse(&encoded), Ok(built));
    }

    /// A `str` cell parses exactly when its bytes are UTF-8.
    #[test]
    fn str_accepts_exactly_utf8(value in prop::collection::vec(any::<u8>(), 0..=MAX_VALUE_LEN)) {
        let cell = [&[CellType::Str.id()][..], &(value.len() as u32).to_be_bytes(), &value].concat();
        prop_assert_eq!(CellValue::parse(&cell).is_ok(), core::str::from_utf8(&value).is_ok());
    }
}
