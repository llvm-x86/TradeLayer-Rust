//! Stage 23C read-only Litecoin evidence authentication and gate.
//!
//! Real data: the two pinned Litecoin genesis headers, the Bitcoin genesis
//! header, and the testnet admin address string from tradelayer.js@dca0888b.
//! Address vectors were produced by an independent Python implementation.
//! Every block and transaction below that is not a genesis header is
//! SYNTHETIC (no proof of work, never broadcast).

use std::collections::BTreeSet;
use tradelayer_core::litecoin_evidence::{
    authenticate_block, display_order, is_nulldata, merkle_branch, merkle_root, script_address,
    stage23c_gate, verify_merkle_position, AttestedHeaderChain, CandidateEvidence, EvidenceError,
    GateBlocker, InclusionProof, LitecoinChain, ParentEvidence, ParentMembership,
    ParityReportSummary, RawHeader, ScriptAddress, LITECOIN_TESTNET_GENESIS,
};
use tradelayer_core::network_profile::{TradeLayerMainnet, LITECOIN_MAINNET_GENESIS};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

const ADMIN_TESTNET: &str = "tltc1qa0kd2d39nmeph3hvcx8ytv65ztcywg5sazhtw8";
const ADMIN_PROGRAM: &str = "ebecd536259ef21bc6ecc18e45b35412f0472290";

fn genesis_headers() -> Vec<(String, Vec<u8>, String)> {
    include_str!("../fixtures/litecoin_genesis_headers.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (
                f[0].to_string(),
                decode_hex(f[1]).unwrap(),
                f[2].to_string(),
            )
        })
        .collect()
}

