//! Fixture-driven checks for `payload_builders`: every row of the JS parity
//! vector file, round trips through the crate's existing legacy decoders, and
//! the documented JS divergences.
use tradelayer_core::legacy_txdecoder_audit::{
    source_decode_body_unverified, LegacyDecoderError, SourceChannelLocation, SourcePropertyId,
    UnverifiedSourceBody,
};
use tradelayer_core::payload_builders::{
    build_attestation, build_commit_to_channel, build_simple_send, parse_atomic_amount,
    ChannelLocation, ClearLists, CommitToChannel, PayloadError, ATOMS_PER_TOKEN,
    MAX_LEGACY_SAFE_ATOMS,
};
use tradelayer_core::wire::{parse_base36, parse_typed, ParsedTransaction};

const FIXTURE: &str = include_str!("../fixtures/payload_builder_js_vectors.tsv");
const EXPECTED_ROWS: usize = 44;
const EXPECTED_BYTE_IDENTICAL: usize = 18;

struct Row<'a> {
    id: &'a str,
    family: &'a str,
    js_payload: &'a str,
    rust_outcome: &'a str,
    parity: &'a str,
}

fn rows() -> Vec<Row<'static>> {
    FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells.len(), 9, "malformed fixture row: {line}");
            Row {
                id: cells[0],
                family: cells[1],
                js_payload: cells[4],
                rust_outcome: cells[6],
                parity: cells[7],
            }
        })
        .collect()
}

fn error_name(error: &PayloadError) -> &'static str {
    match error {
        PayloadError::EmptyField(_) => "EmptyField",
        PayloadError::FieldTooLong { .. } => "FieldTooLong",
        PayloadError::ForbiddenByte { .. } => "ForbiddenByte",
        PayloadError::ReservedReferencePrefix => "ReservedReferencePrefix",
        PayloadError::ChannelAddressRequiresReference => "ChannelAddressRequiresReference",
        PayloadError::MetadataNotUppercaseStable => "MetadataNotUppercaseStable",
        PayloadError::AmbiguousFramedBody => "AmbiguousFramedBody",
        PayloadError::PayloadTooLong => "PayloadTooLong",
        PayloadError::InvalidAmount => "InvalidAmount",
        PayloadError::NonIntegerAmount => "NonIntegerAmount",
        PayloadError::NegativeAmount => "NegativeAmount",
        PayloadError::AmountOverflow => "AmountOverflow",
        PayloadError::AmountExceedsLegacyPrecision => "AmountExceedsLegacyPrecision",
    }
}

fn render(result: Result<String, PayloadError>) -> String {
    match result {
        Ok(payload) => format!("payload:{payload}"),
        Err(error) => format!("error:{}", error_name(&error)),
    }
}

fn parsed_send_outcome(text: &str, property_id: u32) -> String {
    match parse_atomic_amount(text) {
        Ok(atoms) => render(build_simple_send("alice", property_id, atoms)),
        Err(error) => format!("error:{}", error_name(&error)),
    }
}

struct SendCase {
    recipient: String,
    property_id: u32,
    amount_atoms: u64,
}

fn send_case(id: &str) -> Option<SendCase> {
    let (recipient, property_id, amount_atoms) = match id {
        "send-whole-two" => ("alice".to_owned(), 5, 2 * ATOMS_PER_TOKEN),
        "send-property-min" => ("alice".to_owned(), 1, ATOMS_PER_TOKEN),
        "send-property-max" => ("alice".to_owned(), u32::MAX, ATOMS_PER_TOKEN),
        "send-zero-amount" => ("alice".to_owned(), 5, 0),
        "send-recipient-90" => ("a".repeat(90), 5, 2 * ATOMS_PER_TOKEN),
        "send-recipient-91" => ("a".repeat(91), 5, 2 * ATOMS_PER_TOKEN),
        "send-safe-max-whole" => ("alice".to_owned(), 1, 90_071_992 * ATOMS_PER_TOKEN),
        "send-half-token" => ("alice".to_owned(), 5, 50_000_000),
        "send-one-atom" => ("alice".to_owned(), 5, 1),
        "send-safe-max-atoms" => ("alice".to_owned(), 5, MAX_LEGACY_SAFE_ATOMS),
        "send-u64max-atoms" => ("alice".to_owned(), 1, u64::MAX),
        "send-empty-recipient" => (String::new(), 5, 2 * ATOMS_PER_TOKEN),
        "send-semicolon-recipient" => ("a;b".to_owned(), 5, 2 * ATOMS_PER_TOKEN),
        _ => return None,
    };
    Some(SendCase {
        recipient,
        property_id,
        amount_atoms,
    })
}

