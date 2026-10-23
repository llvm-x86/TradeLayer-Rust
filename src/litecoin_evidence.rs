//! Stage 23C boundary: authenticate Litecoin evidence bytes; gate joint replay.
//!
//! Every replay-relevant field is derived here from raw bytes: the txid, the
//! canonical block position (header + Merkle branch + transaction count),
//! the first-input outpoint, the spent parent output and its script-derived
//! address, and both legacy reference outputs. No caller supplies an actor,
//! a height or an input ID. Addresses follow Litecoin Core's DestinationEncoder
//! (src/key_io.cpp at ec1b6489); they are output-script descriptors, not
//! signer proofs.
//!
//! Chain membership is only as strong as the local node's attested header
//! chain: parent links are checked here, Scrypt work, retargeting and
//! chainwork are not. The gate below lists what still blocks feeding
//! Litecoin's JointChainState. In this stage it always reports
//! `RustDecisionReplayMissing`, and nothing converts an
//! `AuthenticatedObservation` into a joint transaction.

use crate::chain::BlockId;
use crate::hash256::double_sha256;
use crate::network_profile::{TradeLayerMainnet, LITECOIN_MAINNET_GENESIS};
use crate::read_only::{inspect_transaction, Carrier, OutPoint, ScanError};
use std::collections::BTreeSet;

/// Litecoin Core src/chainparams.cpp at ec1b6489 (testnet, data dir "testnet4").
pub const LITECOIN_TESTNET_GENESIS: BlockId = BlockId([
    0x49, 0x66, 0x62, 0x5a, 0x4b, 0x28, 0x51, 0xd9, 0xfd, 0xee, 0x13, 0x9e, 0x56, 0x21, 0x1a, 0x0d,
    0x88, 0x57, 0x5f, 0x59, 0xed, 0x81, 0x6f, 0xf5, 0xe6, 0xa6, 0x3d, 0xeb, 0x4e, 0x3e, 0x29, 0xa0,
]);

/// Upper bound on headers held in one attested chain segment.
pub const MAX_ATTESTED_HEADERS: usize = 50_000;
/// Litecoin legacy TradeLayer transaction types that stay denied.
pub const PROHIBITED_TYPES: [u8; 3] = [3, 23, 31];
/// Types the bounded legacy JS decision oracle compares.
pub const ORACLE_TYPES: [u8; 5] = [0, 1, 2, 11, 12];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LitecoinChain {
    Mainnet,
    Testnet,
}

impl LitecoinChain {
    pub const fn genesis(self) -> BlockId {
        match self {
            Self::Mainnet => LITECOIN_MAINNET_GENESIS,
            Self::Testnet => LITECOIN_TESTNET_GENESIS,
        }
    }

    /// Only Litecoin mainnet has a TradeLayer instance (Stage 21 profiles).
    pub const fn tradelayer_instance(self) -> Option<TradeLayerMainnet> {
        match self {
            Self::Mainnet => Some(TradeLayerMainnet::Litecoin),
            Self::Testnet => None,
        }
    }

    const fn p2pkh_version(self) -> u8 {
        match self {
            Self::Mainnet => 48,
            Self::Testnet => 111,
        }
    }

    /// Core encodes P2SH with SCRIPT_ADDRESS2.
    const fn p2sh_version(self) -> u8 {
        match self {
            Self::Mainnet => 50,
            Self::Testnet => 58,
        }
    }