fn hex_id(raw: [u8; 32]) -> String {
    display_order(raw)
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn pinned_genesis_headers_anchor_each_chain_and_reject_others() {
    let headers = genesis_headers();
    let header =
        |net: &str| RawHeader::from_bytes(&headers.iter().find(|h| h.0 == net).unwrap().1).unwrap();
    for (net, bytes, expected) in &headers {
        assert_eq!(
            hex_id(RawHeader::from_bytes(bytes).unwrap().hash()),
            *expected,
            "{net}"
        );
    }
    assert_eq!(
        hex_id(header("main").hash()),
        hex_id({
            let mut raw = LITECOIN_MAINNET_GENESIS.0;
            raw.reverse();
            raw
        })
    );
    assert_eq!(LitecoinChain::Testnet.genesis(), LITECOIN_TESTNET_GENESIS);
    for (chain, own, other) in [
        (LitecoinChain::Mainnet, "main", "test"),
        (LitecoinChain::Testnet, "test", "main"),
    ] {
        assert!(AttestedHeaderChain::from_headers(chain, 0, None, vec![header(own)]).is_ok());
        assert_eq!(
            AttestedHeaderChain::from_headers(chain, 0, None, vec![header(other)]),
            Err(EvidenceError::HeaderChain(
                "height 0 is not this chain's genesis"
            ))
        );
        let bitcoin = include_str!("../fixtures/bitcoin_mainnet_genesis_block1_headers.txt")
            .lines()
            .next()
            .unwrap();
        let bitcoin = RawHeader::from_bytes(&decode_hex(bitcoin).unwrap()).unwrap();
        assert!(AttestedHeaderChain::from_headers(chain, 0, None, vec![bitcoin]).is_err());
    }
    assert!(RawHeader::from_bytes(&[0; 79]).is_err());
}

#[test]
fn script_addresses_match_independent_vectors_and_pinned_admin() {
    let script = |h: &str| decode_hex(h).unwrap();
    let single = |c, h: &str| match script_address(c, &script(h)) {
        ScriptAddress::Single(a) => a,
        other => panic!("{other:?}"),
    };
    use LitecoinChain::{Mainnet, Testnet};
    assert_eq!(
        single(Testnet, &format!("0014{ADMIN_PROGRAM}")),
        ADMIN_TESTNET
    );
    let h11 = "11".repeat(20);
    let h22 = "22".repeat(20);
    assert_eq!(
        single(Mainnet, &format!("76a914{h11}88ac")),
        "LLnCCHbSzfwWquEdaS5TF2Yt7uz5Qb1SZ1"
    );
    assert_eq!(
        single(Testnet, &format!("76a914{h11}88ac")),
        "mh5CE8Nbj38iND267s4XnvhSmhDW7yWc6Q"
    );
    assert_eq!(
        single(Mainnet, &format!("a914{h22}87")),
        "MB1e6aUeL3Zj4s4H2ZqFBHaaHd7kvvzTco"
    );
    assert_eq!(
        single(Testnet, &format!("a914{h22}87")),
        "QPiTySrx1VGjcLAyDvVo4HksKfBJbX21RQ"
    );
    assert_eq!(
        single(Mainnet, &format!("0020{}", "33".repeat(32))),
        "ltc1qxvenxvenxvenxvenxvenxvenxvenxvenxvenxvenxvenxvenxvesm8s4gw"
    );
    assert_eq!(
        single(Testnet, &format!("5120{}", "44".repeat(32))),
        "tltc1pg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zq0m6pne"
    );
    let p2pk = format!("21{}ac", "02".repeat(33));
    assert_eq!(
        script_address(Mainnet, &script(&p2pk)),
        ScriptAddress::NoAddress("P2PK")
    );
    let multisig = format!("5121{}21{}52ae", "02".repeat(33), "03".repeat(33));
    assert!(matches!(
        script_address(Mainnet, &script(&multisig)),
        ScriptAddress::Ambiguous(_)
    ));
    assert_eq!(
        script_address(Mainnet, &script("6a02746c")),
        ScriptAddress::NoAddress("nulldata")
    );
    assert!(is_nulldata(&script("6a")) && is_nulldata(&script("6a4c02746c51")));
    assert!(!is_nulldata(&script("6a746c")) && !is_nulldata(&script("6a05746c")));
    assert!(matches!(
        script_address(Mainnet, &script("6a746c")),
        ScriptAddress::NoAddress(_)
    ));
}

#[test]
fn merkle_position_is_bound_not_just_membership() {
    for size in 1..12u32 {
        let leaves: Vec<[u8; 32]> = (0..size).map(|i| [i as u8 + 1; 32]).collect();
        let root = merkle_root(&leaves).unwrap();
        for (i, leaf) in leaves.iter().enumerate() {
            let branch = merkle_branch(&leaves, i).unwrap();
            verify_merkle_position(*leaf, &branch, i as u32, size, root).unwrap();
            if size > 1 {
                assert!(verify_merkle_position(*leaf, &branch, i as u32 ^ 1, size, root).is_err());
            }
        }
    }
    assert!(merkle_root(&[]).is_none());
    assert!(verify_merkle_position([1; 32], &[], 0, 0, [1; 32]).is_err());
}

// ---- SYNTHETIC two-block segment -----------------------------------------

fn tx(inputs: &[([u8; 32], u32)], outputs: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut raw = vec![2, 0, 0, 0, inputs.len() as u8];
    for (prev, vout) in inputs {
        raw.extend_from_slice(prev);
        raw.extend_from_slice(&vout.to_le_bytes());
        raw.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
    }
    raw.push(outputs.len() as u8);
    for (value, script) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.push(script.len() as u8);
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&[0; 4]);
    raw
}

fn hogex(outputs: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let plain = tx(&[([9; 32], 0)], outputs);
    [
        &plain[..4],
        &[0, 8],
        &plain[4..plain.len() - 4],
        &[0],
        &[0; 4],
    ]
    .concat()
}

fn txid(raw: &[u8]) -> [u8; 32] {
    inspect_transaction(raw).unwrap().txid
}

fn admin_script() -> Vec<u8> {
    decode_hex(&format!("0014{ADMIN_PROGRAM}")).unwrap()
}

fn recipient_script() -> Vec<u8> {
    decode_hex(&format!("0014{}", "55".repeat(20))).unwrap()
}

fn opreturn(payload: &[u8]) -> Vec<u8> {
    [&[0x6a, payload.len() as u8][..], payload].concat()
}

struct Segment {
    chain: AttestedHeaderChain,
    blocks: Vec<Vec<Vec<u8>>>,
}

const START: u64 = 3_082_499;

fn segment(block_txs: Vec<Vec<Vec<u8>>>) -> Segment {
    let mut prev = [0x77; 32];
    let mut headers = Vec::new();
    for txs in &block_txs {
        let ids: Vec<[u8; 32]> = txs.iter().map(|t| txid(t)).collect();
        let header = RawHeader::from_fields(
            0x2000_0000,
            prev,
            merkle_root(&ids).unwrap(),
            1,
            0x1e0f_fff0,
            0,
        );
        prev = header.hash();
        headers.push(header);
    }
    Segment {
        chain: AttestedHeaderChain::from_headers(
            LitecoinChain::Testnet,
            START,
            Some([0x77; 32]),
            headers,
        )
        .unwrap(),
        blocks: block_txs,
    }
}

