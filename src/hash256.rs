//! Stand-alone SHA-256 used only to fingerprint offline transaction fixtures.
//!
//! This implementation has known-answer tests, but is not a production
//! cryptographic dependency or proof of block inclusion. Replace or audit it
//! independently before using transaction hashes in a trusted chain adapter.

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (idx, bytes) in block.chunks_exact(4).enumerate() {
        w[idx] = u32::from_be_bytes(bytes.try_into().expect("4-byte word"));
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

pub(crate) fn sha256(input: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let mut exact = input.chunks_exact(64);
    for block in &mut exact {
        compress(&mut state, block.try_into().expect("64-byte block"));
    }
    let rest = exact.remainder();
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let len = if rest.len() >= 56 { 128 } else { 64 };
    let bit_length = (input.len() as u64).wrapping_mul(8);
    tail[len - 8..len].copy_from_slice(&bit_length.to_be_bytes());
    for block in tail[..len].chunks_exact(64) {
        compress(&mut state, block.try_into().expect("64-byte block"));
    }
    let mut result = [0u8; 32];
    for (out, word) in result.chunks_exact_mut(4).zip(state) {
        out.copy_from_slice(&word.to_be_bytes());
    }
    result
}

/// SHA256 of logically concatenated slices, without allocating a stripped
/// SegWit transaction buffer. The pending 64-byte block survives arbitrary
/// boundaries, including empty slices and a split SHA256 padding boundary.
/// This is only research TXID fingerprinting; not a consensus hash provider.
fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut state = H0;
    let mut pending = [0u8; 64];
    let mut filled = 0usize;
    let mut total_bytes = 0u64;
    for &part in parts {
        total_bytes = total_bytes.wrapping_add(part.len() as u64);
        let mut remaining = part;
        if filled != 0 {
            let take = (64 - filled).min(remaining.len());
            pending[filled..filled + take].copy_from_slice(&remaining[..take]);
            filled += take;
            remaining = &remaining[take..];
            if filled < 64 {
                continue;
            }
            compress(&mut state, &pending);
            filled = 0;
        }
        let mut blocks = remaining.chunks_exact(64);
        for block in &mut blocks {
            compress(&mut state, block.try_into().expect("64-byte block"));
        }
        let rest = blocks.remainder();
        pending[..rest.len()].copy_from_slice(rest);
        filled = rest.len();
    }
    let mut tail = [0u8; 128];
    tail[..filled].copy_from_slice(&pending[..filled]);
    tail[filled] = 0x80;
    let padded_len = if filled >= 56 { 128 } else { 64 };
    tail[padded_len - 8..padded_len]
        .copy_from_slice(&total_bytes.wrapping_mul(8).to_be_bytes());
    for block in tail[..padded_len].chunks_exact(64) {
        compress(&mut state, block.try_into().expect("64-byte block"));
    }
    let mut result = [0u8; 32];
    for (out, word) in result.chunks_exact_mut(4).zip(state) {
        out.copy_from_slice(&word.to_be_bytes());
    }
    result
}

pub(crate) fn double_sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    sha256(&sha256_parts(parts))
}

pub(crate) fn double_sha256(input: &[u8]) -> [u8; 32] {
    sha256(&sha256(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn published_sha256_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(&vec![b'a'; 1000000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 55])),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
    }

    #[test]
    fn chunked_sha256d_matches_flat_hash_at_all_padding_boundaries() {
        for len in [
            0usize, 1, 2, 7, 54, 55, 56, 63, 64, 65, 119, 120, 127, 128,
            129, 255, 256, 257, 4095, 4096, 16384,
        ] {
            let input: Vec<u8> = (0..len)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(19))
                .collect();
            for stride in [1usize, 2, 3, 7, 55, 63, 64, 65, 127, 4096] {
                let mut parts: Vec<&[u8]> = vec![&[]];
                for chunk in input.chunks(stride) {
                    parts.push(chunk);
                    parts.push(&[]);
                }
                assert_eq!(
                    sha256_parts(&parts),
                    sha256(&input),
                    "sha256 length {len}, stride {stride}"
                );
                assert_eq!(
                    double_sha256_parts(&parts),
                    double_sha256(&input),
                    "sha256d length {len}, stride {stride}"
                );
            }
        }
        assert_eq!(sha256_parts(&[&[], &[]]), sha256(b""));
        assert_eq!(double_sha256_parts(&[&[], &[]]), double_sha256(b""));
    }
}
