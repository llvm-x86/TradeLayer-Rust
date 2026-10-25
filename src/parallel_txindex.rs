//! Runtime parallelism for read-only Bitcoin-style transaction indexing.
//!
//! Unlike the serial historical txIndex.js RPC/fetch/decode loop, independent
//! raw-transaction fetches, SHA256d, structural scans and OP_RETURN candidate
//! extraction can run on multiple CPU threads. Results are emitted **only**
//! in original block transaction order, regardless of completion order.
//!
//! THIS IS NOT CONSENSUS VALIDATION. An observed txid does not prove a block,
//! best chain, script/witness validity, UTXO ownership, parent ordering,
//! activated protocol type, historical clearlist, or economic authorization.
//! The caller may inspect results but cannot obtain TrustedChainAdapter,
//! Ledger permissions, authenticated operations, or a consensus transition.
//!
//! The trusted application phase MUST remain strictly transaction ordered:
//! UTXO spending, balances, register writes, matching and auction semantics
//! may depend on all earlier transactions. Never run stateful validators or
//! mutate a Ledger inside workers. The fetching callback MUST be read-only
//! and safe for concurrent use (for example, isolated RPC getrawtransaction).
//!
//! Bounded CPU pool: at most 16 scoped threads and 512 transaction slots per
//! window. No extra crates, unsafe code, unbounded spawn per transaction,
//! order-dependent concurrent maps, hidden source fallback, or worker-owned
//! state commits. Any input/fetch/scan/mismatch error aborts the entire
//! returned report with its earliest block-order position.
//!
//! A separate offline benchmark exercises CPU and mock-RPC throughput but
//! never asserts a speedup on resource-constrained CI runners.