impl Segment {
    fn proof(&self, block: usize, position: usize) -> InclusionProof {
        let ids: Vec<[u8; 32]> = self.blocks[block].iter().map(|t| txid(t)).collect();
        InclusionProof {
            header: self.chain.header_at(START + block as u64).unwrap().clone(),
            branch: merkle_branch(&ids, position).unwrap(),
            position: position as u32,
            tx_count: ids.len() as u32,
        }
    }

    fn candidate(&self, position: usize, parent: (usize, usize)) -> CandidateEvidence {
        CandidateEvidence {
            raw_tx: self.blocks[1][position].clone(),
            inclusion: self.proof(1, position),
            parent: Some(ParentEvidence {
                raw_tx: self.blocks[parent.0][parent.1].clone(),
                height: START + parent.0 as u64,
                inclusion: self.proof(parent.0, parent.1),
            }),
        }
    }
}

fn standard() -> Segment {
    let coinbase0 = tx(&[([0; 32], u32::MAX)], &[(1, admin_script())]);
    let funding = tx(
        &[([0xab; 32], 0)],
        &[(100_000, admin_script()), (100_001, admin_script())],
    );
    let p2pk = tx(
        &[([0xac; 32], 0)],
        &[(7, decode_hex(&format!("21{}ac", "02".repeat(33))).unwrap())],
    );
    let fid = txid(&funding);
    let coinbase1 = tx(&[([0; 32], u32::MAX)], &[(1, recipient_script())]);
    let activate = tx(
        &[(fid, 0)],
        &[(0, opreturn(b"tl01;2,1")), (546, recipient_script())],
    );
    let issue = tx(
        &[(fid, 1)],
        &[
            (0, opreturn(b"tl1rs,SYN,,0,,0,0,")),
            (546, recipient_script()),
        ],
    );
    let from_p2pk = tx(
        &[(txid(&p2pk), 0)],
        &[(546, recipient_script()), (0, opreturn(b"tl0"))],
    );
    segment(vec![
        vec![coinbase0, funding, p2pk],
        vec![coinbase1, activate, issue, from_p2pk],
    ])
}

#[test]
fn authenticates_block_and_derives_actor_from_parent_bytes() {
    let s = standard();
    let out = authenticate_block(
        &s.chain,
        START + 1,
        &[s.candidate(1, (0, 1)), s.candidate(2, (0, 1))],
    )
    .unwrap();
    assert_eq!(out.len(), 2);
    let first = &out[0];
    assert_eq!(first.height(), START + 1);
    assert_eq!(first.position(), 1);
    assert_eq!(first.txid(), txid(&s.blocks[1][1]));
    assert_eq!(
        first.sender_address(),
        &ScriptAddress::Single(ADMIN_TESTNET.into())
    );
    assert_eq!(first.sender_value(), 100_000);
    assert_eq!(first.parent_membership(), ParentMembership::InAttestedChain);
    assert_eq!(first.carriers().len(), 1);
    assert_eq!(first.carriers()[0].transaction_type, Some(0));
    assert_eq!(first.reference_vout_0_1().len(), 1);
    assert_eq!(first.reference_vout_0_1()[0].vout, 1);
    assert_eq!(first.last_non_nulldata().unwrap().vout, 1);
    assert!(first.actor_resolved());
    assert_eq!(out[1].sender_value(), 100_001);
    assert_eq!(first.chain(), LitecoinChain::Testnet);
    assert_eq!(
        first.block_id(),
        display_order(s.chain.header_at(START + 1).unwrap().hash())
    );
}

#[test]
fn p2pk_parent_leaves_actor_unresolved() {
    let s = standard();
    let out = authenticate_block(&s.chain, START + 1, &[s.candidate(3, (0, 2))]).unwrap();
    assert_eq!(out[0].sender_address(), &ScriptAddress::NoAddress("P2PK"));
    assert!(!out[0].actor_resolved());
    // Reference rules differ here: vout 0 is addressed, vout 1 is nulldata.
    assert_eq!(out[0].reference_vout_0_1()[0].vout, 0);
    assert_eq!(out[0].last_non_nulldata().unwrap().vout, 0);
}

