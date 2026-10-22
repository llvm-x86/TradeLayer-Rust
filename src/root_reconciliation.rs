//! Cross-namespace reconciliation of two separately unverified record streams.
//!
//! The TLPT v1 protocol roots cover Rust TLPS snapshots. The zkTL audit roots
//! cover a different state namespace and a JS-generated receipt chain. This
//! read-only module binds *their metadata side by side* with declared L1
//! transaction observations. It never equates the roots and never proves
//! signatures, chain inclusion, economic transitions, or STWO proof bytes.
//!
//! Source observations MUST be independently authenticated before being used
//! in any higher-trust system. Matching this transcript alone proves nothing
//! about that provenance. This API cannot mutate a ledger or authorize txs.

use crate::bitcoin_chainwork::{AnchorStatus, MainnetWorkSegment, Work256};
use crate::bitcoin_inclusion::{CheckedHeaderSegment, IncludedTransaction};
use crate::chain::{BlockId, ChainId, Tip};
use crate::hash256::sha256;
use crate::read_only::decode_hex;
use crate::state_commitment::{CommitmentError, TransitionStatement, TRANSITION_BYTES};
use crate::zk_batch::{AuditBatch, ClaimError, Digest, MAX_RECEIPTS};
use crate::TransactionType;

pub const MAX_ALIGNED_BLOCKS: usize = 500;
pub const MAX_ROOT_BRIDGE_TEXT_BYTES: usize = 4 * 1024 * 1024;

const BRIDGE_DOMAIN: &[u8] = b"TLR_ZKTL_ALIGNMENT_AUDIT_V1\0";

#[derive(Debug)]
pub enum BridgeError {
    Zk(ClaimError),
    Statement(CommitmentError),
    Invalid(&'static str),
    TooManyBlocks,
    WrongChain,
    WrongAnchors,
    MissingBlock,
    DiscontinuousTip,
    DiscontinuousRustState,
    ReceiptMismatch,
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BridgeError {}
impl From<ClaimError> for BridgeError {
    fn from(value: ClaimError) -> Self {
        Self::Zk(value)
    }
}
impl From<CommitmentError> for BridgeError {
    fn from(value: CommitmentError) -> Self {
        Self::Statement(value)
    }
}

/// These expected values are configuration assertions, not validation of L1.
/// For production, an independently verifying chain reader must establish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedChainRange {
    pub chain: ChainId,
    pub anchor: Tip,
    pub final_tip: Tip,
}

/// Caller-declared observation: the block and transaction containing a zkTL
/// receipt. Ordering counts only protocol receipts observed in that block.
/// Independent txid, membership and type verification are external obligations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedReceipt {
    pub height: u64,
    pub block_id: BlockId,
    pub txid: Digest,
    pub tx_type: TransactionType,
    pub order_in_block: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockAlignment {
    pub height: u64,
    pub block_id: BlockId,
    pub tlpt_statement_id: [u8; 32],
    pub rust_before: [u8; 32],
    pub rust_after: [u8; 32],
    pub zktl_before: Digest,
    pub zktl_after: Digest,
    pub receipt_count: u32,
}

/// A strict *audit result* built from untrusted records in different root
/// namespaces. Never present its digest as a Cairo/STWO proof or chain seal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub expected: ExpectedChainRange,
    pub zktl_batch_id: Digest,
    pub rust_first_root: [u8; 32],
    pub rust_last_root: [u8; 32],
    pub zktl_first_root: Digest,
    pub zktl_last_root: Digest,
    pub blocks: Vec<BlockAlignment>,
    pub receipt_count: usize,
    pub audit_digest: [u8; 32],
}

