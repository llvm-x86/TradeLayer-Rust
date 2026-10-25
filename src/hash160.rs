//! Stand-alone RIPEMD-160 + HASH160 for *offline* Bitcoin/Litecoin P2SH
//! script-hash diagnostics. The project still has ZERO external crates.
//!
//! RIPEMD-160 compression is independently checked against OpenSSL/Python
//! hashlib published messages and synthetic two-key HASH160 vectors.
//! This implementation is NOT an independently audited production crypto
//! implementation and does not establish consensus or signature validity.
use crate::hash256::sha256;

const LEFT_ORDER: [usize; 80] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 7, 4, 13, 1, 10, 6, 15, 3, 12, 0, 9, 5,
    2, 14, 11, 8, 3, 10, 14, 4, 9, 15, 8, 1, 2, 7, 0, 6, 13, 11, 5, 12, 1, 9, 11, 10, 0, 8, 12, 4,
    13, 3, 7, 15, 14, 5, 6, 2, 4, 0, 5, 9, 7, 12, 2, 10, 14, 1, 3, 8, 11, 6, 15, 13,
];
const RIGHT_ORDER: [usize; 80] = [
    5, 14, 7, 0, 9, 2, 11, 4, 13, 6, 15, 8, 1, 10, 3, 12, 6, 11, 3, 7, 0, 13, 5, 10, 14, 15, 8, 12,
    4, 9, 1, 2, 15, 5, 1, 3, 7, 14, 6, 9, 11, 8, 12, 2, 10, 0, 4, 13, 8, 6, 4, 1, 3, 11, 15, 0, 5,
    12, 2, 13, 9, 7, 10, 14, 12, 15, 10, 4, 1, 5, 8, 7, 6, 2, 13, 14, 0, 3, 9, 11,
];
const LEFT_ROT: [u32; 80] = [
    11, 14, 15, 12, 5, 8, 7, 9, 11, 13, 14, 15, 6, 7, 9, 8, 7, 6, 8, 13, 11, 9, 7, 15, 7, 12, 15,
    9, 11, 7, 13, 12, 11, 13, 6, 7, 14, 9, 13, 15, 14, 8, 13, 6, 5, 12, 7, 5, 11, 12, 14, 15, 14,
    15, 9, 8, 9, 14, 5, 6, 8, 6, 5, 12, 9, 15, 5, 11, 6, 8, 13, 12, 5, 12, 13, 14, 11, 8, 5, 6,
];
const RIGHT_ROT: [u32; 80] = [
    8, 9, 9, 11, 13, 15, 15, 5, 7, 7, 8, 11, 14, 14, 12, 6, 9, 13, 15, 7, 12, 8, 9, 11, 7, 7, 12,
    7, 6, 15, 13, 11, 9, 7, 15, 11, 8, 6, 6, 14, 12, 13, 5, 14, 13, 13, 7, 5, 15, 5, 8, 11, 14, 14,
    6, 14, 6, 9, 12, 9, 12, 5, 15, 8, 8, 5, 12, 9, 12, 5, 14, 6, 8, 13, 6, 5, 15, 13, 11, 11,
];

fn f(round: usize, x: u32, y: u32, z: u32) -> u32 {
    match round {
        0..=15 => x ^ y ^ z,
        16..=31 => (x & y) | (!x & z),
        32..=47 => (x | !y) ^ z,
        48..=63 => (x & z) | (y & !z),
        _ => x ^ (y | !z),
    }
}

fn compress(h: &mut [u32; 5], chunk: &[u8; 64]) {
    let mut words = [0u32; 16];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        words[i] = u32::from_le_bytes(word.try_into().expect("four-byte word"));
    }

    let [mut al, mut bl, mut cl, mut dl, mut el] = *h;
    let [mut ar, mut br, mut cr, mut dr, mut er] = *h;
    const LEFT_K: [u32; 5] = [0, 0x5a82_7999, 0x6ed9_eba1, 0x8f1b_bcdc, 0xa953_fd4e];
    const RIGHT_K: [u32; 5] = [0x50a2_8be6, 0x5c4d_d124, 0x6d70_3ef3, 0x7a6d_76e9, 0];
    for i in 0..80 {
        let left = al
            .wrapping_add(f(i, bl, cl, dl))
            .wrapping_add(words[LEFT_ORDER[i]])
            .wrapping_add(LEFT_K[i / 16])
            .rotate_left(LEFT_ROT[i])
            .wrapping_add(el);
        al = el;
        el = dl;
        dl = cl.rotate_left(10);
        cl = bl;
        bl = left;

        let right = ar
            .wrapping_add(f(79 - i, br, cr, dr))
            .wrapping_add(words[RIGHT_ORDER[i]])
            .wrapping_add(RIGHT_K[i / 16])
            .rotate_left(RIGHT_ROT[i])
            .wrapping_add(er);
        ar = er;
        er = dr;
        dr = cr.rotate_left(10);
        cr = br;
        br = right;
    }
    let previous = *h;
    h[0] = previous[1].wrapping_add(cl).wrapping_add(dr);
    h[1] = previous[2].wrapping_add(dl).wrapping_add(er);
    h[2] = previous[3].wrapping_add(el).wrapping_add(ar);
    h[3] = previous[4].wrapping_add(al).wrapping_add(br);
    h[4] = previous[0].wrapping_add(bl).wrapping_add(cr);
}