#[derive(Clone)]
enum Channel {
    Address(String),
    Reference(u32),
}

struct CommitCase {
    property_id: u32,
    amount_atoms: u64,
    channel: Channel,
    pay_enabled: bool,
    clear_lists: ClearLists,
    colored_output: bool,
    commit_clearlist_id: Option<u32>,
}

impl CommitCase {
    fn input(&self) -> CommitToChannel<'_> {
        CommitToChannel {
            property_id: self.property_id,
            amount_atoms: self.amount_atoms,
            channel: match &self.channel {
                Channel::Address(address) => ChannelLocation::Address(address),
                Channel::Reference(index) => ChannelLocation::Reference(*index),
            },
            pay_enabled: self.pay_enabled,
            clear_lists: self.clear_lists,
            colored_output: self.colored_output,
            commit_clearlist_id: self.commit_clearlist_id,
        }
    }
}

fn commit_case(id: &str) -> Option<CommitCase> {
    let base = || CommitCase {
        property_id: 7,
        amount_atoms: 150_000_000,
        channel: Channel::Address("chan".to_owned()),
        pay_enabled: false,
        clear_lists: ClearLists::Single(7),
        colored_output: false,
        commit_clearlist_id: None,
    };
    Some(match id {
        "commit-basic" => CommitCase {
            pay_enabled: true,
            ..base()
        },
        "commit-property-max-clearlist-id-max" => CommitCase {
            property_id: u32::MAX,
            amount_atoms: 1,
            pay_enabled: true,
            clear_lists: ClearLists::Single(1),
            colored_output: true,
            commit_clearlist_id: Some(u32::MAX),
            ..base()
        },
        "commit-zero-amount-no-clearlist" => CommitCase {
            property_id: 1,
            amount_atoms: 0,
            clear_lists: ClearLists::Absent,
            ..base()
        },
        "commit-clearlist-id-zero" => CommitCase {
            commit_clearlist_id: Some(0),
            ..base()
        },
        "commit-safe-max-atoms" => CommitCase {
            amount_atoms: MAX_LEGACY_SAFE_ATOMS,
            ..base()
        },
        "commit-long-channel-reference" => CommitCase {
            channel: Channel::Reference(5),
            ..base()
        },
        "commit-reference-zero" => CommitCase {
            channel: Channel::Reference(0),
            ..base()
        },
        "commit-long-channel-no-reference" => CommitCase {
            channel: Channel::Address("C".repeat(43)),
            ..base()
        },
        "commit-empty-channel" => CommitCase {
            channel: Channel::Address(String::new()),
            ..base()
        },
        "commit-clearlist-zero" => CommitCase {
            clear_lists: ClearLists::Single(0),
            ..base()
        },
        "commit-u64max-atoms" => CommitCase {
            amount_atoms: u64::MAX,
            ..base()
        },
        "commit-framed-body-property" => CommitCase {
            property_id: 1065,
            amount_atoms: ATOMS_PER_TOKEN,
            ..base()
        },
        _ => return None,
    })
}

struct AttestCase {
    revoke: bool,
    list_id: u32,
    target: String,
    metadata: String,
}

fn attest_case(id: &str) -> Option<AttestCase> {
    let (revoke, list_id, target, metadata) = match id {
        "attest-revoke-zero" => (false, 7, "alice".to_owned(), "OK".to_owned()),
        "attest-revoke-one" => (true, 7, "alice".to_owned(), "OK".to_owned()),
        "attest-id-max" => (false, u32::MAX, "alice".to_owned(), "OK".to_owned()),
        "attest-empty-metadata" => (false, 7, "alice".to_owned(), String::new()),
        "attest-long-metadata" => (false, 7, "alice".to_owned(), "META-".repeat(60)),
        "attest-revoke-boolean-true" => (true, 7, "alice".to_owned(), "OK".to_owned()),
        "attest-comma-metadata" => (false, 7, "alice".to_owned(), "A,B".to_owned()),
        "attest-lowercase-metadata" => (false, 7, "alice".to_owned(), "ok".to_owned()),
        "attest-empty-target" => (false, 7, String::new(), "OK".to_owned()),
        "attest-target-91" => (false, 7, "a".repeat(91), "OK".to_owned()),
        _ => return None,
    };
    Some(AttestCase {
        revoke,
        list_id,
        target,
        metadata,
    })
}