    const fn hrp(self) -> &'static str {
        match self {
            Self::Mainnet => "ltc",
            Self::Testnet => "tltc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    Transaction(ScanError),
    Header(&'static str),
    HeaderChain(&'static str),
    Merkle(&'static str),
    /// The candidate's block is not the attested header at that height.
    NotInAttestedChain,
    Coinbase,
    Parent(&'static str),
    DuplicateCandidate,
    CandidateOrder,
    DoubleSpend,
    /// The parent output was not supplied (e.g. RPC lost): INCONCLUSIVE.
    MissingParentEvidence,
}

impl core::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EvidenceError {}

impl From<ScanError> for EvidenceError {
    fn from(value: ScanError) -> Self {
        Self::Transaction(value)
    }
}

/// Raw-order (hashing order) digests are reversed for display/BlockId use.
pub fn display_order(raw: [u8; 32]) -> BlockId {
    let mut out = raw;
    out.reverse();
    BlockId(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHeader([u8; 80]);

impl RawHeader {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EvidenceError> {
        let fixed: [u8; 80] = bytes
            .try_into()
            .map_err(|_| EvidenceError::Header("header must be exactly 80 bytes"))?;
        Ok(Self(fixed))
    }

    /// Assemble a header; `prev` and `merkle_root` are in raw hashing order.
    pub fn from_fields(
        version: u32,
        prev: [u8; 32],
        merkle_root: [u8; 32],
        time: u32,
        bits: u32,
        nonce: u32,
    ) -> Self {
        let mut out = [0u8; 80];
        out[0..4].copy_from_slice(&version.to_le_bytes());
        out[4..36].copy_from_slice(&prev);
        out[36..68].copy_from_slice(&merkle_root);
        out[68..72].copy_from_slice(&time.to_le_bytes());
        out[72..76].copy_from_slice(&bits.to_le_bytes());
        out[76..80].copy_from_slice(&nonce.to_le_bytes());
        Self(out)
    }

    /// Block identity hash (SHA256d); Litecoin's PoW hash is Scrypt and is
    /// not checked here.
    pub fn hash(&self) -> [u8; 32] {
        double_sha256(&self.0)
    }

    pub fn prev(&self) -> [u8; 32] {
        self.0[4..36].try_into().expect("fixed header slice")
    }

    pub fn merkle_root(&self) -> [u8; 32] {
        self.0[36..68].try_into().expect("fixed header slice")
    }

    pub fn bytes(&self) -> &[u8; 80] {
        &self.0
    }
}

fn pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut joined = [0u8; 64];
    joined[..32].copy_from_slice(left);
    joined[32..].copy_from_slice(right);
    double_sha256(&joined)
}

/// Bitcoin-family txid Merkle root (raw order); None for an empty list.
pub fn merkle_root(leaves: &[[u8; 32]]) -> Option<[u8; 32]> {
    let mut level = leaves.to_vec();
    if level.is_empty() {
        return None;
    }
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().expect("nonempty"));
        }
        level = level.chunks(2).map(|p| pair(&p[0], &p[1])).collect();
    }
    Some(level[0])
}

/// Sibling hashes proving `leaves[index]`.
pub fn merkle_branch(leaves: &[[u8; 32]], mut index: usize) -> Option<Vec<[u8; 32]>> {
    if index >= leaves.len() {
        return None;
    }
    let mut level = leaves.to_vec();
    let mut branch = Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().expect("nonempty"));
        }
        branch.push(level[index ^ 1]);
        level = level.chunks(2).map(|p| pair(&p[0], &p[1])).collect();
        index /= 2;
    }
    Some(branch)
}

/// Binds a canonical position, not just membership. The last element of an
/// odd level pairs with itself, so every self-pairing must sit on the right
/// edge of a level whose width follows from `tx_count` (node-reported; for a
/// leaf that never self-pairs a duplicated-tail mutation keeps both root and
/// position, so the position stays canonical).
pub fn verify_merkle_position(
    leaf: [u8; 32],
    branch: &[[u8; 32]],
    index: u32,
    tx_count: u32,
    root: [u8; 32],
) -> Result<(), EvidenceError> {
    if tx_count == 0 || index >= tx_count {
        return Err(EvidenceError::Merkle("position outside block"));
    }
    let depth = (u32::BITS - (tx_count - 1).leading_zeros()) as usize;
    if branch.len() != depth {
        return Err(EvidenceError::Merkle("branch depth differs from tx count"));
    }
    let (mut node, mut position, mut width) = (leaf, index, tx_count);
    for sibling in branch {
        let right_edge = position % 2 == 0 && position == width - 1;
        if *sibling == node {
            if !right_edge {
                return Err(EvidenceError::Merkle(
                    "self-paired node away from right edge",
                ));
            }
        } else if right_edge {
            return Err(EvidenceError::Merkle(
                "right-edge node must pair with itself",
            ));
        }
        node = if position % 2 == 1 {
            pair(sibling, &node)
        } else {
            pair(&node, sibling)
        };
        position /= 2;
        width = width.div_ceil(2);
    }
    if node != root {
        return Err(EvidenceError::Merkle("branch does not reach header root"));
    }
    Ok(())
}

/// A contiguous, parent-linked header segment reported by the local node.
/// A segment starting above genesis rests on an UNTRUSTED `previous` hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedHeaderChain {
    chain: LitecoinChain,
    start_height: u64,
    hashes: Vec<[u8; 32]>,
    headers: Vec<RawHeader>,
}

