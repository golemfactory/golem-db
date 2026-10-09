pub(crate) fn nibble(path: &[u8], depth: usize) -> u8 {
    (path[depth / 2] >> (4 * (1 - depth % 2))) & 15
}

/// How many leading `nibbles` `path` repeats from nibble `depth` onwards.
pub(crate) fn matched(nibbles: &[u8], path: &[u8], depth: usize) -> usize {
    (nibbles.iter().enumerate())
        .take_while(|&(i, n)| nibble(path, depth + i) == *n)
        .count()
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
