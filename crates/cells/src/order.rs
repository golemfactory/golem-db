//! [`order_encode`] and [`order_decode`] — value bytes laid out so that
//! bytewise order is value order. See the crate docs, "Order encoding".

use std::borrow::Cow;

use crate::error::OrderError;
use crate::types::CellType;

enum Transform {
    Identity,
    SignFlip,
    Float,
}

fn transform(ty: CellType) -> Result<Transform, OrderError> {
    match ty {
        CellType::Bool
        | CellType::Str
        | CellType::Bytes20
        | CellType::FixedBytes(_)
        | CellType::Uint(_) => Ok(Transform::Identity),
        CellType::Int(_) | CellType::Decimal(_) | CellType::Date32 | CellType::Timestamp64 => {
            Ok(Transform::SignFlip)
        }
        CellType::Float(_) => Ok(Transform::Float),
        CellType::Bytes => Err(OrderError::NotIndexable(ty)),
        #[cfg(feature = "custom_types")]
        CellType::Custom(_) => Err(OrderError::NotIndexable(ty)),
    }
}

/// Encode a value body of `ty` into its order form, borrowing when the bytes
/// are already in order. The value is validated first.
pub fn order_encode(ty: CellType, value: &[u8]) -> Result<Cow<'_, [u8]>, OrderError> {
    let t = transform(ty)?;
    ty.validate(value)?;
    if let Transform::Identity = t {
        return Ok(Cow::Borrowed(value));
    }
    let mut out = value.to_vec();
    apply(t, &mut out, true);
    Ok(Cow::Owned(out))
}

/// [`order_encode`], appended to `out`.
pub fn order_encode_into(ty: CellType, value: &[u8], out: &mut Vec<u8>) -> Result<(), OrderError> {
    let t = transform(ty)?;
    ty.validate(value)?;
    let start = out.len();
    out.extend_from_slice(value);
    apply(t, &mut out[start..], true);
    Ok(())
}

/// Decode an order form back into a value body of `ty`. The result is
/// validated, so bytes no valid value encodes to are rejected.
pub fn order_decode(ty: CellType, encoded: &[u8]) -> Result<Cow<'_, [u8]>, OrderError> {
    let t = transform(ty)?;
    let value = if let Transform::Identity = t {
        Cow::Borrowed(encoded)
    } else {
        let mut out = encoded.to_vec();
        apply(t, &mut out, false);
        Cow::Owned(out)
    };
    ty.validate(&value)?;
    Ok(value)
}

/// Big-endian puts the sign bit in byte 0 at every width, so no transform
/// needs the width. An empty slice is left alone: `order_decode` transforms
/// before it validates the length.
fn apply(t: Transform, bytes: &mut [u8], encode: bool) {
    let Some(&first) = bytes.first() else {
        return;
    };
    match t {
        Transform::Identity => {}
        Transform::SignFlip => bytes[0] ^= 0x80,
        Transform::Float => {
            // A negative float's bits grow with its magnitude, so it is
            // inverted whole. Stored, a negative has its top bit set;
            // encoded, it has it clear.
            let negative = (first & 0x80 != 0) == encode;
            if negative {
                bytes.iter_mut().for_each(|b| *b = !*b);
            } else {
                bytes[0] ^= 0x80;
            }
        }
    }
}