impl AttestedHeaderChain {
    pub fn from_headers(
        chain: LitecoinChain,
        start_height: u64,
        previous: Option<[u8; 32]>,
        headers: Vec<RawHeader>,
    ) -> Result<Self, EvidenceError> {
        if headers.is_empty() || headers.len() > MAX_ATTESTED_HEADERS {
            return Err(EvidenceError::HeaderChain(
                "empty or oversized header segment",
            ));
        }
        if start_height.checked_add(headers.len() as u64).is_none() {
            return Err(EvidenceError::HeaderChain("height overflow"));
        }
        let mut expected_prev = match (start_height, previous) {
            (0, None) => {
                let mut genesis_raw = chain.genesis().0;
                genesis_raw.reverse();
                if headers[0].hash() != genesis_raw || headers[0].prev() != [0; 32] {
                    return Err(EvidenceError::HeaderChain(
                        "height 0 is not this chain's genesis",
                    ));
                }
                None
            }
            (0, Some(_)) => return Err(EvidenceError::HeaderChain("genesis has no parent")),
            (_, None) => return Err(EvidenceError::HeaderChain("segment parent hash required")),
            (_, Some(prev)) => Some(prev),
        };
        let mut hashes = Vec::with_capacity(headers.len());
        for header in &headers {
            if let Some(prev) = expected_prev {
                if header.prev() != prev {
                    return Err(EvidenceError::HeaderChain("header parent link broken"));
                }
            }
            let hash = header.hash();
            hashes.push(hash);
            expected_prev = Some(hash);
        }
        Ok(Self {
            chain,
            start_height,
            hashes,
            headers,
        })
    }

    pub const fn chain(&self) -> LitecoinChain {
        self.chain
    }

    pub fn start_height(&self) -> u64 {
        self.start_height
    }

    pub fn end_height(&self) -> u64 {
        self.start_height + self.hashes.len() as u64 - 1
    }

    pub fn header_at(&self, height: u64) -> Option<&RawHeader> {
        let offset = usize::try_from(height.checked_sub(self.start_height)?).ok()?;
        self.headers.get(offset)
    }