/// Strictly align a source-derived zkTL receipt batch with local TLPT block
/// statements and an exact, externally observed transaction list.
///
/// The zkTL and TLPT roots are *different meanings*. We bind both to block
/// context without assuming any equality or economic transition equivalence.
///
/// This function can only attest that the provided untrusted inputs agree with
/// one another; a dishonest reporter can invent a self-consistent transcript.
pub fn reconcile_unverified(
    batch: &AuditBatch,
    statements: &[TransitionStatement],
    observations: &[ObservedReceipt],
    expected: ExpectedChainRange,
) -> Result<ReconciliationReport, BridgeError> {
    batch.inspect_strict()?;
    if statements.is_empty() {
        return Err(BridgeError::MissingBlock);
    }
    if statements.len() > MAX_ALIGNED_BLOCKS {
        return Err(BridgeError::TooManyBlocks);
    }
    if observations.len() != batch.receipts.len() || observations.len() > MAX_RECEIPTS {
        return Err(BridgeError::ReceiptMismatch);
    }
    if expected.final_tip.height <= expected.anchor.height {
        return Err(BridgeError::WrongAnchors);
    }
    let span = batch
        .end_height
        .checked_sub(batch.start_height)
        .and_then(|n| n.checked_add(1))
        .ok_or(BridgeError::Invalid("bad zkTL block range"))?;
    if usize::try_from(span).ok() != Some(statements.len()) {
        return Err(BridgeError::MissingBlock);
    }
    if statements[0].parent() != expected.anchor
        || statements.last().expect("nonempty").child() != expected.final_tip
        || statements[0].child().height != batch.start_height
        || expected.final_tip.height != batch.end_height
    {
        return Err(BridgeError::WrongAnchors);
    }

    let mut blocks = Vec::with_capacity(statements.len());
    let mut previous = None;
    for (i, statement) in statements.iter().enumerate() {
        if statement.chain() != expected.chain {
            return Err(BridgeError::WrongChain);
        }
        let height = batch
            .start_height
            .checked_add(i as u64)
            .ok_or(BridgeError::Invalid("height overflow"))?;
        if statement.child().height != height {
            return Err(BridgeError::MissingBlock);
        }
        if let Some(older) = previous {
            let older: &TransitionStatement = older;
            if older.child() != statement.parent() {
                return Err(BridgeError::DiscontinuousTip);
            }
            if older.new_protocol_root() != statement.old_protocol_root() {
                return Err(BridgeError::DiscontinuousRustState);
            }
        }
        previous = Some(statement);
        blocks.push(BlockAlignment {
            height,
            block_id: statement.child().id,
            tlpt_statement_id: statement.id(),
            rust_before: statement.old_protocol_root(),
            rust_after: statement.new_protocol_root(),
            zktl_before: batch.first_state_root,
            zktl_after: batch.first_state_root,
            receipt_count: 0,
        });
    }

    // zkTL's strict inspection already verifies ordered, uninterrupted
    // zkTL root transitions. Assign that root chain to each TLPT block,
    // including empty blocks where zkTL's logical root stays unchanged.
    let mut zk_current = batch.first_state_root;
    let mut cursor = 0;
    for block in &mut blocks {
        block.zktl_before = zk_current;
        while cursor < batch.receipts.len() {
            let receipt = &batch.receipts[cursor];
            if receipt.height != block.height {
                break;
            }
            let seen = &observations[cursor];
            if seen.height != block.height
                || seen.block_id != block.block_id
                || seen.txid != receipt.txid
                || seen.tx_type != receipt.tx_type
                || seen.order_in_block != block.receipt_count
            {
                return Err(BridgeError::ReceiptMismatch);
            }
            // Independent of source provenance, all zkTL receipts must
            // continue from the prior zkTL root, never the Rust TLPS root.
            if receipt.old_state_root != zk_current {
                return Err(BridgeError::ReceiptMismatch);
            }
            zk_current = receipt.new_state_root;
            block.receipt_count = block
                .receipt_count
                .checked_add(1)
                .ok_or(BridgeError::Invalid("receipt count overflow"))?;
            cursor += 1;
        }
        block.zktl_after = zk_current;
    }
    if cursor != batch.receipts.len() || zk_current != batch.final_state_root {
        return Err(BridgeError::ReceiptMismatch);
    }

    let mut transcript = Vec::with_capacity(256 + blocks.len() * 230 + observations.len() * 120);
    transcript.extend_from_slice(BRIDGE_DOMAIN);
    transcript.extend_from_slice(&expected.chain.0);
    append_tip(&mut transcript, expected.anchor);
    append_tip(&mut transcript, expected.final_tip);
    transcript.extend_from_slice(&batch.batch_id.as_bytes());
    transcript.extend_from_slice(&batch.first_state_root.as_bytes());
    transcript.extend_from_slice(&batch.final_state_root.as_bytes());
    let rust_first_root = statements[0].old_protocol_root();
    let rust_last_root = statements.last().expect("nonempty").new_protocol_root();
    transcript.extend_from_slice(&rust_first_root);
    transcript.extend_from_slice(&rust_last_root);
    transcript.extend_from_slice(&(blocks.len() as u32).to_be_bytes());
    for block in &blocks {
        transcript.extend_from_slice(&block.height.to_be_bytes());
        transcript.extend_from_slice(&block.block_id.0);
        transcript.extend_from_slice(&block.tlpt_statement_id);
        transcript.extend_from_slice(&block.rust_before);
        transcript.extend_from_slice(&block.rust_after);
        transcript.extend_from_slice(&block.zktl_before.as_bytes());
        transcript.extend_from_slice(&block.zktl_after.as_bytes());
        transcript.extend_from_slice(&block.receipt_count.to_be_bytes());
    }
    transcript.extend_from_slice(&(observations.len() as u32).to_be_bytes());
    for (observed, receipt) in observations.iter().zip(&batch.receipts) {
        transcript.extend_from_slice(&observed.height.to_be_bytes());
        transcript.extend_from_slice(&observed.block_id.0);
        transcript.extend_from_slice(&observed.txid.as_bytes());
        transcript.push(observed.tx_type.as_u8());
        transcript.extend_from_slice(&observed.order_in_block.to_be_bytes());
        transcript.extend_from_slice(&receipt.receipt_id.as_bytes());
    }

    Ok(ReconciliationReport {
        expected,
        zktl_batch_id: batch.batch_id,
        rust_first_root,
        rust_last_root,
        zktl_first_root: batch.first_state_root,
        zktl_last_root: batch.final_state_root,
        blocks,
        receipt_count: observations.len(),
        audit_digest: sha256(&transcript),
    })
}

