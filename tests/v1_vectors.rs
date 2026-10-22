//! Independently constructed Node fixture bytes for the proposed binary wire.
//! These are encoding tests, not evidence of accepted on-chain transactions.

use tradelayer_core::wire_v1::{parse_frame, parse_send, Send, WireV1Error};

fn hex_to_bytes(encoded: &str) -> Vec<u8> {
    assert_eq!(encoded.len() % 2, 0);
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16)
                .expect("fixture must be hex")
        })
        .collect()
}

#[test]
fn node_generated_v1_vectors() {
    let tsv = include_str!("../fixtures/v1_vectors.tsv");
    let mut count = 0;
    for (idx, line) in tsv.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        let [label, hex, expected, property, atoms, address] =
            <[&str; 6]>::try_from(fields).unwrap_or_else(|_| panic!("bad TSV row {}", idx + 1));
        let bytes = hex_to_bytes(hex);
        match expected {
            "direct" => {
                let parsed = parse_send(&bytes).expect(label);
                assert_eq!(
                    parsed,
                    Send::Direct {
                        property_id: property.parse().unwrap(),
                        amount_atoms: atoms.parse().unwrap(),
                        recipient: address,
                    },
                    "{label}"
                );
                assert_eq!(parse_frame(&bytes).unwrap().tx_type.as_u8(), 2);
            }
            "all" => {
                assert_eq!(
                    parse_send(&bytes),
                    Ok(Send::All { recipient: address }),
                    "{label}"
                );
            }
            "reject" => {
                assert!(parse_send(&bytes).is_err(), "{label}");
            }
            other => panic!("unknown fixture category {other}"),
        }
        count += 1;
    }
    assert_eq!(count, 15);
}

#[test]
fn critical_rejections_are_specific() {
    assert_eq!(
        parse_frame(&hex_to_bytes("544c520124")),
        Err(WireV1Error::InvalidTransactionType(36))
    );
    assert_eq!(
        parse_frame(b"tl2tl20;alice;5;2"),
        Err(WireV1Error::InvalidMagic)
    );
    assert_eq!(
        parse_send(&hex_to_bytes("544c5201020103626f6200")),
        Err(WireV1Error::TrailingBytes)
    );
}

#[test]
fn pseudorandom_input_never_panics() {
    let mut seed = 0x5a39_7121_4aab_2c77u64;
    for _ in 0..12_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = (seed % 110) as usize;
        let mut raw = Vec::with_capacity(len);
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            raw.push((seed >> 32) as u8);
        }
        let _ = parse_frame(&raw);
        let _ = parse_send(&raw);
    }
}