/// Maps a fixture row to the outcome of the Rust builders for that row's input.
fn actual_outcome(id: &str) -> String {
    if let Some(case) = commit_case(id) {
        return render(build_commit_to_channel(&case.input()));
    }
    if let Some(case) = send_case(id) {
        return render(build_simple_send(
            &case.recipient,
            case.property_id,
            case.amount_atoms,
        ));
    }
    if let Some(case) = attest_case(id) {
        return render(build_attestation(
            case.revoke,
            case.list_id,
            &case.target,
            &case.metadata,
        ));
    }
    match id {
        "send-float-text" => parsed_send_outcome("1.5", 1),
        "send-negative-number" => parsed_send_outcome("-1", 1),
        "send-u64max-decimal-text" => parsed_send_outcome("18446744073709551615", 1),
        "send-overflow-text" => parsed_send_outcome("18446744073709551616", 1),
        "commit-clearlist-array" => "unrepresentable:array-clearlist-split-by-decoder".to_owned(),
        "commit-subatomic-text" => "unrepresentable:integer-atoms-api".to_owned(),
        "withdraw-all-true" | "withdraw-one-token" | "withdraw-u64max-text" => {
            "not-implemented".to_owned()
        }
        other => panic!("fixture row {other} has no Rust mapping"),
    }
}

#[test]
fn fixture_rows_match_rust_outcomes_and_parity_column() {
    let rows = rows();
    assert_eq!(rows.len(), EXPECTED_ROWS);
    let mut byte_identical = 0;
    for row in &rows {
        assert_eq!(
            actual_outcome(row.id),
            row.rust_outcome,
            "fixture row {} ({})",
            row.id,
            row.family
        );
        let identical = row.rust_outcome.strip_prefix("payload:") == Some(row.js_payload);
        assert_eq!(
            row.parity,
            if identical { "match" } else { "divergent" },
            "parity column of {}",
            row.id
        );
        if identical {
            byte_identical += 1;
        }
    }
    assert_eq!(byte_identical, EXPECTED_BYTE_IDENTICAL);
}

#[test]
fn send_payloads_round_trip_through_wire_decoder() {
    for id in [
        "send-whole-two",
        "send-property-min",
        "send-property-max",
        "send-zero-amount",
        "send-recipient-90",
        "send-safe-max-whole",
        "send-half-token",
        "send-one-atom",
        "send-safe-max-atoms",
    ] {
        let case = send_case(id).expect("send case");
        let payload = build_simple_send(&case.recipient, case.property_id, case.amount_atoms)
            .expect("payload builds");
        assert_eq!(
            parse_typed(&payload),
            Ok(ParsedTransaction::Send {
                recipient: case.recipient.clone(),
                property_id: case.property_id,
                amount_atoms: u128::from(case.amount_atoms),
            }),
            "round trip of {id}: {payload}"
        );
    }
}

#[test]
fn commit_payloads_round_trip_through_legacy_decoder() {
    for id in [
        "commit-basic",
        "commit-property-max-clearlist-id-max",
        "commit-zero-amount-no-clearlist",
        "commit-clearlist-id-zero",
        "commit-safe-max-atoms",
        "commit-long-channel-reference",
        "commit-reference-zero",
        "commit-clearlist-zero",
    ] {
        let case = commit_case(id).expect("commit case");
        let payload = build_commit_to_channel(&case.input()).expect("payload builds");
        let body = &payload[3..];
        let decoded = source_decode_body_unverified(4, body)
            .unwrap_or_else(|error| panic!("{id}: decoder rejected {body}: {error:?}"));
        let expected = UnverifiedSourceBody::Commit {
            property: SourcePropertyId::Regular(u64::from(case.property_id)),
            amount_atoms: u128::from(case.amount_atoms),
            channel: match &case.channel {
                Channel::Address(address) => SourceChannelLocation::Address(address.clone()),
                Channel::Reference(index) => SourceChannelLocation::RawReference(index.to_string()),
            },
            pay_enabled: case.pay_enabled,
            clear_lists: match case.clear_lists {
                ClearLists::Absent => Vec::new(),
                ClearLists::Single(id) => vec![u64::from(id)],
            },
            colored_output: case.colored_output,
            commit_clearlist_id: case.commit_clearlist_id.map(u64::from),
        };
        assert_eq!(
            decoded.source_fields(),
            &expected,
            "round trip of {id}: {payload}"
        );
    }
}

#[test]
fn attestation_payloads_round_trip_through_legacy_decoder() {
    for id in [
        "attest-revoke-zero",
        "attest-revoke-one",
        "attest-id-max",
        "attest-empty-metadata",
        "attest-long-metadata",
        "attest-revoke-boolean-true",
    ] {
        let case = attest_case(id).expect("attestation case");
        let payload = build_attestation(case.revoke, case.list_id, &case.target, &case.metadata)
            .expect("payload builds");
        let decoded = source_decode_body_unverified(9, &payload[3..])
            .unwrap_or_else(|error| panic!("{id}: decoder rejected: {error:?}"));
        assert_eq!(
            decoded.source_fields(),
            &UnverifiedSourceBody::Attestation {
                revoke: case.revoke,
                list_id: u64::from(case.list_id),
                target_address: case.target.clone(),
                metadata_upper: case.metadata.clone(),
            },
            "round trip of {id}: {payload}"
        );
    }
}