fn append_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}

fn canonical_u64(input: &str) -> Result<u64, BridgeError> {
    if input.is_empty()
        || (input.len() > 1 && input.starts_with('0'))
        || !input.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(BridgeError::Invalid("noncanonical decimal"));
    }
    input
        .parse()
        .map_err(|_| BridgeError::Invalid("decimal overflow"))
}

fn lowercase_bytes(input: &str) -> Result<Vec<u8>, BridgeError> {
    if !input
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(BridgeError::Invalid("nonlowercase hex"));
    }
    decode_hex(input).map_err(|_| BridgeError::Invalid("bad hexadecimal input"))
}

/// Each row is exactly two TAB fields: 64-character TLPT statement ID and
/// 554-character lowercase-hex canonical TLPT bytes, with no header/comments.
pub fn parse_tlpt_tsv_unverified(input: &str) -> Result<Vec<TransitionStatement>, BridgeError> {
    if input.len() > MAX_ROOT_BRIDGE_TEXT_BYTES || !input.is_ascii() {
        return Err(BridgeError::Invalid("oversized TLPT input"));
    }
    let mut statements = Vec::new();
    for row in input.lines() {
        let (id, bytes) = row
            .split_once('\t')
            .ok_or(BridgeError::Invalid("TLPT row fields"))?;
        if bytes.contains('\t') || bytes.len() != TRANSITION_BYTES * 2 {
            return Err(BridgeError::Invalid("TLPT row size"));
        }
        let expected_id = Digest::parse(id)?.as_bytes();
        let decoded = lowercase_bytes(bytes)?;
        let statement = TransitionStatement::from_canonical_bytes_unverified(&decoded)?;
        if statement.id() != expected_id {
            return Err(BridgeError::Invalid("TLPT ID mismatch"));
        }
        if statements.len() >= MAX_ALIGNED_BLOCKS {
            return Err(BridgeError::TooManyBlocks);
        }
        statements.push(statement);
    }
    if statements.is_empty() {
        return Err(BridgeError::MissingBlock);
    }
    Ok(statements)
}