#[test]
fn adversarial_reorg_and_competing_headers_are_rejected() {
    let s = standard();
    let mut competing = s.candidate(1, (0, 1));
    competing.inclusion.header = RawHeader::from_fields(
        0x2000_0000,
        s.chain.header_at(START).unwrap().hash(),
        s.chain.header_at(START + 1).unwrap().merkle_root(),
        2,
        0x1e0f_fff0,
        0,
    );
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[competing]),
        Err(EvidenceError::NotInAttestedChain)
    );
    let mut stale_parent = s.candidate(1, (0, 1));
    stale_parent.parent.as_mut().unwrap().inclusion.header = RawHeader::from_fields(
        1,
        [0x78; 32],
        s.chain.header_at(START).unwrap().merkle_root(),
        0,
        0,
        0,
    );
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[stale_parent]),
        Err(EvidenceError::NotInAttestedChain)
    );
    assert_eq!(
        authenticate_block(&s.chain, START + 2, &[s.candidate(1, (0, 1))]),
        Err(EvidenceError::NotInAttestedChain)
    );
    let mut headers = vec![
        s.chain.header_at(START).unwrap().clone(),
        s.chain.header_at(START + 1).unwrap().clone(),
    ];
    headers.swap(0, 1);
    assert_eq!(
        AttestedHeaderChain::from_headers(LitecoinChain::Testnet, START, Some([0x77; 32]), headers),
        Err(EvidenceError::HeaderChain("header parent link broken"))
    );
    assert!(
        AttestedHeaderChain::from_headers(LitecoinChain::Testnet, START, None, vec![]).is_err()
    );
}

#[test]
fn duplicate_misordered_double_spend_and_coinbase_inputs_are_rejected() {
    let s = standard();
    let c = s.candidate(1, (0, 1));
    let mut moved = c.clone();
    moved.inclusion = s.proof(1, 2);
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[c.clone(), moved]),
        Err(EvidenceError::DuplicateCandidate)
    );
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[s.candidate(2, (0, 1)), c.clone()]),
        Err(EvidenceError::CandidateOrder)
    );
    // Two synthetic spends of one outpoint in one block.
    let fid = txid(&standard().blocks[0][1]);
    let spend_a = tx(&[(fid, 0)], &[(0, opreturn(b"tl0a"))]);
    let spend_b = tx(&[(fid, 0)], &[(0, opreturn(b"tl0b"))]);
    let s2 = segment(vec![
        s.blocks[0].clone(),
        vec![s.blocks[1][0].clone(), spend_a, spend_b],
    ]);
    assert_eq!(
        authenticate_block(
            &s2.chain,
            START + 1,
            &[s2.candidate(1, (0, 1)), s2.candidate(2, (0, 1))]
        ),
        Err(EvidenceError::DoubleSpend)
    );
    let coinbase = CandidateEvidence {
        raw_tx: s.blocks[1][0].clone(),
        inclusion: s.proof(1, 0),
        parent: None,
    };
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[coinbase]),
        Err(EvidenceError::Coinbase)
    );
}

#[test]
fn malformed_prior_outputs_and_lost_rpc_fail_closed() {
    let s = standard();
    let mut missing = s.candidate(1, (0, 1));
    missing.parent = None;
    assert_eq!(
        authenticate_block(&s.chain, START + 1, &[missing]),
        Err(EvidenceError::MissingParentEvidence)
    );
    let wrong_parent = s.candidate(1, (0, 2));
    assert!(matches!(
        authenticate_block(&s.chain, START + 1, &[wrong_parent]),
        Err(EvidenceError::Parent(_))
    ));
    let mut truncated = s.candidate(1, (0, 1));
    truncated.parent.as_mut().unwrap().raw_tx.pop();
    assert!(matches!(
        authenticate_block(&s.chain, START + 1, &[truncated]),
        Err(EvidenceError::Transaction(_))
    ));
    let fid = txid(&s.blocks[0][1]);
    let out_of_range = tx(&[(fid, 9)], &[(0, opreturn(b"tl0"))]);
    let s2 = segment(vec![
        s.blocks[0].clone(),
        vec![s.blocks[1][0].clone(), out_of_range],
    ]);
    assert_eq!(
        authenticate_block(&s2.chain, START + 1, &[s2.candidate(1, (0, 1))]),
        Err(EvidenceError::Parent(
            "outpoint index outside parent outputs"
        ))
    );
    let mut later = s.candidate(1, (0, 1));
    later.parent.as_mut().unwrap().height = START + 1;
    assert!(authenticate_block(&s.chain, START + 1, &[later]).is_err());
    let mut bad_branch = s.candidate(1, (0, 1));
    bad_branch.inclusion.position = 2;
    assert!(matches!(
        authenticate_block(&s.chain, START + 1, &[bad_branch]),
        Err(EvidenceError::Merkle(_))
    ));
}

