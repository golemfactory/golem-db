//! Stored-form ordering: a ladder per family, and the property this exists
//! for — `a.cmp(&b) == stored(a).cmp(&stored(b))`.

use std::cmp::Ordering;

use proptest::prelude::*;

use crate::order::{flip_sign, float_from_stored, float_to_stored};
use crate::*;

const WIDTHS: [Width; 4] = [Width::W4, Width::W8, Width::W16, Width::W32];
const F32: CellType = CellType::Float(FloatWidth::F32);
const F64: CellType = CellType::Float(FloatWidth::F64);

fn is_signed(ty: CellType) -> bool {
    matches!(
        ty,
        CellType::Int(_) | CellType::Decimal(_) | CellType::Date32 | CellType::Timestamp64
    )
}

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

/// A value's natural big-endian bytes — two's complement, IEEE-754, or
/// already in order — in stored form.
fn stored(ty: CellType, natural: &[u8]) -> Vec<u8> {
    let mut out = natural.to_vec();
    if is_signed(ty) {
        flip_sign(&mut out);
    } else if let CellType::Float(_) = ty {
        float_to_stored(&mut out);
    }
    out
}

/// The inverse of [`stored`].
fn natural(ty: CellType, stored: &[u8]) -> Vec<u8> {
    let mut out = stored.to_vec();
    if is_signed(ty) {
        flip_sign(&mut out);
    } else if let CellType::Float(_) = ty {
        float_from_stored(&mut out);
    }
    out
}

