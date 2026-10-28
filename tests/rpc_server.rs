//! Read-only JSON-RPC server tests. Exact response bytes are the contract
//! for DRAFT shapes; they are not a parity claim against the JS/C++ node.
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tradelayer_core::joint::JointProtocolState;
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::registry::Registry;
use tradelayer_core::rpc_server::{
    check_bind_policy, handle, read_http_body, serve, BindError, HttpError, MAX_REQUEST_BODY_BYTES,
};
use tradelayer_core::spot::SpotExchange;

fn p(number: u32) -> PropertyId {
    PropertyId::try_from(number).unwrap()
}

fn a(address: &str) -> AccountId {
    AccountId::try_from(address).unwrap()
}

fn tx(number: u64) -> TxId {
    let mut bytes = [0_u8; 32];
    bytes[24..].copy_from_slice(&number.to_be_bytes());
    TxId::from_bytes(bytes)
}

fn seeded_ledger() -> Ledger {
    let mut ledger = Ledger::new();
    let ops = [
        Operation::CreateProperty {
            id: p(7),
            issuer: a("alice"),
            kind: PropertyKind::Fixed,
            initial_atoms: 1_000,
        },
        Operation::CreateProperty {
            id: p(9),
            issuer: a("bob"),
            kind: PropertyKind::Managed,
            initial_atoms: 500,
        },
        Operation::Transfer {
            id: p(7),
            from: a("alice"),
            to: a("carol"),
            amount_atoms: 250,
        },
        Operation::Transfer {
            id: p(9),
            from: a("bob"),
            to: a("carol"),
            amount_atoms: 5,
        },
    ];
    for (i, op) in ops.into_iter().enumerate() {
        ledger.apply(tx(i as u64 + 1), op).unwrap();
    }
    ledger
}

fn permitted() -> BTreeSet<PropertyId> {
    [p(7), p(9)].into_iter().collect()
}

fn admin() -> AccountId {
    a("admin")
}

/// Seeded state with a finalized height of 42 from the spot migration path.
fn finalized_state() -> JointProtocolState {
    let mut spot = SpotExchange::from_ledger_unverified(seeded_ledger(), permitted()).unwrap();
    spot.process_through_unverified(42).unwrap();
    JointProtocolState::migrate_spot_components_unverified(
        TradeLayerMainnet::Litecoin,
        Registry::new(admin()),
        spot,
        42,
    )
    .unwrap()
}

/// Same ledger with no finalized height.
fn bootstrap_state() -> JointProtocolState {
    JointProtocolState::from_bootstrap_unverified(
        TradeLayerMainnet::Litecoin,
        admin(),
        seeded_ledger(),
        permitted(),
    )
    .unwrap()
}

fn call(state: &JointProtocolState, body: &str) -> String {
    String::from_utf8(handle(state, body.as_bytes())).unwrap()
}

fn post_bytes(body: &[u8]) -> Vec<u8> {
    let mut wire = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(body);
    wire
}

const INVALID_PARAMS_ID1: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params"},"id":1}"#;
const PARSE_ERROR_NULL_ID: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error"},"id":null}"#;
const INVALID_REQUEST_NULL_ID: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid Request"},"id":null}"#;
const INVALID_REQUEST_ID1: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid Request"},"id":1}"#;

// ---------------------------------------------------------------------------
// Method responses: exact bytes against the seeded state.
// ---------------------------------------------------------------------------

#[test]
fn getinfo_exact_bytes() {
    let state = finalized_state();
    assert_eq!(
        call(&state, r#"{"jsonrpc":"2.0","method":"tl_getinfo","id":1}"#),
        r#"{"jsonrpc":"2.0","result":{"network":"litecoin","finalized_height":42,"property_count":2,"processed_count":0},"id":1}"#
    );
}

#[test]
fn getinfo_reports_null_height_without_finalization() {
    let state = bootstrap_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":1}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"network":"litecoin","finalized_height":null,"property_count":2,"processed_count":0},"id":1}"#
    );
}

#[test]
fn getproperty_exact_bytes_and_string_id_echo() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":[7],"id":"req-1"}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"propertyid":7,"issuer":"alice","kind":"fixed","total_supply_atoms":"1000"},"id":"req-1"}"#
    );
}

#[test]
fn getproperty_unknown_is_property_not_found() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":[99],"id":1}"#
        ),
        r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"Property not found"},"id":1}"#
    );
}

