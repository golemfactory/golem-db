//! Order encoding: a ladder per family, and the property this exists for —
//! `a.cmp(&b) == enc(a).cmp(&enc(b))`.

use std::borrow::Cow;
use std::cmp::Ordering;

use proptest::prelude::*;

use super::properties::{any_cell_type, any_valid_cell};
use crate::*;

const WIDTHS: [Width; 4] = [Width::W4, Width::W8, Width::W16, Width::W32];
const F32: CellType = CellType::Float(FloatWidth::F32);
const F64: CellType = CellType::Float(FloatWidth::F64);

fn signed_types() -> Vec<CellType> {
    let mut out: Vec<_> = WIDTHS
        .iter()
        .flat_map(|w| [CellType::Int(*w), CellType::Decimal(*w)])
        .collect();
    out.extend([CellType::Date32, CellType::Timestamp64]);
    out
}

fn unsigned_types() -> Vec<CellType> {
    let mut out: Vec<_> = WIDTHS
        .iter()
        .flat_map(|w| [CellType::Uint(*w), CellType::FixedBytes(*w)])
        .collect();
    out.push(CellType::Bytes20);
    out
}

fn width_of(ty: CellType) -> usize {
    let ValueLayout::Fixed(n) = ty.layout() else {
        panic!("{} is not fixed-width", ty.name());
    };
    n
}

/// `n` big-endian bytes: `first`, then `fill`, then `last`.
fn be(n: usize, first: u8, fill: u8, last: u8) -> Vec<u8> {
    let mut out = vec![fill; n];
    out[0] = first;
    out[n - 1] = last;
    out
}

/// `v` sign-extended (or truncated) to `n` big-endian bytes.
fn sext(v: i128, n: usize) -> Vec<u8> {
    let mut wide = [if v < 0 { 0xFF } else { 0 }; 32];
    wide[16..].copy_from_slice(&v.to_be_bytes());
    wide[32 - n..].to_vec()
}

fn enc(ty: CellType, value: &[u8]) -> Vec<u8> {
    order_encode(ty, value)
        .unwrap_or_else(|e| panic!("{}: {value:02x?}: {e}", ty.name()))
        .into_owned()
}

/// The encoded rungs climb strictly, and each decodes back to its value.
fn assert_ladder(ty: CellType, ladder: &[(&str, Vec<u8>)]) {
    let encoded: Vec<_> = ladder.iter().map(|(_, v)| enc(ty, v)).collect();
    for i in 1..ladder.len() {
        assert!(
            encoded[i - 1] < encoded[i],
            "{}: {} does not sort below {}",
            ty.name(),
            ladder[i - 1].0,
            ladder[i].0,
        );
    }
    for ((name, value), e) in ladder.iter().zip(&encoded) {
        let decoded = order_decode(ty, e).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(decoded.as_ref(), &value[..], "{}: {name}", ty.name());
    }
}

/// The flip at every signed type and width. At `i32` and `dec256` these are
/// also Arkiv's `index_bytes` for `Int` and `Decimal`.
#[test]
fn signed_anchors_at_every_width() {
    for ty in signed_types() {
        let n = width_of(ty);
        let rows = [
            ("MIN", be(n, 0x80, 0x00, 0x00), be(n, 0x00, 0x00, 0x00)),
            ("-1", be(n, 0xFF, 0xFF, 0xFF), be(n, 0x7F, 0xFF, 0xFF)),
            ("0", be(n, 0x00, 0x00, 0x00), be(n, 0x80, 0x00, 0x00)),
            ("MAX", be(n, 0x7F, 0xFF, 0xFF), be(n, 0xFF, 0xFF, 0xFF)),
        ];
        for (name, value, expected) in rows {
            assert_eq!(enc(ty, &value), expected, "{} {name}", ty.name());
        }
    }
}

#[test]
fn signed_ladders() {
    for ty in signed_types() {
        let n = width_of(ty);
        assert_ladder(
            ty,
            &[
                ("MIN", be(n, 0x80, 0x00, 0x00)),
                ("MIN+1", be(n, 0x80, 0x00, 0x01)),
                ("-2", sext(-2, n)),
                ("-1", sext(-1, n)),
                ("0", sext(0, n)),
                ("1", sext(1, n)),
                ("MAX-1", be(n, 0x7F, 0xFF, 0xFE)),
                ("MAX", be(n, 0x7F, 0xFF, 0xFF)),
            ],
        );
    }
}

#[test]
fn unsigned_ladders() {
    for ty in unsigned_types() {
        let n = width_of(ty);
        assert_ladder(
            ty,
            &[
                ("0", be(n, 0x00, 0x00, 0x00)),
                ("1", be(n, 0x00, 0x00, 0x01)),
                ("top bit", be(n, 0x80, 0x00, 0x00)),
                ("MAX-1", be(n, 0xFF, 0xFF, 0xFE)),
                ("MAX", be(n, 0xFF, 0xFF, 0xFF)),
            ],
        );
    }
}