#[test]
fn hogex_parent_with_mweb_flag_authenticates() {
    let s = standard();
    let peg_out = hogex(&[(5_000, admin_script())]);
    let spend = tx(
        &[(txid(&peg_out), 0)],
        &[(0, opreturn(b"tl0")), (546, recipient_script())],
    );
    let s2 = segment(vec![
        vec![s.blocks[0][0].clone(), peg_out],
        vec![s.blocks[1][0].clone(), spend],
    ]);
    let out = authenticate_block(&s2.chain, START + 1, &[s2.candidate(1, (0, 1))]).unwrap();
    assert_eq!(
        out[0].sender_address(),
        &ScriptAddress::Single(ADMIN_TESTNET.into())
    );
}

#[test]
fn parent_below_attested_segment_is_node_attested_only() {
    let s = standard();
    let only_tip = AttestedHeaderChain::from_headers(
        LitecoinChain::Testnet,
        START + 1,
        Some(s.chain.header_at(START).unwrap().hash()),
        vec![s.chain.header_at(START + 1).unwrap().clone()],
    )
    .unwrap();
    let out = authenticate_block(&only_tip, START + 1, &[s.candidate(1, (0, 1))]).unwrap();
    assert_eq!(
        out[0].parent_membership(),
        ParentMembership::NodeAttestedOnly
    );
}

fn perfect_report(digest: [u8; 32]) -> ParityReportSummary {
    ParityReportSummary {
        corpus_sha256: digest,
        prior_state_complete: true,
        evidence_accepted: 3,
        syntax_match: 3,
        actor_reference_match: 3,
        decisions_decided: 3,
        observed_types: [0u8, 1, 2].into_iter().collect(),
        ..ParityReportSummary::default()
    }
}

#[test]
fn stage23c_gate_never_authorizes_in_this_stage() {
    let digest = [7; 32];
    let unconditional: BTreeSet<GateBlocker> = [
        GateBlocker::ActivationSameBlockSemanticsDiffer,
        GateBlocker::RustDecisionReplayMissing,
    ]
    .into_iter()
    .collect();
    assert_eq!(
        stage23c_gate(
            LitecoinChain::Mainnet,
            digest,
            Some(&perfect_report(digest))
        ),
        unconditional
    );
    assert_eq!(
        LitecoinChain::Mainnet.tradelayer_instance(),
        Some(TradeLayerMainnet::Litecoin)
    );
    let testnet = stage23c_gate(LitecoinChain::Testnet, digest, None);
    assert!(testnet.contains(&GateBlocker::NoTradeLayerInstance));
    assert!(testnet.contains(&GateBlocker::NoParityReport));
    assert!(testnet.is_superset(&unconditional));
}

#[test]
fn stage23c_gate_reports_each_parity_gap() {
    let digest = [7; 32];
    let mut r = perfect_report(digest);
    r.corpus_sha256 = [8; 32];
    r.prior_state_complete = false;
    r.evidence_inconclusive = 1;
    r.syntax_other = 1;
    r.actor_reference_other = 1;
    r.decisions_other = 1;
    r.order_divergent_blocks = 1;
    r.observed_types.extend([3, 5, 23, 31]);
    let blockers = stage23c_gate(LitecoinChain::Mainnet, digest, Some(&r));
    for expected in [
        GateBlocker::ParityReportUnbound,
        GateBlocker::PriorStateIncomplete,
        GateBlocker::EvidenceNotAccepted,
        GateBlocker::SyntaxParityNotExact,
        GateBlocker::ActorReferenceParityNotExact,
        GateBlocker::DecisionsNotAllDecided,
        GateBlocker::LegacyIntraBlockOrderDiverges,
        GateBlocker::ProhibitedType(3),
        GateBlocker::ProhibitedType(23),
        GateBlocker::ProhibitedType(31),
        GateBlocker::TypeOutsideOracle(5),
    ] {
        assert!(blockers.contains(&expected), "{expected:?}");
    }
    let empty = ParityReportSummary {
        corpus_sha256: digest,
        ..ParityReportSummary::default()
    };
    assert!(stage23c_gate(LitecoinChain::Mainnet, digest, Some(&empty))
        .contains(&GateBlocker::EmptyParityCorpus));
}