#[test]
fn listproperties_exact_bytes_in_ascending_order() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_listproperties","params":[],"id":2}"#
        ),
        r#"{"jsonrpc":"2.0","result":[{"propertyid":7,"issuer":"alice","kind":"fixed","total_supply_atoms":"1000"},{"propertyid":9,"issuer":"bob","kind":"managed","total_supply_atoms":"500"}],"id":2}"#
    );
}

#[test]
fn getbalance_exact_bytes() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getbalance","params":["carol",7],"id":3}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"address":"carol","propertyid":7,"balance":"250"},"id":3}"#
    );
}

#[test]
fn getbalance_unknown_account_is_zero_and_unknown_property_is_error() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getbalance","params":["nobody",7],"id":3}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"address":"nobody","propertyid":7,"balance":"0"},"id":3}"#
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getbalance","params":["carol",99],"id":3}"#
        ),
        r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"Property not found"},"id":3}"#
    );
}

#[test]
fn getallbalancesforaddress_exact_bytes_in_property_order() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getallbalancesforaddress","params":["carol"],"id":4}"#
        ),
        r#"{"jsonrpc":"2.0","result":[{"propertyid":7,"balance":"250"},{"propertyid":9,"balance":"5"}],"id":4}"#
    );
}

#[test]
fn getsyncstatus_exact_bytes() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getsyncstatus","params":[],"id":5}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"source":"checkpoint","finalized_height":42,"processed_count":0},"id":5}"#
    );
}

#[test]
fn money_stays_exact_at_u128_max() {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            tx(1),
            Operation::CreateProperty {
                id: p(7),
                issuer: a("alice"),
                kind: PropertyKind::Fixed,
                initial_atoms: u128::MAX,
            },
        )
        .unwrap();
    let state = JointProtocolState::from_bootstrap_unverified(
        TradeLayerMainnet::Litecoin,
        admin(),
        ledger,
        [p(7)].into_iter().collect(),
    )
    .unwrap();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":[7],"id":1}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"propertyid":7,"issuer":"alice","kind":"fixed","total_supply_atoms":"340282366920938463463374607431768211455"},"id":1}"#
    );
}

#[test]
fn responses_are_byte_reproducible() {
    let state = finalized_state();
    let request = r#"{"method":"tl_listproperties","id":1,"jsonrpc":"2.0"}"#;
    let first = handle(&state, request.as_bytes());
    let second = handle(&state, request.as_bytes());
    assert_eq!(first, second);
    // Key order is fixed by the server, not by the request's member order.
    assert!(first.starts_with(br#"{"jsonrpc":"2.0","result":"#));
}

// ---------------------------------------------------------------------------
// Methods that do not exist, including every write-shaped name.
// ---------------------------------------------------------------------------

#[test]
fn unknown_method_is_method_not_found() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getinfo2","params":[],"id":9}"#
        ),
        r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":9}"#
    );
}

#[test]
fn write_shaped_methods_are_never_served() {
    let state = finalized_state();
    for method in [
        "tl_send",
        "tl_sendrawtransaction",
        "tl_createpayload_simplesend",
        "tl_createrawtx_opreturn",
        "tl_createpayload_commit_tochannel",
        "tl_createpayload_sendvesting",
        "tl_sendtoaddress",
        "tl_getinfo_write",
    ] {
        let body = format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":[],"id":1}}"#);
        assert_eq!(
            call(&state, &body),
            r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":1}"#,
            "method {method} must not be served"
        );
    }
}

#[test]
fn method_names_are_case_sensitive() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"TL_GETINFO","params":[],"id":1}"#
        ),
        r#"{"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":1}"#
    );
}

// ---------------------------------------------------------------------------
// Malformed JSON, envelope errors and param errors.
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_is_parse_error() {
    let state = finalized_state();
    for body in [
        r#"{"jsonrpc":"2.0","#,
        "",
        r#"{"jsonrpc":"2.0","method":"tl_getinfo","id":1} trailing"#,
        r#"{"jsonrpc":"2.0","method":"tl_getinfo","id":1,}"#,
        r#"{"jsonrpc":"2.0","method":"tl_getinfo","id":01}"#,
        r#"{"jsonrpc":"2.0","method":"tl_getinfo","id":1.}"#,
        r#"{"jsonrpc":"2.0","method":"\ud800","id":1}"#,
        "{\"jsonrpc\":\"2.0\",\"method\":\"a\x01\",\"id\":1}",
    ] {
        assert_eq!(call(&state, body), PARSE_ERROR_NULL_ID, "body: {body:?}");
    }
    let invalid_utf8: &[u8] = b"{\"jsonrpc\":\"2.0\",\"method\":\"\xff\",\"id\":1}";
    assert_eq!(
        String::from_utf8(handle(&state, invalid_utf8)).unwrap(),
        PARSE_ERROR_NULL_ID
    );
}

