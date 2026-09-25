use crate::{MerkleError, Result};

pub(crate) fn check_width<const N: usize>() -> Result<()> {
    if (1..=32).contains(&N) {
        Ok(())
    } else {
        Err(MerkleError::InvalidPathWidth)
    }
}

pub(crate) fn nibble(path: &[u8], depth: usize) -> u8 {
    (path[depth / 2] >> (4 * (1 - depth % 2))) & 15
}

pub(crate) fn nibbles(path: &[u8]) -> Vec<u8> {
    (0..path.len() * 2).map(|i| nibble(path, i)).collect()
}

pub(crate) fn pack(nibbles: &[u8]) -> Vec<u8> {
    nibbles
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair.get(1).copied().unwrap_or(0))
        .collect()
}
