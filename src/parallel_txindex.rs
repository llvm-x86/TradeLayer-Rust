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
    inspect_prepared_transaction, inspect_transaction, ScanError, TransactionLayout,
    TransactionObservation, MAX_RAW_TRANSACTION,
};
use std::collections::{BTreeMap, HashMap};
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

// TXIDs are reduced only after workers join and always in block order.
// A randomized std HashMap changes lookup cost, never duplicate ordering:
// neither implementation iterates its keys to choose a failure.
trait TxidSeen {
    fn first_or_insert(&mut self, txid: [u8; 32], position: usize) -> Option<usize>;
}

impl TxidSeen for BTreeMap<[u8; 32], usize> {
    fn first_or_insert(&mut self, txid: [u8; 32], position: usize) -> Option<usize> {
        if let Some(&first) = self.get(&txid) {
            return Some(first);
        }
        self.insert(txid, position);
        None
    }
}

impl TxidSeen for HashMap<[u8; 32], usize> {
    fn first_or_insert(&mut self, txid: [u8; 32], position: usize) -> Option<usize> {
        match self.entry(txid) {
            std::collections::hash_map::Entry::Occupied(found) => Some(*found.get()),
            std::collections::hash_map::Entry::Vacant(vacant) => {
                vacant.insert(position);
                None
            }
        }
    }
}

fn append_observation<I: TxidSeen>(
    report: &mut UnverifiedOrderedBlockIndex,
    seen: &mut I,
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
    if let Some(first) = seen.first_or_insert(observation.txid, index) {
        return Err(ParallelIndexError::DuplicateTxid {
            first,
            second: index,
        });
    }
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
fn inspect_unverified_block_generic<T, F>(
    raw_transactions: &[T],
    config: ParallelIndexConfig,
    scan: F,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError>
where
    T: AsRef<[u8]> + Sync,
    F: Fn(&T) -> Result<TransactionObservation, ScanError> + Sync,
{
    let config = config.validate()?;
    validate_count(raw_transactions.len())?;
    let mut bytes = 0usize;
    for (index, raw) in raw_transactions.iter().enumerate() {
        validate_row(index, raw.as_ref())?;
        bytes = bytes
            .checked_add(raw.as_ref().len())
            .filter(|sum| *sum <= MAX_BLOCK_INPUT_BYTES)
            .ok_or(ParallelIndexError::ExceededBlockByteLimit)?;
    }
    let mut report = blank_index(raw_transactions.len());
    let mut seen = BTreeMap::<[u8; 32], usize>::new();
    for (window_index, window) in raw_transactions
        .chunks(config.window_transactions)
        .enumerate()
    {
        let start = window_index * config.window_transactions;
        let completed = work_window(start, window.len(), config.workers, &|index| {
            scan(&raw_transactions[index])
                .map_err(|source| ParallelIndexError::RawScan { index, source })
        })?;
        for (offset, result) in completed.into_iter().enumerate() {
            let index = start + offset;
            let observation = result?;
            append_observation(
                &mut report,
                &mut seen,
                index,
                raw_transactions[index].as_ref().len(),
                observation,
            )?;
        }
    }
    // A validated untrusted observation must account for every input byte.
    debug_assert_eq!(report.raw_bytes_inspected, bytes);
    Ok(report)
}

/// Each borrowed raw-TX span retains its one-pass strict structural offsets.
pub(crate) struct PreparedRawTransaction<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) layout: &'a TransactionLayout,
}
impl AsRef<[u8]> for PreparedRawTransaction<'_> {
    fn as_ref(&self) -> &[u8] {
        self.raw
    }
}

/// Same bounded workers, windows and ordered error semantics as the
/// original scanner, but WITHOUT a second transaction structural parse.
pub(crate) fn inspect_prepared_block_parallel(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_unverified_block_generic(transactions, config, |item| {
        inspect_prepared_transaction(item.raw, item.layout)
    })
}