#[test]
fn batch_arrays_and_scalars_are_invalid_request() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"[{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":1}]"#
        ),
        INVALID_REQUEST_NULL_ID
    );
    assert_eq!(call(&state, "[]"), INVALID_REQUEST_NULL_ID);
    assert_eq!(call(&state, "42"), INVALID_REQUEST_NULL_ID);
}

#[test]
fn envelope_violations_are_invalid_request() {
    let state = finalized_state();
    // A valid id is still echoed on these envelope errors.
    assert_eq!(
        call(&state, r#"{"method":"tl_getinfo","params":[],"id":1}"#),
        INVALID_REQUEST_ID1
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"1.0","method":"tl_getinfo","params":[],"id":1}"#
        ),
        INVALID_REQUEST_ID1
    );
    assert_eq!(
        call(&state, r#"{"jsonrpc":"2.0","params":[],"id":1}"#),
        INVALID_REQUEST_ID1
    );
    assert_eq!(
        call(&state, r#"{"jsonrpc":"2.0","method":7,"params":[],"id":1}"#),
        INVALID_REQUEST_ID1
    );
    // Notification (no id) is refused rather than silently unanswered.
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getinfo","params":[]}"#
        ),
        INVALID_REQUEST_NULL_ID
    );
    // Invalid id types are refused and echoed as null.
    for id in [r#"{"x":1}"#, "[1]", "1.5", "true"] {
        let body = format!(r#"{{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":{id}}}"#);
        assert_eq!(call(&state, &body), INVALID_REQUEST_NULL_ID, "id {id}");
    }
}

#[test]
fn null_and_integer_ids_are_echoed_verbatim() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getsyncstatus","params":[],"id":null}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"source":"checkpoint","finalized_height":42,"processed_count":0},"id":null}"#
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getsyncstatus","params":[],"id":-7}"#
        ),
        r#"{"jsonrpc":"2.0","result":{"source":"checkpoint","finalized_height":42,"processed_count":0},"id":-7}"#
    );
}

#[test]
fn wrong_param_types_are_invalid_params() {
    let state = finalized_state();
    let bad_params = [
        r#"["7"]"#,      // string where a number is required
        "[7.0]",         // non-integer number
        "[1e3]",         // exponent form
        "[-1]",          // negative
        "[0]",           // zero is not a property id
        "[4294967296]",  // overflows u32
        "[null]",        // null
        "[true]",        // boolean
        r#"[{"id":7}]"#, // object
    ];
    for params in bad_params {
        let body =
            format!(r#"{{"jsonrpc":"2.0","method":"tl_getproperty","params":{params},"id":1}}"#);
        assert_eq!(call(&state, &body), INVALID_PARAMS_ID1, "params {params}");
    }
    // Swapped getbalance arguments, string-typed property id, bad address.
    for params in [
        r#"[7,"carol"]"#,
        r#"["carol","7"]"#,
        "[7]",
        r#"["ca rol",7]"#,
    ] {
        let body =
            format!(r#"{{"jsonrpc":"2.0","method":"tl_getbalance","params":{params},"id":1}}"#);
        assert_eq!(call(&state, &body), INVALID_PARAMS_ID1, "params {params}");
    }
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getallbalancesforaddress","params":[7],"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
}

#[test]
fn params_must_be_a_positional_array() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":{"propertyid":7},"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":null,"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
}

#[test]
fn missing_or_extra_params_are_invalid_params() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getproperty","params":[],"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getbalance","params":["carol"],"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getinfo","params":[1],"id":1}"#
        ),
        INVALID_PARAMS_ID1
    );
}

#[test]
fn nesting_depth_cap_is_exactly_eight_containers() {
    let state = finalized_state();
    let nested = |levels: usize| {
        format!(
            r#"{{"jsonrpc":"2.0","method":"tl_getinfo","id":1,"params":{}{}}}"#,
            "[".repeat(levels),
            "]".repeat(levels)
        )
    };
    // Object plus 7 nested arrays = 8 containers: parses, then fails arity.
    assert_eq!(call(&state, &nested(7)), INVALID_PARAMS_ID1);
    // Object plus 8 nested arrays = 9 containers: rejected by the parser.
    assert_eq!(call(&state, &nested(8)), INVALID_REQUEST_NULL_ID);
}