/// Only used by offline P2SH source audits, never as a consensus crypto API.
pub(crate) fn ripemd160(input: &[u8]) -> [u8; 20] {
    let mut h = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for chunk in input.chunks_exact(64) {
        compress(&mut h, chunk.try_into().expect("64-byte input chunk"));
    }
    let remaining = input.len() % 64;
    let extra = &input[input.len() - remaining..];
    let pad_len = if remaining >= 56 { 128 } else { 64 };
    let mut final_blocks = [0u8; 128];
    final_blocks[..remaining].copy_from_slice(extra);
    final_blocks[remaining] = 0x80;
    final_blocks[pad_len - 8..pad_len]
        .copy_from_slice(&(input.len() as u64).wrapping_mul(8).to_le_bytes());
    for chunk in final_blocks[..pad_len].chunks_exact(64) {
        compress(&mut h, chunk.try_into().expect("64-byte padded chunk"));
    }
    let mut output = [0u8; 20];
    for (dest, word) in output.chunks_exact_mut(4).zip(h) {
        dest.copy_from_slice(&word.to_le_bytes());
    }
    output
}

pub(crate) fn hash160(bytes: &[u8]) -> [u8; 20] {
    ripemd160(&sha256(bytes))
}

#[cfg(test)]
mod tests {
    use super::{hash160, ripemd160};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn independent_hashlib_ripemd160_known_answers_and_padding_edges() {
        for (data, expected) in [
            ("", "9c1185a5c5e9fc54612808977ee8f548b2258d31"),
            ("a", "0bdc9d2d256b3ee9daae347be6f4dc835a467ffe"),
            ("abc", "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"),
            ("message digest", "5d0689ef49d2fae572b881b123a85ffa21595f36"),
        ] {
            assert_eq!(hex(&ripemd160(data.as_bytes())), expected);
        }
        assert_eq!(
            hex(&ripemd160(&[b'a'; 55])),
            "0d8a8c9063a48576a7c97e9f95253a6e53ff6765"
        );
        assert_eq!(
            hex(&ripemd160(&[b'a'; 56])),
            "e72334b46c83cc70bef979e15453706c95b888be"
        );
        assert_eq!(
            hex(&ripemd160(&vec![b'a'; 1_000_000])),
            "52783243c1697bdbe16d37f97f68f08325dc1528"
        );
    }

    #[test]
    fn independent_hashlib_two_key_script_hash160_responds_to_order() {
        let x = [
            0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87,
            0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b,
            0x16, 0xf8, 0x17, 0x98,
        ];
        let mut a = [0u8; 33];
        let mut b = [0u8; 33];
        a[0] = 2;
        b[0] = 3;
        a[1..].copy_from_slice(&x);
        b[1..].copy_from_slice(&x);
        let mut redeem = vec![0x52, 0x21];
        redeem.extend_from_slice(&a);
        redeem.push(0x21);
        redeem.extend_from_slice(&b);
        redeem.extend_from_slice(&[0x52, 0xae]);
        assert_eq!(
            hex(&hash160(&redeem)),
            "3fb5b5fbb96ad2b002a5ab6f3e05da931641c0f4"
        );
        let mut swapped = vec![0x52, 0x21];
        swapped.extend_from_slice(&b);
        swapped.push(0x21);
        swapped.extend_from_slice(&a);
        swapped.extend_from_slice(&[0x52, 0xae]);
        assert_eq!(
            hex(&hash160(&swapped)),
            "62c5a9dce49968f5ebf6691fb6fa43e1ebede59d"
        );
    }
}