/// Observation file format is six tab-separated fields per line:
/// O, height, block_hash_hex, txid_hex, tx_type, order_within_block.
/// This is an *unverified declaration*, not proof of L1 inclusion.
pub fn parse_observations_tsv_unverified(input: &str) -> Result<Vec<ObservedReceipt>, BridgeError> {
    if input.len() > MAX_ROOT_BRIDGE_TEXT_BYTES || !input.is_ascii() {
        return Err(BridgeError::Invalid("oversized observation input"));
    }
    let mut result = Vec::new();
    for line in input.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 6 || fields[0] != "O" {
            return Err(BridgeError::Invalid("observation row fields"));
        }
        let height = canonical_u64(fields[1])?;
        let block_id = BlockId(Digest::parse(fields[2])?.as_bytes());
        let txid = Digest::parse(fields[3])?;
        let ty = canonical_u64(fields[4])?;
        let tx_type = TransactionType::try_from(
            u8::try_from(ty).map_err(|_| BridgeError::Invalid("transaction type"))?,
        )
        .map_err(|_| BridgeError::Invalid("transaction type"))?;
        let position = canonical_u64(fields[5])?;
        let order_in_block =
            u32::try_from(position).map_err(|_| BridgeError::Invalid("observation order"))?;
        if result.len() >= MAX_RECEIPTS {
            return Err(BridgeError::Invalid("too many observations"));
        }
        result.push(ObservedReceipt {
            height,
            block_id,
            txid,
            tx_type,
            order_in_block,
        });
    }
    Ok(result)
}

const SPV_ALIGNMENT_DOMAIN: &[u8] = b"TLR_SHA256D_MERKLE_AUDIT_V1\0";

/// Internal consistency of hash/PoW/Merkle proofs, bound to separate zkTL
/// audit receipts and TLPT roots. It does not prove the anchor is trusted,
/// the headers are the active best chain, or settlement economics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedInclusionReport {
    pub root_alignment: ReconciliationReport,
    pub inclusion_audit_digest: [u8; 32],
}

/// Unlike reconcile_unverified, this bridge does not accept caller-declared
/// receipt positions, txids or transaction type metadata. It requires opaque
/// IncludedTransaction evidence produced after raw transaction parsing and
/// Merkle verification against SHA256d PoW-checked headers. However, the
/// header chain's anchor and network choice still come from an external caller.
pub fn reconcile_checked_bitcoin_inclusions_unverified(
    batch: &AuditBatch,
    statements: &[TransitionStatement],
    header_segment: &CheckedHeaderSegment,
    included: &[IncludedTransaction],
    expected: ExpectedChainRange,
) -> Result<CheckedInclusionReport, BridgeError> {
    if header_segment.anchor() != expected.anchor
        || header_segment.tip() != expected.final_tip
        || header_segment.headers().len() != statements.len()
    {
        return Err(BridgeError::WrongAnchors);
    }
    if included.len() != batch.receipts.len() || included.len() > MAX_RECEIPTS {
        return Err(BridgeError::ReceiptMismatch);
    }
    let mut observations = Vec::with_capacity(included.len());
    let mut last_height = None;
    let mut last_index = None;
    let mut tx_count_this_block = None;
    let mut protocol_order = 0u32;
    for proof in included {
        let header = header_segment
            .headers()
            .iter()
            .find(|header| header.tip().height == proof.height())
            .ok_or(BridgeError::MissingBlock)?;
        if header.tip().id != proof.block_id() {
            return Err(BridgeError::ReceiptMismatch);
        }
        if last_height != Some(proof.height()) {
            if last_height.is_some_and(|last| proof.height() <= last) {
                return Err(BridgeError::ReceiptMismatch);
            }
            last_height = Some(proof.height());
            last_index = None;
            tx_count_this_block = None;
            protocol_order = 0;
        }
        if last_index.is_some_and(|index| proof.tx_index() <= index)
            || tx_count_this_block.is_some_and(|count| proof.tx_count() != count)
        {
            return Err(BridgeError::ReceiptMismatch);
        }
        last_index = Some(proof.tx_index());
        tx_count_this_block = Some(proof.tx_count());
        let hint = proof
            .legacy_type_hint()
            .ok_or(BridgeError::ReceiptMismatch)?;
        let txid_hex: String = proof
            .txid_display()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let txid = Digest::parse(&txid_hex)?;
        observations.push(ObservedReceipt {
            height: proof.height(),
            block_id: proof.block_id(),
            txid,
            tx_type: hint,
            order_in_block: protocol_order,
        });
        protocol_order += 1;
    }
    let root_alignment = reconcile_unverified(batch, statements, &observations, expected)?;
    // Cryptographically bind full-L1 transaction positions and counts in
    // addition to Stage 15's protocol-receipt-only ordering. A hash of a
    // claimed checked result is still not a proof of chain membership.
    let mut transcript = Vec::new();
    transcript.extend_from_slice(SPV_ALIGNMENT_DOMAIN);
    transcript.extend_from_slice(&root_alignment.audit_digest);
    transcript.extend_from_slice(&expected.chain.0);
    append_tip(&mut transcript, header_segment.anchor());
    transcript.extend_from_slice(&(header_segment.headers().len() as u32).to_be_bytes());
    for header in header_segment.headers() {
        append_tip(&mut transcript, header.tip());
        transcript.extend_from_slice(&header.parent().0);
        transcript.extend_from_slice(&header.merkle_root_raw());
        transcript.extend_from_slice(&header.n_bits().to_be_bytes());
    }
    transcript.extend_from_slice(&(included.len() as u32).to_be_bytes());
    for proof in included {
        transcript.extend_from_slice(&proof.height().to_be_bytes());
        transcript.extend_from_slice(&proof.block_id().0);
        transcript.extend_from_slice(&proof.txid_display());
        transcript.extend_from_slice(&proof.tx_index().to_be_bytes());
        transcript.extend_from_slice(&proof.tx_count().to_be_bytes());
        transcript.push(proof.legacy_type_hint().expect("checked earlier").as_u8());
    }
    Ok(CheckedInclusionReport {
        root_alignment,
        inclusion_audit_digest: sha256(&transcript),
    })
}


