//! Phase 3 live follower tests. All blocks come from the in-memory mock or from
//! a loopback fake JSON-RPC server. No real node is contacted. Transactions and
//! actors are UNVERIFIED caller assertions, decoded by the test `BlockDecoder`.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::joint::{JointOperation, UnverifiedJointTx};
use tradelayer_core::joint_chain::{JointChainState, UnverifiedJointBlock};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::node_follower::{
    sync_to_source, synthetic_fields, synthetic_transaction, BlockDecoder, BlockSource,
    DecodableTx, FollowerError, MockBlockSource, MockFault, SourceError, SyncReport,
};
use tradelayer_core::node_rpc::{EndpointPolicy, RpcBlockSource, RpcConfig, RpcCredentials};
use tradelayer_core::raw_block_ingest::RawBlockIngestError;
use tradelayer_core::registry::RegistryAction;

const ADMIN: &str = "admin";
const RPC_USER: &str = "opsuser-4417";
const RPC_PASSWORD: &str = "hunter-SECRET-9b2e";
/// `base64("opsuser-4417:hunter-SECRET-9b2e")`, computed outside Rust.
const EXPECTED_BASIC: &str = "Basic b3BzdXNlci00NDE3Omh1bnRlci1TRUNSRVQtOWIyZQ==";
const HISTORY: usize = 8;

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}

fn acct(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}