#[test]
fn bool_ladder() {
    assert_ladder(
        CellType::Bool,
        &[("false", vec![0x00]), ("true", vec![0x01])],
    );
    assert_eq!(
        order_encode(CellType::Bool, &[2]),
        Err(OrderError::Invalid(CellParseError::InvalidBool(2)))
    );
}

macro_rules! float_ladder {
    ($ty:expr, $f:ty) => {{
        let rungs: [(&str, $f); 11] = [
            ("-inf", <$f>::NEG_INFINITY),
            ("MIN", <$f>::MIN),
            ("-1.0", -1.0),
            ("-MIN_POSITIVE", -<$f>::MIN_POSITIVE),
            ("-smallest subnormal", -<$f>::from_bits(1)),
            ("+0.0", 0.0),
            ("smallest subnormal", <$f>::from_bits(1)),
            ("MIN_POSITIVE", <$f>::MIN_POSITIVE),
            ("1.0", 1.0),
            ("MAX", <$f>::MAX),
            ("+inf", <$f>::INFINITY),
        ];
        let ladder: Vec<_> = rungs
            .iter()
            .map(|(name, v)| (*name, v.to_be_bytes().to_vec()))
            .collect();
        assert_ladder($ty, &ladder);
    }};
}

#[test]
fn float_ladders() {
    float_ladder!(F32, f32);
    float_ladder!(F64, f64);
}