use crate::read_only::{
    inspect_transaction, ScanError, TransactionObservation, MAX_RAW_TRANSACTION,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread;

pub const MAX_PARALLEL_WORKERS: usize = 16;
pub const MAX_INDEX_WINDOW: usize = 512;
pub const MAX_BLOCK_TRANSACTIONS: usize = 200_000;
pub const MAX_BLOCK_INPUT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelIndexConfig {
    /// Number of read-only CPU/fetch worker threads; 1 runs serially.
    pub workers: usize,
    /// Maximum transactions admitted to a parallel window. Source order
    /// is recovered before any observations leave a window.
    pub window_transactions: usize,
}
impl Default for ParallelIndexConfig {
    fn default() -> Self {
        let available = thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            workers: available.min(8),
            window_transactions: 256,
        }
    }
}
impl ParallelIndexConfig {
    pub const fn serial() -> Self {
        Self {
            workers: 1,
            window_transactions: 256,
        }
    }
    pub fn validate(self) -> Result<Self, ParallelIndexError> {
        if !(1..=MAX_PARALLEL_WORKERS).contains(&self.workers)
            || !(1..=MAX_INDEX_WINDOW).contains(&self.window_transactions)
        {
            return Err(ParallelIndexError::InvalidConfig);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelIndexError {
    InvalidConfig,
    EmptyBlock,
    TooManyTransactions,
    ExceededBlockByteLimit,
    EmptyRawTransaction { index: usize },
    OversizedRawTransaction { index: usize },
    RawScan { index: usize, source: ScanError },
    Fetch { index: usize, message: String },
    ReturnedTxidMismatch { index: usize },
    DuplicateTxid { first: usize, second: usize },
    WorkerPanicked,
    MissingWorkerResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedTransaction {
    /// Original *zero-based block position*, not completion/scheduling order.
    pub position: usize,
    /// Read-only parsed bytes, TXID/wTXID, and candidate OP_RETURN markers.
    pub observation: TransactionObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedOrderedBlockIndex {
    ordered: Vec<IndexedTransaction>,
    candidate_positions: Vec<usize>,
    raw_bytes_inspected: usize,
    op_return_outputs: u64,
}
impl UnverifiedOrderedBlockIndex {
    pub fn transaction_count(&self) -> usize {
        self.ordered.len()
    }
    pub fn raw_bytes_inspected(&self) -> usize {
        self.raw_bytes_inspected
    }
    pub fn op_return_output_count(&self) -> u64 {
        self.op_return_outputs
    }
    pub fn candidate_positions(&self) -> &[usize] {
        &self.candidate_positions
    }
    pub fn ordered(&self) -> &[IndexedTransaction] {
        &self.ordered
    }
    /// Read-only candidate references in canonical *input order*.
    /// No actual spend, signature, sender, list, or ledger proof exists.
    pub fn candidates_in_source_order(&self) -> impl Iterator<Item = &IndexedTransaction> {
        self.candidate_positions
            .iter()
            .map(|position| &self.ordered[*position])
    }
    /// An unordered worker result can NEVER supply trusted consensus input.
    pub const fn require_authenticated_block(&self) -> Result<(), ParallelIndexTrustError> {
        Err(ParallelIndexTrustError::FullBlockAndChainNotVerified)
    }
    pub const fn require_authenticated_transaction_state(
        &self,
    ) -> Result<(), ParallelIndexTrustError> {
        Err(ParallelIndexTrustError::ScriptAndStateNotVerified)
    }
    pub const fn require_canonical_signed_channel_revision(
        &self,
    ) -> Result<(), ParallelIndexTrustError> {
        Err(ParallelIndexTrustError::SignedStateRevisionUnavailable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelIndexTrustError {
    FullBlockAndChainNotVerified,
    ScriptAndStateNotVerified,
    SignedStateRevisionUnavailable,
}

fn validate_row(index: usize, raw: &[u8]) -> Result<(), ParallelIndexError> {
    if raw.is_empty() {
        return Err(ParallelIndexError::EmptyRawTransaction { index });
    }
    if raw.len() > MAX_RAW_TRANSACTION {
        return Err(ParallelIndexError::OversizedRawTransaction { index });
    }
    Ok(())
}

fn validate_count(count: usize) -> Result<(), ParallelIndexError> {
    if count == 0 {
        return Err(ParallelIndexError::EmptyBlock);
    }
    if count > MAX_BLOCK_TRANSACTIONS {
        return Err(ParallelIndexError::TooManyTransactions);
    }
    Ok(())
}

/// Execute one bounded window with dynamic atomic work distribution.
/// There is one result slot per input, written at most once. The coordinator
/// only consumes slots AFTER every worker has joined. This guarantees output
/// order and first-failing-index determinism even when the last scheduled
/// worker finishes before a lower-index slow/failing worker.
fn work_window<T, F>(
    start: usize,
    length: usize,
    worker_count: usize,
    job: &F,
) -> Result<Vec<Result<T, ParallelIndexError>>, ParallelIndexError>
where
    T: Send + Sync,
    F: Fn(usize) -> Result<T, ParallelIndexError> + Sync,
{
    if worker_count == 1 || length == 1 {
        return Ok((start..start + length).map(job).collect());
    }
    let slots: Vec<OnceLock<Result<T, ParallelIndexError>>> =
        (0..length).map(|_| OnceLock::new()).collect();
    let next = AtomicUsize::new(0);
    // Dynamic micro-batching reduces atomic contention for thousands of
    // cheap TXID scans, while preserving >=4 scheduling opportunities per
    // worker and retaining fine-grained balancing for slow read-only RPC.
    // No thread is allowed to mutate protocol state through this function.
    let worker_slots = worker_count.min(length);
    let claim_size = (length / (worker_slots * 4)).clamp(1, 8);
    let panicked = thread::scope(|scope| {
        let handles: Vec<_> = (0..worker_slots)
            .map(|_| {
                scope.spawn(|| loop {
                    let offset = next.fetch_add(claim_size, Ordering::Relaxed);
                    if offset >= length {
                        break;
                    }
                    let end = offset.saturating_add(claim_size).min(length);
                    for (slot, cell) in slots.iter().enumerate().take(end).skip(offset) {
                        let result = job(start + slot);
                        // Each index is owned by exactly one claimed range.
                        let _ = cell.set(result);
                    }
                })
            })
            .collect();
        let mut failed = false;
        for handle in handles {
            if handle.join().is_err() {
                failed = true;
            }
        }
        failed
    });
    if panicked {
        return Err(ParallelIndexError::WorkerPanicked);
    }
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .ok_or(ParallelIndexError::MissingWorkerResult)
        })
        .collect()
}

fn append_observation(
    report: &mut UnverifiedOrderedBlockIndex,
    seen: &mut BTreeMap<[u8; 32], usize>,
    index: usize,
    bytes: usize,
    observation: TransactionObservation,
) -> Result<(), ParallelIndexError> {
    // Output is a local, untrusted report: no consensus/state side effects.
    report.raw_bytes_inspected = report
        .raw_bytes_inspected
        .checked_add(bytes)
        .filter(|total| *total <= MAX_BLOCK_INPUT_BYTES)
        .ok_or(ParallelIndexError::ExceededBlockByteLimit)?;
    if let Some(&first) = seen.get(&observation.txid) {
        return Err(ParallelIndexError::DuplicateTxid {
            first,
            second: index,
        });
    }
    seen.insert(observation.txid, index);
    report.op_return_outputs += u64::from(observation.op_return_count);
    if !observation.carriers.is_empty() {
        report.candidate_positions.push(index);
    }
    report.ordered.push(IndexedTransaction {
        position: index,
        observation,
    });
    Ok(())
}
fn blank_index(size: usize) -> UnverifiedOrderedBlockIndex {
    UnverifiedOrderedBlockIndex {
        ordered: Vec::with_capacity(size),
        candidate_positions: Vec::new(),
        raw_bytes_inspected: 0,
        op_return_outputs: 0,
    }
}

/// Parallel read-only scan of already fetched, **caller-supplied** raw
/// transactions. CPU parsing/SHA256d is parallel; results are placed in
/// original block order and never committed to chain or ledger state.
/// An error aborts the entire report; no partially indexed success escapes.
pub fn inspect_unverified_block_parallel(
    raw_transactions: &[Vec<u8>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    let config = config.validate()?;
    validate_count(raw_transactions.len())?;
    let mut bytes = 0usize;
    for (index, raw) in raw_transactions.iter().enumerate() {
        validate_row(index, raw)?;
        bytes = bytes
            .checked_add(raw.len())
            .filter(|sum| *sum <= MAX_BLOCK_INPUT_BYTES)
            .ok_or(ParallelIndexError::ExceededBlockByteLimit)?;
    }
    let mut report = blank_index(raw_transactions.len());
    let mut seen = BTreeMap::new();
    for (window_index, window) in raw_transactions
        .chunks(config.window_transactions)
        .enumerate()
    {
        let start = window_index * config.window_transactions;
        let completed = work_window(start, window.len(), config.workers, &|index| {
            inspect_transaction(&raw_transactions[index])
                .map_err(|source| ParallelIndexError::RawScan { index, source })
        })?;
        for (offset, result) in completed.into_iter().enumerate() {
            let index = start + offset;
            let observation = result?;
            append_observation(
                &mut report,
                &mut seen,
                index,
                raw_transactions[index].len(),
                observation,
            )?;
        }
    }
    // A validated untrusted observation must account for every input byte.
    debug_assert_eq!(report.raw_bytes_inspected, bytes);
    Ok(report)
}

pub fn inspect_unverified_block_serial(
    raw_transactions: &[Vec<u8>],
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_unverified_block_parallel(raw_transactions, ParallelIndexConfig::serial())
}

/// Fetch raw transactions concurrently from an **external, read-only**
/// caller-provided function. The txids are in *Bitcoin raw SHA256d digest
/// order*, NOT explorer-display reversed hex. Each fetched transaction is
/// hashed again locally; a wrong raw reply always fails closed.
///
/// Workers keep at most ONE fetched serialized transaction alive each,
/// then discard bytes after parsing. Windows limit the queued observations
/// to 512. No RPC/HTTP dependency or on-chain node configuration is built
/// into the Rust crate. A production fetcher must implement independent
/// source authentication and only ever perform idempotent reads.
pub fn fetch_and_inspect_unverified_parallel<F>(
    expected_txids_raw: &[[u8; 32]],
    config: ParallelIndexConfig,
    fetch_raw: F,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError>
where
    F: Fn(usize, [u8; 32]) -> Result<Vec<u8>, String> + Sync,
{
    let config = config.validate()?;
    validate_count(expected_txids_raw.len())?;
    let mut report = blank_index(expected_txids_raw.len());
    let mut seen = BTreeMap::new();
    for (window_index, window) in expected_txids_raw
        .chunks(config.window_transactions)
        .enumerate()
    {
        let start = window_index * config.window_transactions;
        let completed = work_window(start, window.len(), config.workers, &|index| {
            let expected = expected_txids_raw[index];
            let raw = fetch_raw(index, expected)
                .map_err(|message| ParallelIndexError::Fetch { index, message })?;
            validate_row(index, &raw)?;
            let observation = inspect_transaction(&raw)
                .map_err(|source| ParallelIndexError::RawScan { index, source })?;
            if observation.txid != expected {
                return Err(ParallelIndexError::ReturnedTxidMismatch { index });
            }
            Ok((raw.len(), observation))
        })?;
        for (offset, result) in completed.into_iter().enumerate() {
            let index = start + offset;
            let (bytes, observation) = result?;
            append_observation(&mut report, &mut seen, index, bytes, observation)?;
        }
    }
    Ok(report)
}

/// Deterministic serial reference for verifying the network-fetch path.
/// Neither variant authenticates the expected IDs' block inclusion.
pub fn fetch_and_inspect_unverified_serial<F>(
    expected_txids_raw: &[[u8; 32]],
    fetch_raw: F,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError>
where
    F: Fn(usize, [u8; 32]) -> Result<Vec<u8>, String> + Sync,
{
    fetch_and_inspect_unverified_parallel(
        expected_txids_raw,
        ParallelIndexConfig::serial(),
        fetch_raw,
    )
}