    fn hash_at(&self, height: u64) -> Option<[u8; 32]> {
        let offset = usize::try_from(height.checked_sub(self.start_height)?).ok()?;
        self.hashes.get(offset).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InclusionProof {
    pub header: RawHeader,
    pub branch: Vec<[u8; 32]>,
    pub position: u32,
    pub tx_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentEvidence {
    pub raw_tx: Vec<u8>,
    pub height: u64,
    pub inclusion: InclusionProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEvidence {
    pub raw_tx: Vec<u8>,
    pub inclusion: InclusionProof,
    /// None when the node could not supply it; never guessed.
    pub parent: Option<ParentEvidence>,
}

/// What Litecoin Core 0.21's RPC would list for a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptAddress {
    /// Exactly one address (legacy `addresses[0]`).
    Single(String),
    /// Several addresses (bare multisig): legacy JS silently takes the first.
    Ambiguous(&'static str),
    /// No address (P2PK, nulldata, nonstandard): legacy getSender returns Error.
    NoAddress(&'static str),
}

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BECH32: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn base58check(version: u8, payload: &[u8]) -> String {
    let mut data = Vec::with_capacity(payload.len() + 5);
    data.push(version);
    data.extend_from_slice(payload);
    let check = double_sha256(&data);
    data.extend_from_slice(&check[..4]);
    let zeros = data.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for byte in &data {
        let mut carry = u32::from(*byte);
        for digit in digits.iter_mut() {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = "1".repeat(zeros);
    out.extend(digits.iter().rev().map(|d| BASE58[*d as usize] as char));
    out
}

fn bech32_polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(*v);
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn segwit_address(hrp: &str, version: u8, program: &[u8]) -> String {
    let mut data = vec![version];
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in program {
        acc = (acc << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            data.push(((acc >> bits) & 31) as u8);
        }
    }
    if bits > 0 {
        data.push(((acc << (5 - bits)) & 31) as u8);
    }
    let mut values: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|c| c & 31));
    values.extend_from_slice(&data);
    values.extend_from_slice(&[0; 6]);
    // BIP173 for v0, BIP350 (bech32m) for v1+, as in key_io.cpp.
    let constant = if version == 0 { 1 } else { 0x2bc8_30a3 };
    let polymod = bech32_polymod(&values) ^ constant;
    let mut out = String::from(hrp);
    out.push('1');
    for d in data {
        out.push(BECH32[d as usize] as char);
    }
    for i in 0..6 {
        out.push(BECH32[((polymod >> (5 * (5 - i))) & 31) as usize] as char);
    }
    out
}

fn is_push_only(script: &[u8]) -> bool {
    let mut i = 0usize;
    while i < script.len() {
        let op = script[i];
        i += 1;
        if op > 0x60 {
            return false;
        }
        let size = match op {
            0x01..=0x4b => usize::from(op),
            0x4c..=0x4e => {
                let width = [1usize, 2, 4][usize::from(op - 0x4c)];
                let Some(bytes) = script.get(i..i + width) else {
                    return false;
                };
                i += width;
                bytes
                    .iter()
                    .rev()
                    .fold(0usize, |acc, b| (acc << 8) | usize::from(*b))
            }
            _ => 0,
        };
        let Some(end) = i.checked_add(size) else {
            return false;
        };
        if end > script.len() {
            return false;
        }
        i = end;
    }
    true
}

/// Core's TxoutType::NULL_DATA: OP_RETURN followed only by pushes.
pub fn is_nulldata(script: &[u8]) -> bool {
    script.first() == Some(&0x6a) && is_push_only(&script[1..])
}

/// Script-derived address per Litecoin Core 0.21 ScriptPubKeyToUniv.
pub fn script_address(chain: LitecoinChain, script: &[u8]) -> ScriptAddress {
    match script {
        [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] if hash.len() == 20 => {
            ScriptAddress::Single(base58check(chain.p2pkh_version(), hash))
        }
        [0xa9, 0x14, hash @ .., 0x87] if hash.len() == 20 => {
            ScriptAddress::Single(base58check(chain.p2sh_version(), hash))
        }
        [0x00, 0x14, program @ ..] if program.len() == 20 => {
            ScriptAddress::Single(segwit_address(chain.hrp(), 0, program))
        }
        [0x00, 0x20, program @ ..] if program.len() == 32 => {
            ScriptAddress::Single(segwit_address(chain.hrp(), 0, program))
        }
        [op @ 0x51..=0x60, len, program @ ..]
            if usize::from(*len) == program.len() && (2..=40).contains(&program.len()) =>
        {
            ScriptAddress::Single(segwit_address(chain.hrp(), op - 0x50, program))
        }
        [0x21, key @ .., 0xac] if key.len() == 33 => ScriptAddress::NoAddress("P2PK"),
        [0x41, key @ .., 0xac] if key.len() == 65 => ScriptAddress::NoAddress("P2PK"),
        [m @ 0x51..=0x60, .., n @ 0x51..=0x60, 0xae] if m <= n => {
            ScriptAddress::Ambiguous("bare multisig lists one address per key")
        }
        _ if is_nulldata(script) => ScriptAddress::NoAddress("nulldata"),
        _ => ScriptAddress::NoAddress("nonstandard or unsupported script"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputView {
    pub vout: u32,
    pub value: u64,
    pub nulldata: bool,
    pub address: ScriptAddress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentMembership {
    /// The parent's header is the attested header at its height.
    InAttestedChain,
    /// Below the attested segment: inclusion checked, membership node-attested.
    NodeAttestedOnly,
}

/// Read-only. Constructed only by `authenticate_block`; fields have no setters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedObservation {
    chain: LitecoinChain,
    height: u64,
    block_hash: [u8; 32],
    position: u32,
    txid: [u8; 32],
    carriers: Vec<Carrier>,
    first_input: OutPoint,
    sender_value: u64,
    sender_address: ScriptAddress,
    parent_membership: ParentMembership,
    outputs: Vec<OutputView>,
}

impl AuthenticatedObservation {
    pub const fn chain(&self) -> LitecoinChain {
        self.chain
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn block_id(&self) -> BlockId {
        display_order(self.block_hash)
    }
    pub fn position(&self) -> u32 {
        self.position
    }
    pub fn txid(&self) -> [u8; 32] {
        self.txid
    }
    pub fn carriers(&self) -> &[Carrier] {
        &self.carriers
    }
    pub fn first_input(&self) -> OutPoint {
        self.first_input
    }
    pub fn sender_value(&self) -> u64 {
        self.sender_value
    }
    pub fn sender_address(&self) -> &ScriptAddress {
        &self.sender_address
    }
    pub fn parent_membership(&self) -> ParentMembership {
        self.parent_membership
    }
    pub fn outputs(&self) -> &[OutputView] {
        &self.outputs
    }

    /// txIndex.processTransaction: addressed outputs at vout 0 or 1.
    pub fn reference_vout_0_1(&self) -> Vec<&OutputView> {
        self.outputs
            .iter()
            .filter(|o| o.vout <= 1 && !matches!(o.address, ScriptAddress::NoAddress(_)))
            .collect()
    }

    /// txUtils.getReference: the last non-nulldata output.
    pub fn last_non_nulldata(&self) -> Option<&OutputView> {
        self.outputs.iter().rev().find(|o| !o.nulldata)
    }

    /// Sender and every reference output resolve to exactly one address.
    pub fn actor_resolved(&self) -> bool {
        let single = |a: &ScriptAddress| matches!(a, ScriptAddress::Single(_));
        single(&self.sender_address)
            && self.reference_vout_0_1().iter().all(|o| single(&o.address))
            && self.last_non_nulldata().is_none_or(|o| single(&o.address))
    }
}

/// Authenticate every candidate claimed for one block of the attested chain.
pub fn authenticate_block(
    chain: &AttestedHeaderChain,
    height: u64,
    candidates: &[CandidateEvidence],
) -> Result<Vec<AuthenticatedObservation>, EvidenceError> {
    let block_hash = chain
        .hash_at(height)
        .ok_or(EvidenceError::NotInAttestedChain)?;
    let header = chain.header_at(height).expect("hash implies header");
    let mut seen_txids = BTreeSet::new();
    let mut seen_spends = BTreeSet::new();
    let mut last_position = None;
    let mut out = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.inclusion.header.hash() != block_hash {
            return Err(EvidenceError::NotInAttestedChain);
        }
        if last_position.is_some_and(|p| candidate.inclusion.position <= p) {
            return Err(EvidenceError::CandidateOrder);
        }
        last_position = Some(candidate.inclusion.position);
        let tx = inspect_transaction(&candidate.raw_tx)?;
        if !seen_txids.insert(tx.txid) {
            return Err(EvidenceError::DuplicateCandidate);
        }
        verify_merkle_position(
            tx.txid,
            &candidate.inclusion.branch,
            candidate.inclusion.position,
            candidate.inclusion.tx_count,
            header.merkle_root(),
        )?;
        if tx.inputs.iter().any(OutPoint::is_coinbase) {
            return Err(EvidenceError::Coinbase);
        }
        for input in &tx.inputs {
            if !seen_spends.insert(*input) {
                return Err(EvidenceError::DoubleSpend);
            }
        }
        let first_input = tx.inputs[0];
        let parent = candidate
            .parent
            .as_ref()
            .ok_or(EvidenceError::MissingParentEvidence)?;
        let parent_tx = inspect_transaction(&parent.raw_tx)?;
        if parent_tx.txid != first_input.txid {
            return Err(EvidenceError::Parent(
                "parent bytes do not hash to the outpoint txid",
            ));
        }
        let spent =
            parent_tx
                .outputs
                .get(first_input.vout as usize)
                .ok_or(EvidenceError::Parent(
                    "outpoint index outside parent outputs",
                ))?;
        verify_merkle_position(
            parent_tx.txid,
            &parent.inclusion.branch,
            parent.inclusion.position,
            parent.inclusion.tx_count,
            parent.inclusion.header.merkle_root(),
        )?;
        let earlier = parent.height < height
            || (parent.height == height
                && parent.inclusion.position < candidate.inclusion.position);
        if !earlier {
            return Err(EvidenceError::Parent(
                "parent not earlier in canonical order",
            ));
        }
        let parent_membership = match chain.hash_at(parent.height) {
            Some(hash) if hash == parent.inclusion.header.hash() => {
                ParentMembership::InAttestedChain
            }
            Some(_) => return Err(EvidenceError::NotInAttestedChain),
            None if parent.height > chain.end_height() => {
                return Err(EvidenceError::Parent("parent above attested segment"));
            }
            None => ParentMembership::NodeAttestedOnly,
        };
        let outputs = tx
            .outputs
            .iter()
            .enumerate()
            .map(|(n, o)| OutputView {
                vout: n as u32,
                value: o.value,
                nulldata: is_nulldata(&o.script),
                address: script_address(chain.chain(), &o.script),
            })
            .collect();
        out.push(AuthenticatedObservation {
            chain: chain.chain(),
            height,
            block_hash,
            position: candidate.inclusion.position,
            txid: tx.txid,
            carriers: tx.carriers,
            first_input,
            sender_value: spent.value,
            sender_address: script_address(chain.chain(), &spent.script),
            parent_membership,
            outputs,
        });
    }
    Ok(out)
}

/// Counts from legacy_js_replay.cjs and verify_litecoin_evidence.py. These
/// are caller-supplied summaries: they can only add or remove blockers,
/// never grant authority on their own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParityReportSummary {
    pub corpus_sha256: [u8; 32],
    pub prior_state_complete: bool,
    pub evidence_accepted: u32,
    pub evidence_inconclusive: u32,
    pub evidence_rejected: u32,
    pub syntax_match: u32,
    pub syntax_other: u32,
    pub actor_reference_match: u32,
    pub actor_reference_other: u32,
    pub decisions_decided: u32,
    pub decisions_other: u32,
    pub order_divergent_blocks: u32,
    pub observed_types: BTreeSet<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum GateBlocker {
    /// Litecoin testnet has no TradeLayer instance or JointChainState.
    NoTradeLayerInstance,
    NoParityReport,
    /// The report's corpus digest differs from the authenticated corpus.
    ParityReportUnbound,
    EmptyParityCorpus,
    PriorStateIncomplete,
    EvidenceNotAccepted,
    SyntaxParityNotExact,
    ActorReferenceParityNotExact,
    DecisionsNotAllDecided,
    /// Legacy catch-up orders a block by txid after types 4/20; Rust applies
    /// block position. Needs a ratified rule before any affected block.
    LegacyIntraBlockOrderDiverges,
    ProhibitedType(u8),
    TypeOutsideOracle(u8),
    /// JS lets a type be used in its activation block; Stage 22 rejects it
    /// (JointError::ActivatedInSameBlock).
    ActivationSameBlockSemanticsDiffer,
    /// No Rust replay from AuthenticatedObservation exists to compare
    /// decisions and state digests with the legacy oracle.
    RustDecisionReplayMissing,
}

/// The narrow Stage 23C gate. An empty result would mean "eligible for
/// human review", never automatic authority; in this stage the last two
/// blockers are unconditional.
pub fn stage23c_gate(
    chain: LitecoinChain,
    authenticated_corpus_sha256: [u8; 32],
    report: Option<&ParityReportSummary>,
) -> BTreeSet<GateBlocker> {
    let mut blockers = BTreeSet::new();
    if chain.tradelayer_instance().is_none() {
        blockers.insert(GateBlocker::NoTradeLayerInstance);
    }
    match report {
        None => {
            blockers.insert(GateBlocker::NoParityReport);
        }
        Some(r) => {
            if r.corpus_sha256 != authenticated_corpus_sha256 {
                blockers.insert(GateBlocker::ParityReportUnbound);
            }
            if r.decisions_decided == 0 {
                blockers.insert(GateBlocker::EmptyParityCorpus);
            }
            if !r.prior_state_complete {
                blockers.insert(GateBlocker::PriorStateIncomplete);
            }
            if r.evidence_inconclusive + r.evidence_rejected > 0 {
                blockers.insert(GateBlocker::EvidenceNotAccepted);
            }
            if r.syntax_other > 0 {
                blockers.insert(GateBlocker::SyntaxParityNotExact);
            }
            if r.actor_reference_other > 0 {
                blockers.insert(GateBlocker::ActorReferenceParityNotExact);
            }
            if r.decisions_other > 0 {
                blockers.insert(GateBlocker::DecisionsNotAllDecided);
            }
            if r.order_divergent_blocks > 0 {
                blockers.insert(GateBlocker::LegacyIntraBlockOrderDiverges);
            }
            for ty in &r.observed_types {
                if PROHIBITED_TYPES.contains(ty) {
                    blockers.insert(GateBlocker::ProhibitedType(*ty));
                } else if !ORACLE_TYPES.contains(ty) {
                    blockers.insert(GateBlocker::TypeOutsideOracle(*ty));
                }
            }
        }
    }
    blockers.insert(GateBlocker::ActivationSameBlockSemanticsDiffer);
    blockers.insert(GateBlocker::RustDecisionReplayMissing);
    blockers
}