/// Both branches of the float transform, byte for byte.
#[test]
fn f64_encoding_vectors() {
    #[rustfmt::skip]
    let rows: &[(f64, [u8; 8])] = &[
        (f64::NEG_INFINITY, [0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        (-1.0,              [0x40, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        (0.0,               [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (1.0,               [0xBF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (f64::INFINITY,     [0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ];
    for (v, expected) in rows {
        assert_eq!(enc(F64, &v.to_be_bytes()), expected, "{v}");
    }
}

/// The order forms -0.0 and NaN would have do not decode.
#[test]
fn nan_and_negative_zero_have_no_order_form() {
    let nan = Err(OrderError::Invalid(CellParseError::FloatNaN));
    let neg_zero = Err(OrderError::Invalid(CellParseError::NegativeZero));

    assert_eq!(order_encode(F64, &(-0.0f64).to_be_bytes()), neg_zero);
    assert_eq!(
        order_decode(F64, &[0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        neg_zero
    );
    assert_eq!(order_decode(F64, &[0xFF, 0xF8, 0, 0, 0, 0, 0, 0]), nan);
    assert_eq!(order_decode(F32, &[0x7F, 0xFF, 0xFF, 0xFF]), neg_zero);
}

/// Includes byte-prefixes ("Alice" < "Alice2" < "Alicia") and multi-byte UTF-8.
#[test]
fn str_ladder() {
    let ladder: Vec<_> = ["", "A", "Alice", "Alice2", "Alicia", "Bob", "a", "é", "😀"]
        .iter()
        .map(|s| (*s, s.as_bytes().to_vec()))
        .collect();
    assert_ladder(CellType::Str, &ladder);
}

#[test]
fn field_only_types_are_not_indexable() {
    #[cfg(not(feature = "custom_types"))]
    let types = [CellType::Bytes];
    #[cfg(feature = "custom_types")]
    let types = [
        CellType::Bytes,
        CellType::Custom(CustomTypeId::new(64).unwrap()),
    ];
    for ty in types {
        assert_eq!(order_encode(ty, &[]), Err(OrderError::NotIndexable(ty)));
        assert_eq!(order_decode(ty, &[]), Err(OrderError::NotIndexable(ty)));
    }
}

/// Changing a scale after genesis re-encodes every value of that type.
#[test]
fn decimal_scales_are_pinned() {
    assert_eq!(WIDTHS.map(Width::decimal_scale), [4, 6, 18, 18]);
}

/// `enc(a).cmp(enc(b))` equals `expected`, the domain comparison.
fn order_matches(
    ty: CellType,
    a: &[u8],
    b: &[u8],
    expected: Ordering,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        enc(ty, a).cmp(&enc(ty, b)),
        expected,
        "{}: {:02x?} vs {:02x?}",
        ty.name(),
        a,
        b
    );
    Ok(())
}

/// 256-bit two's complement, which Rust lacks: a negative sorts below a
/// non-negative, and same-sign values compare as unsigned bytes.
fn i256_cmp(a: &[u8; 32], b: &[u8; 32]) -> Ordering {
    let (neg_a, neg_b) = (a[0] & 0x80 != 0, b[0] & 0x80 != 0);
    neg_b.cmp(&neg_a).then_with(|| a.cmp(b))
}

fn sext32(v: i128) -> [u8; 32] {
    sext(v, 32).try_into().unwrap()
}

/// Arbitrary bytes, and sign-extended `i128`s so values near zero are hit.
fn any_i256() -> impl Strategy<Value = [u8; 32]> {
    prop_oneof![any::<[u8; 32]>(), any::<i128>().prop_map(sext32)]
}

proptest! {
    #[test]
    fn order_preserved_i32(a: i32, b: i32) {
        for ty in [CellType::Int(Width::W4), CellType::Decimal(Width::W4), CellType::Date32] {
            order_matches(ty, &a.to_be_bytes(), &b.to_be_bytes(), a.cmp(&b))?;
        }
    }

    #[test]
    fn order_preserved_i64(a: i64, b: i64) {
        for ty in [CellType::Int(Width::W8), CellType::Decimal(Width::W8), CellType::Timestamp64] {
            order_matches(ty, &a.to_be_bytes(), &b.to_be_bytes(), a.cmp(&b))?;
        }
    }

    #[test]
    fn order_preserved_i128(a: i128, b: i128) {
        for ty in [CellType::Int(Width::W16), CellType::Decimal(Width::W16)] {
            order_matches(ty, &a.to_be_bytes(), &b.to_be_bytes(), a.cmp(&b))?;
        }
    }

    #[test]
    fn order_preserved_i256(a in any_i256(), b in any_i256()) {
        for ty in [CellType::Int(Width::W32), CellType::Decimal(Width::W32)] {
            order_matches(ty, &a, &b, i256_cmp(&a, &b))?;
        }
    }

    #[test]
    fn i256_reference_agrees_with_i128(a: i128, b: i128) {
        prop_assert_eq!(i256_cmp(&sext32(a), &sext32(b)), a.cmp(&b));
    }

    #[test]
    fn order_preserved_unsigned(a: u128, b: u128) {
        let (a32, b32, a64, b64) = (a as u32, b as u32, a as u64, b as u64);
        order_matches(CellType::Uint(Width::W4), &a32.to_be_bytes(), &b32.to_be_bytes(), a32.cmp(&b32))?;
        order_matches(CellType::Uint(Width::W8), &a64.to_be_bytes(), &b64.to_be_bytes(), a64.cmp(&b64))?;
        order_matches(CellType::Uint(Width::W16), &a.to_be_bytes(), &b.to_be_bytes(), a.cmp(&b))?;
    }

    /// `total_cmp` is numeric order once NaN and -0.0 are excluded.
    #[test]
    fn order_preserved_f64(a: u64, b: u64) {
        let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
        prop_assume!(F64.validate(&a).is_ok() && F64.validate(&b).is_ok());
        order_matches(F64, &a, &b, f64::from_be_bytes(a).total_cmp(&f64::from_be_bytes(b)))?;
    }

    #[test]
    fn order_preserved_f32(a: u32, b: u32) {
        let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
        prop_assume!(F32.validate(&a).is_ok() && F32.validate(&b).is_ok());
        order_matches(F32, &a, &b, f32::from_be_bytes(a).total_cmp(&f32::from_be_bytes(b)))?;
    }

    /// `str` order is code-point order.
    #[test]
    fn order_preserved_str(a: String, b: String) {
        order_matches(CellType::Str, a.as_bytes(), b.as_bytes(), a.chars().cmp(b.chars()))?;
    }

    /// Every valid value survives encode → decode, `order_encode_into` agrees
    /// with `order_encode`, and exactly the in-order types borrow.
    #[test]
    fn round_trips((ty, value) in any_valid_cell()) {
        // Field-only types and over-long values; the ladders pin that every
        // indexable type encodes.
        let Ok(encoded) = order_encode(ty, &value) else { return Ok(()) };
        let decoded = order_decode(ty, &encoded).unwrap();
        prop_assert_eq!(decoded.as_ref(), &value[..]);

        let mut buf = vec![0xAA];
        order_encode_into(ty, &value, &mut buf).unwrap();
        prop_assert_eq!(&buf[1..], encoded.as_ref());

        let in_order = matches!(
            ty,
            CellType::Bool | CellType::Str | CellType::Bytes20 | CellType::FixedBytes(_) | CellType::Uint(_)
        );
        prop_assert_eq!(matches!(encoded, Cow::Borrowed(_)), in_order);
    }

    /// Decoding never panics, and whatever decodes re-encodes to the same bytes.
    #[test]
    fn decode_is_total_and_canonical(
        ty in any_cell_type(),
        bytes in prop::collection::vec(any::<u8>(), 0..40),
    ) {
        if let Ok(value) = order_decode(ty, &bytes) {
            prop_assert_eq!(enc(ty, &value), bytes);
        }
    }
}