/// A read-only commitment over Stage 16 inclusion checks PLUS independently
/// checked mainnet difficulty/MTP/chainwork since an explicit anchor.
/// Neither this hash nor a published-genesis ancestry tag certifies the
/// Bitcoin active best chain, script authority, or zkTL economics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainnetWorkAlignment {
    pub inclusion: CheckedInclusionReport,
    pub anchor_status: AnchorStatus,
    pub work_before: Work256,
    pub work_after: Work256,
    pub header_work_audit_digest: [u8; 32],
}
const MAINNET_WORK_ALIGNMENT_DOMAIN: &[u8] = b"TLR_BITCOIN_MAINNET_HEADER_WORK_AUDIT_V1\0";

/// Stronger audit metadata than caller-observed transaction rows. It
/// requires an opaque MainnetWorkSegment created only by the mainnet
/// difficulty/MTP/chainwork checker. The Stage 13 trusted adapter and
/// independent STWO economic prover remain unimplemented.
pub fn reconcile_mainnet_work_headers_unverified(
    batch: &AuditBatch,
    statements: &[TransitionStatement],
    work_segment: &MainnetWorkSegment,
    included: &[IncludedTransaction],
    expected: ExpectedChainRange,
) -> Result<MainnetWorkAlignment, BridgeError> {
    let inclusion = reconcile_checked_bitcoin_inclusions_unverified(
        batch, statements, work_segment.checked_headers(), included, expected,
    )?;
    let mut bytes = Vec::with_capacity(MAINNET_WORK_ALIGNMENT_DOMAIN.len() + 32 * 4 + 1);
    bytes.extend_from_slice(MAINNET_WORK_ALIGNMENT_DOMAIN);
    bytes.extend_from_slice(&inclusion.inclusion_audit_digest);
    let anchor_status = work_segment.anchor_status();
    bytes.push(match anchor_status {
        AnchorStatus::PublishedGenesis => 1,
        AnchorStatus::ClaimedExternal => 0,
    });
    let work_before = work_segment.work_before();
    let work_after = work_segment.work_after();
    bytes.extend_from_slice(&work_before.to_be_bytes());
    bytes.extend_from_slice(&work_after.to_be_bytes());
    bytes.extend_from_slice(&expected.chain.0);
    Ok(MainnetWorkAlignment {
        inclusion,
        anchor_status,
        work_before,
        work_after,
        header_work_audit_digest: sha256(&bytes),
    })
}