#[test]
fn duplicate_keys_are_invalid_request() {
    let state = finalized_state();
    assert_eq!(
        call(
            &state,
            r#"{"jsonrpc":"2.0","method":"tl_getinfo","method":"tl_getsyncstatus","id":1}"#
        ),
        INVALID_REQUEST_NULL_ID
    );
}

#[test]
fn oversized_strings_values_and_numbers_are_invalid_request() {
    let state = finalized_state();
    let long_string = "a".repeat(2_000);
    assert_eq!(
        call(
            &state,
            &format!(r#"{{"jsonrpc":"2.0","method":"{long_string}","id":1}}"#)
        ),
        INVALID_REQUEST_NULL_ID
    );
    let many_values = format!(
        r#"{{"jsonrpc":"2.0","method":"tl_listproperties","id":1,"params":[{}0]}}"#,
        "0,".repeat(600)
    );
    assert_eq!(call(&state, &many_values), INVALID_REQUEST_NULL_ID);
    let long_number = format!(
        r#"{{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":{}}}"#,
        "1".repeat(40)
    );
    assert_eq!(call(&state, &long_number), INVALID_REQUEST_NULL_ID);
}

#[test]
fn request_over_the_body_cap_is_rejected_by_the_handler() {
    let state = finalized_state();
    let padded = format!(
        r#"{{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":1,"pad":"{}"}}"#,
        " ".repeat(MAX_REQUEST_BODY_BYTES)
    );
    assert!(padded.len() > MAX_REQUEST_BODY_BYTES);
    assert_eq!(call(&state, &padded), INVALID_REQUEST_NULL_ID);
}

// ---------------------------------------------------------------------------
// HTTP framing. A `&[u8]` is a BufRead, so consumed bytes are observable.
// ---------------------------------------------------------------------------

#[test]
fn well_formed_post_returns_exact_body() {
    let body = br#"{"jsonrpc":"2.0","method":"tl_getinfo","params":[],"id":1}"#;
    let wire = post_bytes(body);
    let mut reader: &[u8] = &wire;
    assert_eq!(read_http_body(&mut reader).unwrap(), body.to_vec());
    assert!(reader.is_empty());
}

#[test]
fn body_at_the_cap_is_accepted_and_one_past_is_refused() {
    let at_cap = vec![b' '; MAX_REQUEST_BODY_BYTES];
    let wire = post_bytes(&at_cap);
    let mut reader: &[u8] = &wire;
    assert_eq!(
        read_http_body(&mut reader).unwrap().len(),
        MAX_REQUEST_BODY_BYTES
    );

    let over = vec![b' '; MAX_REQUEST_BODY_BYTES + 1];
    let wire = post_bytes(&over);
    let mut reader: &[u8] = &wire;
    assert_eq!(read_http_body(&mut reader), Err(HttpError::PayloadTooLarge));
}

#[test]
fn seventy_kib_body_is_rejected_before_its_bytes_are_read() {
    // The declared length is checked against the cap before any body byte
    // is consumed or any buffer is sized from it. Only the head is consumed.
    let body = vec![b'x'; 70 * 1024];
    let head = format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", body.len());
    let mut wire = head.into_bytes();
    wire.extend_from_slice(&body);
    let head_len = wire.len() - body.len();
    let mut reader: &[u8] = &wire;
    assert_eq!(read_http_body(&mut reader), Err(HttpError::PayloadTooLarge));
    assert_eq!(
        reader.len(),
        wire.len() - head_len,
        "exactly the head was consumed; the 70 KiB body is untouched"
    );
}

#[test]
fn absurd_content_length_is_refused() {
    let overflow = "1".repeat(21);
    for length in [
        "18446744073709551616",
        "99999999999999999999999",
        overflow.as_str(),
    ] {
        let wire = format!("POST / HTTP/1.1\r\nContent-Length: {length}\r\n\r\n");
        let mut reader: &[u8] = wire.as_bytes();
        assert_eq!(
            read_http_body(&mut reader),
            Err(HttpError::BadRequest),
            "length {length}"
        );
    }
    let wire = "POST / HTTP/1.1\r\nContent-Length: 18446744073709551615\r\n\r\n";
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::PayloadTooLarge));
}

#[test]
fn chunked_bodies_are_refused_without_reading_the_body() {
    let wire = "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::NotImplemented));
    let head_len = wire.find("\r\n\r\n").unwrap() + 4;
    assert_eq!(
        reader.len(),
        wire.len() - head_len,
        "only the head was consumed; the chunked body is untouched"
    );
}