/// EXPERIMENTAL whole-block CPU worker team. In contrast to work_window,
/// which spawns and joins threads every 512 transactions, this scans all
/// prepared TXs in ONE scoped team. Worker jobs remain stateless and pure.
/// Dynamic atomic claims preserve work balancing across variable TX sizes.
///
/// Resource tradeoff: this variant allocates ONE result slot per transaction
/// (bounded by MAX_BLOCK_TRANSACTIONS and MAX_BLOCK_INPUT_BYTES), rather than
/// the regular 512-slot window. It is opt-in until benchmarks show a repeatable
/// win and bounded-memory policy is reviewed.
fn inspect_prepared_block_one_team_with_index<I: TxidSeen>(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
    make_index: impl FnOnce(usize) -> I,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    let config = config.validate()?;
    validate_count(transactions.len())?;
    let mut bytes = 0usize;
    for (index, row) in transactions.iter().enumerate() {
        validate_row(index, row.raw)?;
        bytes = bytes
            .checked_add(row.raw.len())
            .filter(|sum| *sum <= MAX_BLOCK_INPUT_BYTES)
            .ok_or(ParallelIndexError::ExceededBlockByteLimit)?;
    }
    if config.workers == 1 || transactions.len() == 1 {
        return inspect_prepared_block_parallel(transactions, config);
    }
    let worker_count = config.workers.min(transactions.len());
    let claim_size = (config.window_transactions / (worker_count * 4)).clamp(1, 16);
    let next = AtomicUsize::new(0);
    let slots: Vec<OnceLock<Result<TransactionObservation, ParallelIndexError>>> =
        (0..transactions.len()).map(|_| OnceLock::new()).collect();
    let panicked = thread::scope(|scope| {
        let handles: Vec<_> = (0..worker_count)
            .map(|_| {
                scope.spawn(|| loop {
                    let first = next.fetch_add(claim_size, Ordering::Relaxed);
                    if first >= transactions.len() {
                        break;
                    }
                    let end = first.saturating_add(claim_size).min(transactions.len());
                    for index in first..end {
                        let row = &transactions[index];
                        let result = inspect_prepared_transaction(row.raw, row.layout)
                            .map_err(|source| ParallelIndexError::RawScan { index, source });
                        // Each index belongs to exactly one atomic claim.
                        let _ = slots[index].set(result);
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
    let mut report = blank_index(transactions.len());
    let mut seen = make_index(transactions.len());
    // Consume only AFTER all workers join, strictly in original order.
    // A failed row at a higher index can never mask a lower-index failure.
    for (index, slot) in slots.into_iter().enumerate() {
        let observation = slot
            .into_inner()
            .ok_or(ParallelIndexError::MissingWorkerResult)??;
        append_observation(
            &mut report,
            &mut seen,
            index,
            transactions[index].raw.len(),
            observation,
        )?;
    }
    debug_assert_eq!(report.raw_bytes_inspected, bytes);
    Ok(report)
}

/// Keep the original deterministic tree reducer as the baseline.
pub(crate) fn inspect_prepared_block_one_team(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_prepared_block_one_team_with_index(transactions, config, |_| {
        BTreeMap::<[u8; 32], usize>::new()
    })
}

/// EXPERIMENTAL source-ordered reduction with randomized std HashMap.
/// Reserve one key per prepared TX, bounded by prior input-count validation.
/// This changes only an in-memory, untrusted duplicate-TXID lookup.
pub(crate) fn inspect_prepared_block_one_team_hash_seen(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_prepared_block_one_team_with_index(transactions, config, |count| {
        HashMap::<[u8; 32], usize>::with_capacity(count)
    })
}

/// Preserve the original owned-Vec API for existing callers. The shared
/// implementation now accepts borrowed slices without copying transaction
/// bytes, enabling one-getblock/one-parser/parallel-TXID preindexing.
pub fn inspect_unverified_block_parallel(
    raw_transactions: &[Vec<u8>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_unverified_block_generic(raw_transactions, config, |item| {
        inspect_transaction(item.as_ref())
    })
}

/// Analyze independent borrowed transaction spans in parallel. No per-TX
/// serialized byte buffer is allocated or retained by this function.
/// Results and the lowest failing index are ordered by original block index.
pub fn inspect_unverified_block_slices_parallel(
    raw_transactions: &[&[u8]],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_unverified_block_generic(raw_transactions, config, |item| inspect_transaction(item))
}

pub fn inspect_unverified_block_slices_serial(
    raw_transactions: &[&[u8]],
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    inspect_unverified_block_generic(raw_transactions, ParallelIndexConfig::serial(), |item| {
        inspect_transaction(item)
    })
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
    let mut seen = BTreeMap::<[u8; 32], usize>::new();
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
