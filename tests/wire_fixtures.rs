//! Check fixed legacy wire-format vectors from the companion Node script.
use tradelayer_core::wire::{
    parse_frame, parse_typed, ParsedTransaction, WireError, ATOMS_PER_TOKEN,
};

#[test]
fn source_derived_wire_fixtures() {
    let fixtures = include_str!("../fixtures/wire_vectors.tsv");
    let mut count = 0;
    for (line_number, line) in fixtures.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 5, "bad TSV at line {}", line_number + 1);
        let [label, raw, kind, body, expected] =
            <[&str; 5]>::try_from(fields).expect("five TSV fields");
        let number: u8 = kind.parse().expect("transaction type is numeric");
        let frame = parse_frame(raw).expect(label);
        assert_eq!(frame.tx_type.as_u8(), number, "type: {label}");
        assert_eq!(frame.body, body, "body: {label}");
        match expected {
            "ok" => assert!(parse_typed(raw).is_ok(), "typed: {label}"),
            "reject" => assert!(parse_typed(raw).is_err(), "typed: {label}"),
            "unsupported" => assert_eq!(
                parse_typed(raw),
                Err(WireError::UnsupportedType(number)),
                "typed: {label}"
            ),
            _ => panic!("unknown fixture expectation: {expected}"),
        }
        count += 1;
    }
    assert!(
        count >= 10,
        "expected enough fixtures to cover all wire cases"
    );
}

#[test]
fn representative_transaction_values() {
    match parse_typed("tl00;2;11;12,73").unwrap() {
        ParsedTransaction::Activate {
            types,
            code_hash_base36,
        } => {
            assert_eq!(
                types.iter().map(|t| t.as_u8()).collect::<Vec<_>>(),
                [0, 2, 11, 12]
            );
            assert_eq!(code_hash_base36, "73");
        }
        other => panic!("expected activation, got {other:?}"),
    }
    match parse_typed("tl1a,FOO,,1,,0,0,").unwrap() {
        ParsedTransaction::Issue {
            initial_tokens,
            ticker,
            managed,
            ..
        } => {
            assert_eq!(initial_tokens, 10);
            assert_eq!(ticker, "FOO");
            assert!(managed);
        }
        other => panic!("expected issue, got {other:?}"),
    }
    match parse_typed("tl20;alice;5;2").unwrap() {
        ParsedTransaction::Send {
            property_id,
            amount_atoms,
            ..
        } => {
            assert_eq!(property_id, 5);
            assert_eq!(amount_atoms, 2 * ATOMS_PER_TOKEN);
        }
        other => panic!("expected send, got {other:?}"),
    }
    match parse_typed("tlb5,89oqgw,bob,,,,,").unwrap() {
        ParsedTransaction::Grant {
            property_id,
            amount_atoms,
            ..
        } => {
            assert_eq!(property_id, 5);
            assert_eq!(amount_atoms, 5 * ATOMS_PER_TOKEN);
        }
        other => panic!("expected grant, got {other:?}"),
    }
}

#[test]
fn reference_encoder_missing_header_is_rejected() {
    // Encode.encodeSend({sendAll:true}) emits this without `tl2`.
    assert_eq!(parse_frame("1;alice"), Err(WireError::InvalidMarker));
}

#[test]
fn short_ascii_fuzz_never_panics() {
    let alphabet = b"tl0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ,;~|-_!";
    let mut state = 0x7351_294d_51ad_c399u64;
    for _ in 0..5000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = (state % 110) as usize;
        let mut input = String::with_capacity(len);
        for _ in 0..len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            input.push(char::from(alphabet[(state as usize) % alphabet.len()]));
        }
        let _ = parse_typed(&input);
    }
}