#[test]
fn missing_content_length_is_length_required() {
    let wire = "POST / HTTP/1.1\r\nHost: x\r\n\r\n{}";
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::LengthRequired));
}

#[test]
fn malformed_content_length_is_bad_request() {
    for value in ["+5", "-1", " ", "5, 5", "0x10", ""] {
        let wire = format!("POST / HTTP/1.1\r\nContent-Length: {value}\r\n\r\n");
        let mut reader: &[u8] = wire.as_bytes();
        assert_eq!(
            read_http_body(&mut reader),
            Err(HttpError::BadRequest),
            "value {value:?}"
        );
    }
    let duplicated = "POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}";
    let mut reader: &[u8] = duplicated.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::BadRequest));
}

#[test]
fn short_body_is_bad_request() {
    let wire = "POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}";
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::BadRequest));
}

#[test]
fn oversized_header_block_is_refused() {
    let filler = "a".repeat(9 * 1024);
    let wire = format!("POST / HTTP/1.1\r\nX-Pad: {filler}\r\nContent-Length: 2\r\n\r\n{{}}");
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::HeaderTooLarge));
}

#[test]
fn wrong_method_path_or_version_is_refused() {
    let cases = [
        (
            "GET / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::MethodNotAllowed,
        ),
        (
            "PUT / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::MethodNotAllowed,
        ),
        (
            "POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::NotFound,
        ),
        (
            "POST / HTTP/2.0\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::BadRequest,
        ),
        (
            "POST / HTTP/1.1\r\n folded: x\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::BadRequest,
        ),
        (
            "POST / HTTP/1.1\r\nBad Name: x\r\nContent-Length: 2\r\n\r\n{}",
            HttpError::BadRequest,
        ),
    ];
    for (wire, expected) in cases {
        let mut reader: &[u8] = wire.as_bytes();
        assert_eq!(read_http_body(&mut reader), Err(expected), "wire {wire:?}");
    }
}

#[test]
fn bare_lf_line_endings_are_refused() {
    let wire = "POST / HTTP/1.1\nContent-Length: 2\n\n{}";
    let mut reader: &[u8] = wire.as_bytes();
    assert_eq!(read_http_body(&mut reader), Err(HttpError::BadRequest));
}

// ---------------------------------------------------------------------------
// Bind policy.
// ---------------------------------------------------------------------------

#[test]
fn loopback_binds_are_allowed() {
    for addr in ["127.0.0.1:0", "127.8.9.10:7360", "[::1]:0"] {
        let addr: SocketAddr = addr.parse().unwrap();
        assert_eq!(check_bind_policy(addr, false), Ok(()), "{addr}");
    }
}

#[test]
fn non_loopback_bind_is_refused_without_the_flag() {
    for addr in [
        "0.0.0.0:7360",
        "10.0.0.5:7360",
        "192.168.1.9:7360",
        "[::]:7360",
        "[2001:db8::1]:7360",
    ] {
        let addr: SocketAddr = addr.parse().unwrap();
        assert_eq!(
            check_bind_policy(addr, false),
            Err(BindError::NonLoopback(addr)),
            "{addr}"
        );
    }
}

#[test]
fn non_loopback_bind_is_allowed_only_with_explicit_flag() {
    let addr: SocketAddr = "0.0.0.0:7360".parse().unwrap();
    assert_eq!(check_bind_policy(addr, true), Ok(()));
}

// ---------------------------------------------------------------------------
// Checkpoint file loading.
// ---------------------------------------------------------------------------

fn checkpoint_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn write_checkpoint(name: &str, state: &JointProtocolState) -> PathBuf {
    let path = checkpoint_path(name);
    std::fs::write(&path, state.snapshot_bytes().unwrap()).unwrap();
    path
}

#[test]
fn checkpoint_file_round_trip_serves_identical_bytes() {
    let state = finalized_state();
    let path = write_checkpoint("rpc_round_trip.tljp", &state);
    let restored = JointProtocolState::from_snapshot_file(
        &path,
        TradeLayerMainnet::Litecoin,
        &admin(),
        &permitted(),
    )
    .unwrap();
    let request = br#"{"jsonrpc":"2.0","method":"tl_listproperties","params":[],"id":1}"#;
    assert_eq!(handle(&state, request), handle(&restored, request));
}

