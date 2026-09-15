//! The property the stored forms exist for:
//! `a.cmp(&b) == stored(a).cmp(&stored(b))`.

use proptest::prelude::*;

use crate::*;

#[test]
fn flip_sign_vectors() {
    assert_eq!(flip_sign(i32::MIN.to_be_bytes()), [0x00, 0x00, 0x00, 0x00]);
    assert_eq!(flip_sign((-1i32).to_be_bytes()), [0x7F, 0xFF, 0xFF, 0xFF]);
    assert_eq!(flip_sign(0i32.to_be_bytes()), [0x80, 0x00, 0x00, 0x00]);
    assert_eq!(flip_sign(i32::MAX.to_be_bytes()), [0xFF, 0xFF, 0xFF, 0xFF]);

    // The same flip at 32 bytes: -1 as an `i256`/`dec256`, as Arkiv indexes it.
    let mut minus_one = [0xFF; 32];
    minus_one[0] = 0x7F;
    assert_eq!(flip_sign([0xFF; 32]), minus_one);
}

/// Every rung's stored form is valid and sorts strictly above the one before.
macro_rules! float_ladder {
    ($f:ty, $ty:expr) => {{
        let rungs: [$f; 11] = [
            <$f>::NEG_INFINITY,
            <$f>::MIN,
            -1.0,
            -<$f>::MIN_POSITIVE,
            -<$f>::from_bits(1), // smallest subnormal
            0.0,
            <$f>::from_bits(1),
            <$f>::MIN_POSITIVE,
            1.0,
            <$f>::MAX,
            <$f>::INFINITY,
        ];
        let stored = rungs.map(|v| encode_float(v.to_be_bytes()));
        for (i, s) in stored.iter().enumerate() {
            assert_eq!($ty.validate(s), Ok(()), "{:e}", rungs[i]);
            assert_eq!(decode_float(*s), rungs[i].to_be_bytes(), "{:e}", rungs[i]);
            if i > 0 {
                assert!(stored[i - 1] < *s, "{:e} !< {:e}", rungs[i - 1], rungs[i]);
            }
        }
    }};
}

#[test]
fn float_ladders() {
    float_ladder!(f32, CellType::Float(FloatWidth::F32));
    float_ladder!(f64, CellType::Float(FloatWidth::F64));
}

/// Changing a scale after genesis re-encodes every value of that type.
#[test]
fn decimal_scales_are_pinned() {
    let widths = [Width::W4, Width::W8, Width::W16, Width::W32];
    assert_eq!(widths.map(Width::decimal_scale), [4, 6, 18, 18]);
}

proptest! {
    /// Random `i128`s, truncated to each width and sign-extended to 32 bytes.
    #[test]
    fn flip_sign_preserves_order(a: i128, b: i128) {
        let (a32, b32, a64, b64) = (a as i32, b as i32, a as i64, b as i64);
        prop_assert_eq!(flip_sign(a32.to_be_bytes()).cmp(&flip_sign(b32.to_be_bytes())), a32.cmp(&b32));
        prop_assert_eq!(flip_sign(a64.to_be_bytes()).cmp(&flip_sign(b64.to_be_bytes())), a64.cmp(&b64));
        prop_assert_eq!(flip_sign(a.to_be_bytes()).cmp(&flip_sign(b.to_be_bytes())), a.cmp(&b));

        let sext = |v: i128| {
            let mut out = [if v < 0 { 0xFF } else { 0 }; 32];
            out[16..].copy_from_slice(&v.to_be_bytes());
            out
        };
        prop_assert_eq!(flip_sign(sext(a)).cmp(&flip_sign(sext(b))), a.cmp(&b));
    }

    /// Random bit patterns minus NaN and -0.0, where `total_cmp` is numeric order.
    #[test]
    fn encode_float_preserves_order(a: u32, b: u32, c: u64, d: u64) {
        let (a, b, c, d) = (f32::from_bits(a), f32::from_bits(b), f64::from_bits(c), f64::from_bits(d));
        prop_assume!(![a, b].iter().any(|x| x.is_nan() || x.to_bits() == 1 << 31));
        prop_assume!(![c, d].iter().any(|x| x.is_nan() || x.to_bits() == 1 << 63));
        prop_assert_eq!(encode_float(a.to_be_bytes()).cmp(&encode_float(b.to_be_bytes())), a.total_cmp(&b));
        prop_assert_eq!(encode_float(c.to_be_bytes()).cmp(&encode_float(d.to_be_bytes())), c.total_cmp(&d));
    }
}