/// Each rung's stored form is valid, decodes back, and sorts strictly above
/// the rung before it.
fn assert_ladder(ty: CellType, ladder: &[(&str, Vec<u8>)]) {
    let encoded: Vec<_> = ladder.iter().map(|(_, v)| stored(ty, v)).collect();
    for ((name, value), e) in ladder.iter().zip(&encoded) {
        ty.validate(e)
            .unwrap_or_else(|err| panic!("{} {name}: {err}", ty.name()));
        assert_eq!(&natural(ty, e), value, "{} {name}", ty.name());
    }
    for i in 1..ladder.len() {
        assert!(
            encoded[i - 1] < encoded[i],
            "{}: {} does not sort below {}",
            ty.name(),
            ladder[i - 1].0,
            ladder[i].0,
        );
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
            assert_eq!(stored(ty, &value), expected, "{} {name}", ty.name());
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
fn f64_stored_vectors() {
    #[rustfmt::skip]
    let rows: &[(f64, [u8; 8])] = &[
        (f64::NEG_INFINITY, [0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        (-1.0,              [0x40, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
        (0.0,               [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (1.0,               [0xBF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (f64::INFINITY,     [0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ];
    for (v, expected) in rows {
        assert_eq!(stored(F64, &v.to_be_bytes()), expected, "{v}");
        assert_eq!(encode_float(v.to_be_bytes()), *expected, "{v}");
    }
}

/// Whatever stored bytes -0.0 and NaN would take are not valid values.
#[test]
fn nan_and_negative_zero_are_not_stored() {
    let neg_zero = Err(CellParseError::NegativeZero);
    assert_eq!(
        F64.validate(&encode_float((-0.0f64).to_be_bytes())),
        neg_zero
    );
    assert_eq!(
        F32.validate(&encode_float((-0.0f32).to_be_bytes())),
        neg_zero
    );
    for bits in [
        f64::NAN.to_bits(),
        (-f64::NAN).to_bits(),
        0x7FF0_0000_0000_0001,
    ] {
        assert_eq!(
            F64.validate(&encode_float(bits.to_be_bytes())),
            Err(CellParseError::FloatNaN),
            "{bits:#x}"
        );
    }
}

/// Includes byte-prefixes ("Alice" < "Alice2" < "Alicia") and multi-byte
/// UTF-8. `str` is the trailing field of an index term, so it needs no
/// terminator.
#[test]
fn str_ladder() {
    let ladder: Vec<_> = ["", "A", "Alice", "Alice2", "Alicia", "Bob", "a", "é", "😀"]
        .iter()
        .map(|s| (*s, s.as_bytes().to_vec()))
        .collect();
    assert_ladder(CellType::Str, &ladder);
}

/// Changing a scale after genesis re-encodes every value of that type.
#[test]
fn decimal_scales_are_pinned() {
    assert_eq!(WIDTHS.map(Width::decimal_scale), [4, 6, 18, 18]);
}

/// Stored bytes compare as `expected`, the domain comparison.
fn order_matches(
    ty: CellType,
    a: &[u8],
    b: &[u8],
    expected: Ordering,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        stored(ty, a).cmp(&stored(ty, b)),
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

/// An indexable cell of `ty` over stored bytes.
fn cell(ty: CellType, value: &[u8]) -> CellValue<'_> {
    CellValue::new(ty, value, true).unwrap()
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
        let (a, b) = (f64::from_bits(a), f64::from_bits(b));
        prop_assume!(!a.is_nan() && !b.is_nan() && a.to_bits() != 1 << 63 && b.to_bits() != 1 << 63);
        order_matches(F64, &a.to_be_bytes(), &b.to_be_bytes(), a.total_cmp(&b))?;
    }

    #[test]
    fn order_preserved_f32(a: u32, b: u32) {
        let (a, b) = (f32::from_bits(a), f32::from_bits(b));
        prop_assume!(!a.is_nan() && !b.is_nan() && a.to_bits() != 1 << 31 && b.to_bits() != 1 << 31);
        order_matches(F32, &a.to_be_bytes(), &b.to_be_bytes(), a.total_cmp(&b))?;
    }

    /// `str` order is code-point order.
    #[test]
    fn order_preserved_str(a: String, b: String) {
        order_matches(CellType::Str, a.as_bytes(), b.as_bytes(), a.chars().cmp(b.chars()))?;
    }

    /// `encode_int` → cell → accessor gives back the value, at every signed type.
    #[test]
    fn int_accessors_round_trip(a: i32, b: i64, c: i128, d in any_i256()) {
        let a4 = encode_int(a.to_be_bytes());
        prop_assert_eq!(cell(CellType::Int(Width::W4), &a4).as_i32(), Some(a));
        prop_assert_eq!(cell(CellType::Decimal(Width::W4), &a4).as_dec32_unscaled(), Some(a));
        prop_assert_eq!(cell(CellType::Date32, &a4).as_date32(), Some(a));

        let b8 = encode_int(b.to_be_bytes());
        prop_assert_eq!(cell(CellType::Int(Width::W8), &b8).as_i64(), Some(b));
        prop_assert_eq!(cell(CellType::Decimal(Width::W8), &b8).as_dec64_unscaled(), Some(b));
        prop_assert_eq!(cell(CellType::Timestamp64, &b8).as_timestamp64(), Some(b));

        let c16 = encode_int(c.to_be_bytes());
        prop_assert_eq!(cell(CellType::Int(Width::W16), &c16).as_i128(), Some(c));
        prop_assert_eq!(cell(CellType::Decimal(Width::W16), &c16).as_dec128_unscaled(), Some(c));

        let d32 = encode_int(d);
        prop_assert_eq!(cell(CellType::Int(Width::W32), &d32).as_i256_be(), Some(d));
        prop_assert_eq!(cell(CellType::Decimal(Width::W32), &d32).as_dec256_unscaled_be(), Some(d));
    }

    /// `encode_float` → cell → accessor gives back the same bits.
    #[test]
    fn float_accessors_round_trip(a: u32, b: u64) {
        let (a, b) = (f32::from_bits(a), f64::from_bits(b));
        prop_assume!(!a.is_nan() && !b.is_nan() && a.to_bits() != 1 << 31 && b.to_bits() != 1 << 63);
        let a4 = encode_float(a.to_be_bytes());
        let b8 = encode_float(b.to_be_bytes());
        prop_assert_eq!(cell(F32, &a4).as_f32().map(f32::to_bits), Some(a.to_bits()));
        prop_assert_eq!(cell(F64, &b8).as_f64().map(f64::to_bits), Some(b.to_bits()));
    }
}