#[test]
fn checkpoint_is_rejected_for_the_wrong_network_or_missing_file() {
    let state = finalized_state();
    let path = write_checkpoint("rpc_wrong_network.tljp", &state);
    assert!(JointProtocolState::from_snapshot_file(
        &path,
        TradeLayerMainnet::Bitcoin,
        &admin(),
        &permitted()
    )
    .is_err());
    assert!(JointProtocolState::from_snapshot_file(
        &checkpoint_path("does_not_exist.tljp"),
        TradeLayerMainnet::Litecoin,
        &admin(),
        &permitted()
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// Real sockets: round trip, 413, read timeout, and the connection cap.
// ---------------------------------------------------------------------------

fn spawn_server(state: JointProtocolState) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let _ = serve(listener, Arc::new(state));
    });
    addr
}

/// Reads until EOF or error, keeping whatever arrived before either.
fn read_all(stream: &mut TcpStream) -> String {
    let mut out = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    stream
}

#[test]
fn loopback_socket_round_trip_returns_exact_http_and_body() {
    let addr = spawn_server(finalized_state());
    let body = r#"{"jsonrpc":"2.0","method":"tl_getbalance","params":["carol",7],"id":3}"#;
    let mut stream = connect(addr);
    stream.write_all(&post_bytes(body.as_bytes())).unwrap();
    let response = read_all(&mut stream);
    let expected_body =
        r#"{"jsonrpc":"2.0","result":{"address":"carol","propertyid":7,"balance":"250"},"id":3}"#;
    let expected = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
        expected_body.len(),
        expected_body
    );
    assert_eq!(response, expected);
}

#[test]
fn oversized_content_length_over_a_socket_gets_413() {
    let addr = spawn_server(finalized_state());
    let mut stream = connect(addr);
    // Only the head is sent. A server that waited for the body would hang.
    stream
        .write_all(b"POST / HTTP/1.1\r\nContent-Length: 70000\r\n\r\n")
        .unwrap();
    let response = read_all(&mut stream);
    assert!(
        response.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
        "got {response:?}"
    );
}

#[test]
fn idle_connection_is_closed_after_the_read_timeout() {
    let addr = spawn_server(finalized_state());
    let mut stream = connect(addr);
    let started = std::time::Instant::now();
    let response = read_all(&mut stream);
    let elapsed = started.elapsed();
    assert!(
        response.starts_with("HTTP/1.1 408 Request Timeout\r\n"),
        "got {response:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(4),
        "closed too early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(12),
        "closed too late: {elapsed:?}"
    );
}

#[test]
fn connection_cap_answers_503_past_sixteen_open_connections() {
    let addr = spawn_server(finalized_state());
    // Sixteen idle connections hold their slots until the read timeout.
    let held: Vec<TcpStream> = (0..16).map(|_| connect(addr)).collect();
    let mut extra = connect(addr);
    let response = read_all(&mut extra);
    assert!(
        response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
        "got {response:?}"
    );
    drop(held);
}

// ---------------------------------------------------------------------------
// The binary: refusal of a non-loopback bind, and a live loopback serve.
// ---------------------------------------------------------------------------

fn daemon_command(checkpoint: &Path, bind: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tradelayer-rpcd"));
    command
        .arg("--checkpoint")
        .arg(checkpoint)
        .args(["--network", "litecoin", "--admin", "admin"])
        .args(["--permitted", "7,9"])
        .args(["--bind", bind]);
    command
}

#[test]
fn binary_refuses_non_loopback_bind_before_loading_anything() {
    let path = write_checkpoint("rpc_binary_refuse.tljp", &finalized_state());
    let output = daemon_command(&path, "0.0.0.0:0").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non-loopback"), "stderr: {stderr}");
}

#[test]
fn binary_serves_a_checkpoint_on_loopback() {
    let path = write_checkpoint("rpc_binary_serve.tljp", &finalized_state());
    let mut child = daemon_command(&path, "127.0.0.1:0")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let addr: SocketAddr = line
        .trim()
        .rsplit("listening on ")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let mut stream = connect(addr);
    stream
        .write_all(&post_bytes(
            br#"{"jsonrpc":"2.0","method":"tl_getsyncstatus","params":[],"id":5}"#,
        ))
        .unwrap();
    let response = read_all(&mut stream);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "got {response:?}"
    );
    assert!(response.ends_with(
        r#"{"jsonrpc":"2.0","result":{"source":"checkpoint","finalized_height":42,"processed_count":0},"id":5}"#
    ));
}