/// Documents what the JavaScript bytes mean to the legacy decoders. These
/// are the reasons the corresponding Rust outputs differ from JS.
#[test]
fn documented_javascript_divergences_as_read_by_decoders() {
    // Fractional send: JS omits the ~, so 0.5 tokens reads as 5e7 whole tokens.
    assert_eq!(
        parse_typed("tl20;alice;5;tro8w"),
        Ok(ParsedTransaction::Send {
            recipient: "alice".into(),
            property_id: 5,
            amount_atoms: 50_000_000 * u128::from(ATOMS_PER_TOKEN),
        })
    );
    assert_eq!(
        parse_typed("tl20;alice;5;tro8w~").map(|tx| match tx {
            ParsedTransaction::Send { amount_atoms, .. } => amount_atoms,
            other => panic!("unexpected {other:?}"),
        }),
        Ok(50_000_000)
    );
    // One atomic unit: JS bytes read as one whole token.
    assert_eq!(
        parse_typed("tl20;alice;5;1").map(|tx| match tx {
            ParsedTransaction::Send { amount_atoms, .. } => amount_atoms,
            other => panic!("unexpected {other:?}"),
        }),
        Ok(u128::from(ATOMS_PER_TOKEN))
    );
    // u64::MAX atoms in JS bytes read as whole tokens far beyond u64 atoms.
    assert_eq!(
        parse_typed("tl20;alice;1;3w5e11264sgsf").map(|tx| match tx {
            ParsedTransaction::Send { amount_atoms, .. } => amount_atoms,
            other => panic!("unexpected {other:?}"),
        }),
        Ok(u128::from(u64::MAX) * u128::from(ATOMS_PER_TOKEN))
    );

    // Withdrawal: literal true is not withdrawAll, and decimal atoms decode as base-36.
    match source_decode_body_unverified(21, "true,7,100000000,1,chan") {
        Ok(decoded) => match decoded.source_fields() {
            UnverifiedSourceBody::Withdrawal {
                withdraw_all,
                amount_atoms,
                ..
            } => {
                assert!(!withdraw_all, "JS true must decode as withdrawAll=false");
                assert_eq!(*amount_atoms, parse_base36("100000000").expect("base36"));
                assert_ne!(*amount_atoms, 100_000_000);
            }
            other => panic!("unexpected {other:?}"),
        },
        Err(error) => panic!("withdrawal body rejected: {error:?}"),
    }

    // Boolean revoke: JS writes "true", which decodes as not revoked.
    let boolean_revoke = source_decode_body_unverified(9, "true,7,alice,OK").expect("decodes");
    assert!(matches!(
        boolean_revoke.source_fields(),
        UnverifiedSourceBody::Attestation { revoke: false, .. }
    ));

    // Comma in metadata: decoder silently keeps only the first fragment.
    let comma = source_decode_body_unverified(9, "0,7,alice,A,B").expect("decodes");
    assert!(matches!(
        comma.source_fields(),
        UnverifiedSourceBody::Attestation { metadata_upper, .. } if metadata_upper == "A"
    ));

    // Clearlist 0 is dropped by JS; the decoder then reads no clearlist.
    let dropped = source_decode_body_unverified(4, "7,2hb0qo,chan,0,,0").expect("decodes");
    assert!(matches!(
        dropped.source_fields(),
        UnverifiedSourceBody::Commit { clear_lists, .. } if clear_lists.is_empty()
    ));

    // Long channel with no ref: JS writes ref:0, which decodes as reference 0.
    let redirected = source_decode_body_unverified(4, "7,2hb0qo,ref:0,0,7,0").expect("decodes");
    assert!(matches!(
        redirected.source_fields(),
        UnverifiedSourceBody::Commit { channel: SourceChannelLocation::RawReference(r), .. } if r == "0"
    ));

    // Array clearlist: the decoder splits [7,b] on comma before bracket parsing.
    assert_eq!(
        source_decode_body_unverified(4, "7,2hb0qo,chan,0,[7,b],0"),
        Err(LegacyDecoderError::BracketedListSplitBeforeParsing)
    );

    // Framed commit body: the crate decoder refuses it although JS decodes it.
    assert_eq!(
        source_decode_body_unverified(4, "tl,1njchs,chan,0,7,0"),
        Err(LegacyDecoderError::NestedOrFramedBody)
    );
}