fn ledger() -> Ledger {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            TxId::from_bytes([1; 32]),
            Operation::CreateProperty {
                id: p(7),
                issuer: acct("seller"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
        .apply(
            TxId::from_bytes([2; 32]),
            Operation::CreateProperty {
                id: p(8),
                issuer: acct("buyer"),
                kind: PropertyKind::Managed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
}

fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}

fn anchor(network: TradeLayerMainnet) -> Tip {
    Tip {
        height: 0,
        id: network.genesis(),
    }
}

fn fresh_with_history(network: TradeLayerMainnet, history: usize) -> JointChainState {
    JointChainState::new_unverified(
        network,
        anchor(network),
        acct(ADMIN),
        ledger(),
        permitted(),
        history,
    )
    .unwrap()
}

fn fresh(network: TradeLayerMainnet) -> JointChainState {
    fresh_with_history(network, HISTORY)
}

fn snapshot(state: &JointChainState) -> Vec<u8> {
    state.state().snapshot_bytes().unwrap()
}

/// Synthetic code to typed operation. Code 1 activates types, 2 and 3 send
/// small balances, and 4 overdraws, so it fails mid-block.
fn op_for(txid: TxId, code: u8, nonce: u32) -> Result<UnverifiedJointTx, String> {
    let amount = u128::from(nonce);
    let (actor, operation) = match code {
        1 => (
            ADMIN,
            JointOperation::Registry(RegistryAction::Activate {
                types: vec![1, 2, 5, 6, 11, 12],
                code_hash: [0x29; 32],
            }),
        ),
        2 => (
            "seller",
            JointOperation::Send {
                id: p(7),
                recipient: acct("buyer"),
                amount_atoms: amount,
            },
        ),
        3 => (
            "buyer",
            JointOperation::Send {
                id: p(8),
                recipient: acct("seller"),
                amount_atoms: amount,
            },
        ),
        4 => (
            "seller",
            JointOperation::Send {
                id: p(7),
                recipient: acct("buyer"),
                amount_atoms: 1_000_000_000,
            },
        ),
        other => return Err(format!("unknown synthetic code {other}")),
    };
    Ok(UnverifiedJointTx {
        txid,
        actor: acct(actor),
        operation,
    })
}

#[derive(Default)]
struct Scripted {
    heights: RefCell<Vec<u64>>,
}

impl BlockDecoder for Scripted {
    fn decode_block(
        &self,
        height: u64,
        transactions: &[DecodableTx<'_>],
    ) -> Result<Vec<UnverifiedJointTx>, String> {
        self.heights.borrow_mut().push(height);
        transactions
            .iter()
            .map(|tx| -> Result<UnverifiedJointTx, String> {
                let (code, nonce) = synthetic_fields(tx.raw)
                    .ok_or_else(|| "not a synthetic transaction".to_owned())?;
                op_for(tx.txid, code, nonce)
            })
            .collect()
    }
}

/// Decodes correctly but returns the operations in reverse block order.
struct Reversed;

impl BlockDecoder for Reversed {
    fn decode_block(
        &self,
        height: u64,
        transactions: &[DecodableTx<'_>],
    ) -> Result<Vec<UnverifiedJointTx>, String> {
        let mut decoded = Scripted::default().decode_block(height, transactions)?;
        decoded.reverse();
        Ok(decoded)
    }
}

/// Returns an operation whose txid does not occur in the block.
struct Forging;

impl BlockDecoder for Forging {
    fn decode_block(
        &self,
        _height: u64,
        _transactions: &[DecodableTx<'_>],
    ) -> Result<Vec<UnverifiedJointTx>, String> {
        Ok(vec![op_for(TxId::from_bytes([9; 32]), 2, 1)?])
    }
}

/// Independent oracle: applies a source's blocks directly through
/// `apply_unverified_block`, bypassing the follower.
fn straight_replay(source: &MockBlockSource, network: TradeLayerMainnet) -> JointChainState {
    let mut state = fresh(network);
    for height in 1..=source.tip_height() {
        let block = source.block(height).unwrap();
        let parent = if height == 1 {
            network.genesis()
        } else {
            source.block(height - 1).unwrap().id()
        };
        let transactions = block
            .transactions()
            .iter()
            .zip(block.txids())
            .map(|(raw, txid)| {
                let (code, nonce) = synthetic_fields(raw).unwrap();
                op_for(*txid, code, nonce).unwrap()
            })
            .collect();
        state
            .apply_unverified_block(&UnverifiedJointBlock {
                network,
                header: BlockHeader {
                    height,
                    id: block.id(),
                    parent,
                },
                transactions,
            })
            .unwrap();
    }
    state
}

/// Activation block followed by three sends. Each later test fork is built on
/// this shared three-block prefix.
fn common_prefix(network: TradeLayerMainnet) -> MockBlockSource {
    let mut source = MockBlockSource::for_network(network);
    source
        .push_block(vec![synthetic_transaction(1, 1)], 1)
        .unwrap();
    source
        .push_block(vec![synthetic_transaction(2, 5)], 2)
        .unwrap();
    source
        .push_block(vec![synthetic_transaction(3, 7)], 3)
        .unwrap();
    source
}

fn sync<S: BlockSource>(
    state: &mut JointChainState,
    source: &S,
) -> Result<SyncReport, FollowerError> {
    sync_to_source(state, source, &Scripted::default(), 100)
}

// ---------------------------------------------------------------------------
// Follower over the mock source.
// ---------------------------------------------------------------------------

#[test]
fn linear_advance_matches_straight_replay() {
    let network = TradeLayerMainnet::Bitcoin;
    let mut source = common_prefix(network);
    source
        .push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    let mut state = fresh(network);
    let report = sync(&mut state, &source).unwrap();
    assert_eq!(
        report,
        SyncReport {
            rolled_back: 0,
            applied: 4,
            tip: Tip {
                height: 4,
                id: source.tip_id(),
            },
        }
    );
    assert_eq!(
        snapshot(&state),
        snapshot(&straight_replay(&source, network))
    );
}

#[test]
fn reorg_depth_one_reaches_straight_replay_of_winning_branch() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut losing = common.clone();
    losing
        .push_block(vec![synthetic_transaction(2, 100)], 40)
        .unwrap();
    let mut winning = common.clone();
    winning
        .push_block(vec![synthetic_transaction(2, 200)], 41)
        .unwrap();

    let mut state = fresh(network);
    sync(&mut state, &losing).unwrap();
    let before = snapshot(&state);
    let report = sync(&mut state, &winning).unwrap();

    assert_eq!(report.rolled_back, 1);
    assert_eq!(report.applied, 1);
    assert_eq!(report.tip.id, winning.tip_id());
    assert_ne!(snapshot(&state), before);
    assert_eq!(
        snapshot(&state),
        snapshot(&straight_replay(&winning, network))
    );
}

#[test]
fn reorg_depth_three_reaches_straight_replay_of_winning_branch() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut losing = common.clone();
    losing
        .push_block(vec![synthetic_transaction(2, 100)], 40)
        .unwrap();
    losing
        .push_block(
            vec![synthetic_transaction(2, 101), synthetic_transaction(3, 102)],
            41,
        )
        .unwrap();
    losing
        .push_block(vec![synthetic_transaction(2, 103)], 42)
        .unwrap();
    let mut winning = common.clone();
    winning
        .push_block(vec![synthetic_transaction(2, 200)], 50)
        .unwrap();
    winning
        .push_block(vec![synthetic_transaction(3, 201)], 51)
        .unwrap();
    winning
        .push_block(
            vec![synthetic_transaction(2, 202), synthetic_transaction(3, 203)],
            52,
        )
        .unwrap();

    let mut state = fresh(network);
    sync(&mut state, &losing).unwrap();
    let report = sync(&mut state, &winning).unwrap();

    assert_eq!(report.rolled_back, 3);
    assert_eq!(report.applied, 3);
    assert_eq!(report.tip.height, 6);
    assert_eq!(report.tip.id, winning.tip_id());
    assert_eq!(
        snapshot(&state),
        snapshot(&straight_replay(&winning, network))
    );
}

#[test]
fn failing_block_leaves_state_byte_identical() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut state = fresh(network);
    sync(&mut state, &common).unwrap();
    let before = state.checkpoint_bytes().unwrap();

    // Block 4 is valid. Block 5 contains an overdraft that fails mid-block.
    let mut bad = common.clone();
    bad.push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    bad.push_block(vec![synthetic_transaction(4, 0)], 5)
        .unwrap();
    let err = sync(&mut state, &bad).unwrap_err();
    assert!(matches!(err, FollowerError::Apply(_)), "got {err:?}");
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
    assert_eq!(state.tip(), common_tip(&common));

    // A reorg whose new branch fails must also roll back its own rollback.
    let mut losing = common.clone();
    losing
        .push_block(vec![synthetic_transaction(2, 300)], 6)
        .unwrap();
    sync(&mut state, &losing).unwrap();
    let before_reorg = state.checkpoint_bytes().unwrap();
    let mut failing_branch = common.clone();
    failing_branch
        .push_block(vec![synthetic_transaction(2, 400)], 7)
        .unwrap();
    failing_branch
        .push_block(vec![synthetic_transaction(4, 0)], 8)
        .unwrap();
    let err = sync(&mut state, &failing_branch).unwrap_err();
    assert!(matches!(err, FollowerError::Apply(_)), "got {err:?}");
    assert_eq!(state.checkpoint_bytes().unwrap(), before_reorg);

    // Repairing the branch lets the same state reach the straight replay.
    let mut repaired = common.clone();
    repaired
        .push_block(vec![synthetic_transaction(2, 400)], 7)
        .unwrap();
    repaired
        .push_block(vec![synthetic_transaction(2, 401)], 8)
        .unwrap();
    sync(&mut state, &repaired).unwrap();
    assert_eq!(
        snapshot(&state),
        snapshot(&straight_replay(&repaired, network))
    );
}

fn common_tip(common: &MockBlockSource) -> Tip {
    Tip {
        height: common.tip_height(),
        id: common.tip_id(),
    }
}

#[test]
fn restart_from_checkpoint_resumes_at_right_height() {
    let network = TradeLayerMainnet::Bitcoin;
    let mut full = common_prefix(network);
    full.push_block(vec![synthetic_transaction(2, 200)], 50)
        .unwrap();
    full.push_block(vec![synthetic_transaction(3, 201)], 51)
        .unwrap();
    full.push_block(vec![synthetic_transaction(2, 202)], 52)
        .unwrap();
    let mut partial = full.clone();
    partial.truncate_to(3);

    let mut state = fresh(network);
    sync(&mut state, &partial).unwrap();
    assert_eq!(state.tip().height, 3);

    let path = temp_path("restart");
    state.write_new_checkpoint(&path).unwrap();
    drop(state);
    let mut resumed = JointChainState::load_checkpoint(
        &path,
        network,
        anchor(network),
        &acct(ADMIN),
        &permitted(),
        HISTORY,
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(resumed.tip().height, 3);

    let decoder = Scripted::default();
    let report = sync_to_source(&mut resumed, &full, &decoder, 100).unwrap();
    assert_eq!(report.applied, 3);
    assert_eq!(*decoder.heights.borrow(), vec![4, 5, 6]);
    assert_eq!(
        snapshot(&resumed),
        snapshot(&straight_replay(&full, network))
    );
}

#[test]
fn lying_best_height_fails_closed_without_state_change() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut state = fresh(network);
    sync(&mut state, &common).unwrap();
    let before = state.checkpoint_bytes().unwrap();

    let mut lying = common.clone();
    lying
        .push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    lying.set_fault(Some(MockFault::LyingBestHeight(50)));
    let err = sync(&mut state, &lying).unwrap_err();
    assert!(
        matches!(err, FollowerError::Source(SourceError::MissingBlock(5))),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn per_call_block_budget_is_enforced_and_uncommitted() {
    let network = TradeLayerMainnet::Bitcoin;
    let mut source = MockBlockSource::for_network(network);
    source
        .push_block(vec![synthetic_transaction(1, 0)], 0)
        .unwrap();
    for nonce in 1..=11u32 {
        source
            .push_block(vec![synthetic_transaction(2, nonce)], nonce)
            .unwrap();
    }
    // A source that claims an enormous height cannot drive unbounded work.
    source.set_fault(Some(MockFault::LyingBestHeight(u64::MAX)));
    let mut state = fresh(network);
    let before = state.checkpoint_bytes().unwrap();
    let err = sync_to_source(&mut state, &source, &Scripted::default(), 5).unwrap_err();
    assert!(
        matches!(err, FollowerError::BudgetExceeded { limit: 5 }),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);

    source.set_fault(None);
    let report = sync_to_source(&mut state, &source, &Scripted::default(), 100).unwrap();
    assert_eq!(report.applied, 12);
}

#[test]
fn short_and_oversized_raw_blocks_are_rejected_without_state_change() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut honest = common.clone();
    honest
        .push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    let mut state = fresh(network);
    sync(&mut state, &common).unwrap();
    let before = state.checkpoint_bytes().unwrap();

    let mut short = honest.clone();
    short.set_fault(Some(MockFault::ShortBlock(4)));
    let err = sync(&mut state, &short).unwrap_err();
    assert!(matches!(err, FollowerError::Raw(_)), "got {err:?}");
    assert_eq!(state.checkpoint_bytes().unwrap(), before);

    let mut oversized = honest.clone();
    oversized.set_fault(Some(MockFault::OversizedBlock(4)));
    let err = sync(&mut state, &oversized).unwrap_err();
    assert!(
        matches!(
            err,
            FollowerError::Raw(RawBlockIngestError::OversizedRawBlock)
        ),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn getblockhash_versus_header_hash_mismatch_is_rejected() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut honest = common.clone();
    honest
        .push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    // The source names block 4's hash but serves block 3's bytes.
    let mut lying = honest.clone();
    lying.set_fault(Some(MockFault::SwappedBlockBytes { height: 4, with: 3 }));
    let mut state = fresh(network);
    sync(&mut state, &common).unwrap();
    let before = state.checkpoint_bytes().unwrap();
    let err = sync(&mut state, &lying).unwrap_err();
    assert!(
        matches!(
            err,
            FollowerError::Raw(RawBlockIngestError::RequestedHeaderHashMismatch)
        ),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn litecoin_profile_is_refused_until_scrypt_block_hashes_exist() {
    let network = TradeLayerMainnet::Litecoin;
    let source = MockBlockSource::for_network(network);
    let mut state = fresh(network);
    let before = state.checkpoint_bytes().unwrap();
    let err = sync(&mut state, &source).unwrap_err();
    assert!(
        matches!(
            err,
            FollowerError::HeaderHashUnavailable(TradeLayerMainnet::Litecoin)
        ),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn source_on_a_different_genesis_is_refused() {
    let state_network = TradeLayerMainnet::Bitcoin;
    let other = MockBlockSource::for_network(TradeLayerMainnet::Litecoin);
    let mut state = fresh(state_network);
    let before = state.checkpoint_bytes().unwrap();
    let err = sync(&mut state, &other).unwrap_err();
    assert!(matches!(err, FollowerError::GenesisMismatch), "got {err:?}");
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn decoder_txids_must_belong_to_the_block_in_order() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut two_tx = common.clone();
    two_tx
        .push_block(
            vec![synthetic_transaction(2, 100), synthetic_transaction(3, 101)],
            4,
        )
        .unwrap();

    let mut state = fresh(network);
    sync(&mut state, &common).unwrap();
    let before = state.checkpoint_bytes().unwrap();

    let err = sync_to_source(&mut state, &two_tx, &Forging, 100).unwrap_err();
    assert!(
        matches!(err, FollowerError::DecodedTxNotInBlock { height: 4 }),
        "got {err:?}"
    );
    let err = sync_to_source(&mut state, &two_tx, &Reversed, 100).unwrap_err();
    assert!(
        matches!(err, FollowerError::DecodedTxOutOfOrder { height: 4 }),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

#[test]
fn reorg_deeper_than_retained_history_fails_closed() {
    let network = TradeLayerMainnet::Bitcoin;
    let common = common_prefix(network);
    let mut losing = common.clone();
    losing
        .push_block(vec![synthetic_transaction(2, 100)], 40)
        .unwrap();
    losing
        .push_block(vec![synthetic_transaction(2, 101)], 41)
        .unwrap();
    losing
        .push_block(vec![synthetic_transaction(2, 102)], 42)
        .unwrap();
    let mut winning = common.clone();
    winning
        .push_block(vec![synthetic_transaction(2, 200)], 50)
        .unwrap();
    winning
        .push_block(vec![synthetic_transaction(2, 201)], 51)
        .unwrap();
    winning
        .push_block(vec![synthetic_transaction(2, 202)], 52)
        .unwrap();

    let mut state = fresh_with_history(network, 1);
    sync(&mut state, &losing).unwrap();
    let before = state.checkpoint_bytes().unwrap();
    let err = sync(&mut state, &winning).unwrap_err();
    assert!(
        matches!(err, FollowerError::ReorgExceedsHistory),
        "got {err:?}"
    );
    assert_eq!(state.checkpoint_bytes().unwrap(), before);
}

// ---------------------------------------------------------------------------
// JSON-RPC client over a loopback fake server.
// ---------------------------------------------------------------------------

fn temp_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tradelayer-follower-{tag}-{}-{n}.tljc",
        std::process::id()
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn http(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads one request and returns its `Authorization` value and body text.
fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut pending = Vec::new();
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(position) = find(&pending, b"\r\n\r\n") {
            break position;
        }
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            return (String::new(), String::new());
        }
        pending.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&pending[..end]).into_owned();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let auth = head
        .lines()
        .find_map(|line| line.strip_prefix("Authorization: "))
        .unwrap_or("")
        .to_owned();
    let mut body = pending[end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    (auth, String::from_utf8_lossy(&body).into_owned())
}

/// Serves each accepted connection with `responder(authorization, body)`.
fn serve<F>(responder: F) -> SocketAddr
where
    F: Fn(&str, &str) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (auth, body) = read_request(&mut stream);
            let _ = stream.write_all(&responder(&auth, &body));
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    addr
}

fn creds() -> RpcCredentials {
    RpcCredentials::new(RPC_USER, RPC_PASSWORD).unwrap()
}

fn rpc_to(addr: SocketAddr) -> RpcBlockSource {
    let config = RpcConfig::new(addr, creds(), EndpointPolicy::LoopbackOnly)
        .unwrap()
        .with_timeouts(Duration::from_secs(2), Duration::from_secs(2))
        .unwrap();
    RpcBlockSource::new(config)
}

/// Answers every request with the same bytes and returns the client error.
fn error_for_fixed_reply(reply: Vec<u8>) -> SourceError {
    let addr = serve(move |_, _| reply.clone());
    rpc_to(addr).best_height().unwrap_err()
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text.find(start).map_or(0, |i| i + start.len());
    let rest = &text[from..];
    let to = rest.find(end).unwrap_or(rest.len());
    &rest[..to]
}

fn unhex32(text: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// A JSON-RPC responder backed by a mock chain. It rejects wrong credentials.
fn mock_responder(
    source: MockBlockSource,
    expected_auth: String,
) -> impl Fn(&str, &str) -> Vec<u8> + Send {
    move |auth, body| {
        if auth != expected_auth {
            return http(401, "{\"result\":null,\"error\":null,\"id\":1}");
        }
        let method = between(body, "\"method\":\"", "\"");
        let params = between(body, "\"params\":", "}");
        let result = match method {
            "getblockcount" => Ok(source.tip_height().to_string()),
            "getblockhash" => {
                let height = between(params, "[", "]").parse::<u64>().unwrap_or(u64::MAX);
                source
                    .block_hash_at(height)
                    .map(|id| format!("\"{}\"", hex(&id.0)))
            }
            "getblock" => {
                let hash = unhex32(between(params, "[\"", "\""));
                source
                    .raw_block(BlockId(hash))
                    .map(|raw| format!("\"{}\"", hex(&raw)))
            }
            _ => Err(SourceError::Mock("unsupported method")),
        };
        match result {
            Ok(value) => http(
                200,
                &format!("{{\"result\":{value},\"error\":null,\"id\":1}}"),
            ),
            Err(_) => http(
                500,
                "{\"result\":null,\"error\":{\"code\":-5,\"message\":\"x\"},\"id\":1}",
            ),
        }
    }
}

#[test]
fn rpc_source_drives_follower_end_to_end_with_basic_auth() {
    let network = TradeLayerMainnet::Bitcoin;
    let mut source = common_prefix(network);
    source
        .push_block(vec![synthetic_transaction(2, 100)], 4)
        .unwrap();
    source
        .push_block(vec![synthetic_transaction(3, 101)], 5)
        .unwrap();
    source
        .push_block(vec![synthetic_transaction(2, 102)], 6)
        .unwrap();
    let addr = serve(mock_responder(source.clone(), EXPECTED_BASIC.to_owned()));
    let rpc = rpc_to(addr);

    let mut state = fresh(network);
    let report = sync(&mut state, &rpc).unwrap();
    assert_eq!(report.applied, 6);
    assert_eq!(
        snapshot(&state),
        snapshot(&straight_replay(&source, network))
    );
}

#[test]
fn rpc_rejects_wrong_credentials_as_unauthorized() {
    let source = common_prefix(TradeLayerMainnet::Bitcoin);
    let addr = serve(mock_responder(source, "Basic d3Jvbmc=".to_owned()));
    let err = rpc_to(addr).best_height().unwrap_err();
    assert_eq!(err, SourceError::Unauthorized);
}

#[test]
fn rpc_rejects_short_body_and_chunked_encoding() {
    let short = b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n{\"result\":1}".to_vec();
    assert_eq!(error_for_fixed_reply(short), SourceError::ShortResponse);
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    assert!(matches!(
        error_for_fixed_reply(chunked),
        SourceError::MalformedHttp(_)
    ));
}

#[test]
fn rpc_rejects_oversized_content_length_before_reading_body() {
    let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\n\r\n".to_vec();
    assert_eq!(error_for_fixed_reply(reply), SourceError::ResponseTooLarge);
}

#[test]
fn rpc_rejects_lying_or_malformed_envelopes() {
    let bad_hash = http(200, "{\"result\":\"abcd\",\"error\":null,\"id\":1}");
    let addr = serve(move |_, _| bad_hash.clone());
    let err = rpc_to(addr).block_hash_at(1).unwrap_err();
    assert!(
        matches!(err, SourceError::UnexpectedResult(_)),
        "got {err:?}"
    );

    assert_eq!(
        error_for_fixed_reply(http(200, "{\"result\":1,\"error\":null,\"id\":2}")),
        SourceError::RpcIdMismatch
    );
    assert!(matches!(
        error_for_fixed_reply(http(200, "<html>not json</html>")),
        SourceError::MalformedJson(_)
    ));
    assert_eq!(
        error_for_fixed_reply(http(
            500,
            "{\"result\":null,\"error\":{\"code\":-8,\"message\":\"Block height out of range\"},\"id\":1}"
        )),
        SourceError::RpcError(-8)
    );
    assert_eq!(
        error_for_fixed_reply(http(403, "{}")),
        SourceError::HttpStatus(403)
    );
}

#[test]
fn rpc_rejects_raw_block_hex_beyond_the_unverified_bound() {
    // 8,000,002 hex characters decode to 4,000,001 bytes, one above the cap.
    // The envelope stays under the 8 MiB HTTP bound, so only the block bound fires.
    let big = format!("\"{}\"", "0".repeat(8_000_002));
    let addr = serve(move |_, _| {
        http(
            200,
            &format!("{{\"result\":{big},\"error\":null,\"id\":1}}"),
        )
    });
    let err = rpc_to(addr).raw_block(BlockId([1; 32])).unwrap_err();
    assert_eq!(err, SourceError::BlockTooLarge);
}

#[test]
fn rpc_times_out_on_a_silent_node() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            thread::sleep(Duration::from_secs(3));
            drop(stream);
        }
    });
    let config = RpcConfig::new(addr, creds(), EndpointPolicy::LoopbackOnly)
        .unwrap()
        .with_timeouts(Duration::from_secs(2), Duration::from_millis(200))
        .unwrap();
    let err = RpcBlockSource::new(config).best_height().unwrap_err();
    assert_eq!(err, SourceError::Timeout);
}

#[test]
fn rpc_reports_refused_connections_without_address_or_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let err = rpc_to(addr).best_height().unwrap_err();
    assert_eq!(err, SourceError::Io(std::io::ErrorKind::ConnectionRefused));
}

#[test]
fn remote_endpoint_and_missing_env_are_refused_with_safe_messages() {
    let remote: SocketAddr = "10.20.30.40:8332".parse().unwrap();
    let err = RpcConfig::new(remote, creds(), EndpointPolicy::LoopbackOnly).unwrap_err();
    assert_eq!(err, SourceError::NonLoopbackRefused);
    assert!(RpcConfig::new(remote, creds(), EndpointPolicy::ExplicitRemoteOptIn).is_ok());

    let err = RpcCredentials::from_env_vars(
        "TL_FOLLOWER_TEST_UNSET_USER_7731",
        "TL_FOLLOWER_TEST_UNSET_PASS_7731",
    )
    .unwrap_err();
    assert_eq!(
        err,
        SourceError::MissingCredential("TL_FOLLOWER_TEST_UNSET_USER_7731")
    );
}

#[test]
fn credentials_load_from_environment_and_stay_redacted() {
    std::env::set_var("TL_FOLLOWER_TEST_USER_8812", RPC_USER);
    std::env::set_var("TL_FOLLOWER_TEST_PASS_8812", RPC_PASSWORD);
    let loaded =
        RpcCredentials::from_env_vars("TL_FOLLOWER_TEST_USER_8812", "TL_FOLLOWER_TEST_PASS_8812")
            .unwrap();
    let addr = serve(mock_responder(
        common_prefix(TradeLayerMainnet::Bitcoin),
        EXPECTED_BASIC.to_owned(),
    ));
    let config = RpcConfig::new(addr, loaded, EndpointPolicy::LoopbackOnly).unwrap();
    let rendered = format!("{config:?}");
    assert!(!rendered.contains(RPC_PASSWORD));
    assert!(!rendered.contains(RPC_USER));
    assert_eq!(RpcBlockSource::new(config).best_height().unwrap(), 3);
}

#[test]
fn credentials_never_appear_in_any_error_text() {
    let mut messages: Vec<String> = Vec::new();
    let mut record = |err: &SourceError| {
        messages.push(err.to_string());
        messages.push(format!("{err:?}"));
    };
    let replies = [
        b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\n\r\n".to_vec(),
        http(401, "{}"),
        http(
            500,
            "{\"result\":null,\"error\":{\"code\":-28,\"message\":\"warming up\"},\"id\":1}",
        ),
        http(200, "not json"),
        http(200, "{\"result\":1,\"error\":null,\"id\":9}"),
    ];
    for reply in replies {
        record(&error_for_fixed_reply(reply));
    }
    let remote: SocketAddr = "10.9.8.7:8332".parse().unwrap();
    record(&RpcConfig::new(remote, creds(), EndpointPolicy::LoopbackOnly).unwrap_err());
    record(&SourceError::MissingCredential("TRADELAYER_RPC_PASSWORD"));

    let base64_of_pair = EXPECTED_BASIC.trim_start_matches("Basic ");
    for text in &messages {
        assert!(!text.contains(RPC_PASSWORD), "password leaked: {text}");
        assert!(!text.contains(RPC_USER), "user leaked: {text}");
        assert!(!text.contains("SECRET"), "secret fragment leaked: {text}");
        assert!(!text.contains(base64_of_pair), "basic token leaked: {text}");
    }
    assert!(messages.len() >= 10);
}
