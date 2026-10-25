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
use std::collections::BTreeMap;
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::sync::{mpsc, Arc, Barrier, Mutex};
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
    let mut seen = BTreeMap::new();
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
pub(crate) fn inspect_prepared_block_one_team(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
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
    let mut seen = BTreeMap::new();
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

/// CPU-byte-work estimate used ONLY to size scheduling claims.
/// SegWit requires hashing raw (wtxid) and stripped (txid) bytes.
/// Classification work may differ, so all chunk assignments remain dynamic.
fn prepared_hash_work(row: &PreparedRawTransaction<'_>) -> usize {
    let stripped = row.layout.stripped_bytes();
    if stripped < row.raw.len() {
        row.raw.len() + stripped
    } else {
        row.raw.len()
    }
}

/// Build source-contiguous, variable-sized microclaims from a bounded window.
/// Each worker pulls the next claim on completion, avoiding the load imbalance
/// from assigning entire static shards to workers. This returns scheduling
/// metadata only; transactions remain borrowed from the original raw block.
fn weighted_window_claims(
    transactions: &[PreparedRawTransaction<'_>],
    start: usize,
    end: usize,
    workers: usize,
) -> Vec<Range<usize>> {
    let total_work: usize = transactions[start..end]
        .iter()
        .map(prepared_hash_work)
        .sum();
    let target_work = total_work.div_ceil(workers * 4).max(512);
    let mut claims = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let first = cursor;
        let mut work = 0usize;
        while cursor < end && (cursor == first || (work < target_work && cursor - first < 32)) {
            work += prepared_hash_work(&transactions[cursor]);
            cursor += 1;
        }
        claims.push(first..cursor);
    }
    claims
}

#[derive(Debug, Clone, Copy)]
struct WeightedWindowJob {
    chunk: usize,
    first: usize,
    end: usize,
}

struct WeightedWindowResult {
    chunk: usize,
    rows: Result<Vec<Result<TransactionObservation, ParallelIndexError>>, ParallelIndexError>,
}

/// EXPERIMENTAL: reuse one scoped worker team across all source-ordered
/// windows. At most config.window_transactions (<=512) observations can be
/// in flight in a single window, and the work queue itself is bounded to
/// twice the number of workers. No per-TX OnceLock, global atomic TX index,
/// extra crate, unsafe or serialized-TX copying.
///
/// Within each window, workers dynamically claim contiguous TX ranges sized
/// by estimated SHA256d bytes. A worker never touches protocol state. Only
/// the coordinator reduces observations, detects the earliest error/duplicate
/// by block position and validates the block Merkle in the existing caller.
///
/// Catch unwind at the *job* boundary so worker panics become WorkerPanicked
/// rather than leaving a missing result stuck behind a blocking receiver.
/// This remains an untrusted CPU preindex, never Script/UTXO/chain/ledger.
pub(crate) fn inspect_prepared_block_pooled_weighted(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
) -> Result<UnverifiedOrderedBlockIndex, ParallelIndexError> {
    let config = config.validate()?;
    validate_count(transactions.len())?;
    let mut total_bytes = 0usize;
    for (index, row) in transactions.iter().enumerate() {
        validate_row(index, row.raw)?;
        total_bytes = total_bytes
            .checked_add(row.raw.len())
            .filter(|sum| *sum <= MAX_BLOCK_INPUT_BYTES)
            .ok_or(ParallelIndexError::ExceededBlockByteLimit)?;
    }
    if config.workers == 1 || transactions.len() == 1 {
        return inspect_prepared_block_parallel(transactions, config);
    }

    let worker_count = config.workers.min(transactions.len());
    let (task_sender, task_receiver) = mpsc::sync_channel::<WeightedWindowJob>(worker_count * 2);
    let task_receiver = Mutex::new(task_receiver);
    let (result_sender, result_receiver) = mpsc::channel::<WeightedWindowResult>();
    let mut report = blank_index(transactions.len());
    let mut seen = BTreeMap::new();

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let tasks = &task_receiver;
            let completed = result_sender.clone();
            handles.push(scope.spawn(move || loop {
                // Hold the receiver mutex only while accepting the next job.
                // Worker hashing and classification happen outside the lock.
                let job = match tasks.lock() {
                    Ok(guard) => guard.recv(),
                    Err(_) => break,
                };
                let Ok(job) = job else { break };
                let rows = catch_unwind(AssertUnwindSafe(|| {
                    let mut local = Vec::with_capacity(job.end - job.first);
                    for (offset, item) in transactions[job.first..job.end].iter().enumerate() {
                        let index = job.first + offset;
                        let row = inspect_prepared_transaction(item.raw, item.layout)
                            .map_err(|source| ParallelIndexError::RawScan { index, source });
                        let failed = row.is_err();
                        local.push(row);
                        if failed {
                            break;
                        }
                    }
                    local
                }))
                .map_err(|_| ParallelIndexError::WorkerPanicked);
                if completed
                    .send(WeightedWindowResult {
                        chunk: job.chunk,
                        rows,
                    })
                    .is_err()
                {
                    break;
                }
            }));
        }
        drop(result_sender);

        let scanned = (|| {
            for (window_index, window) in
                transactions.chunks(config.window_transactions).enumerate()
            {
                let start = window_index * config.window_transactions;
                let claims =
                    weighted_window_claims(transactions, start, start + window.len(), worker_count);
                // Only this window is dispatched at a time. At most 512 raw
                // observations are retained by workers and result messages.
                for (chunk, claim) in claims.iter().enumerate() {
                    task_sender
                        .send(WeightedWindowJob {
                            chunk,
                            first: claim.start,
                            end: claim.end,
                        })
                        .map_err(|_| ParallelIndexError::WorkerPanicked)?;
                }
                let mut gathered = (0..claims.len())
                    .map(|_| None)
                    .collect::<Vec<Option<WeightedWindowResult>>>();
                for _ in 0..claims.len() {
                    let finished = result_receiver
                        .recv()
                        .map_err(|_| ParallelIndexError::WorkerPanicked)?;
                    let slot = gathered
                        .get_mut(finished.chunk)
                        .ok_or(ParallelIndexError::MissingWorkerResult)?;
                    if slot.replace(finished).is_some() {
                        return Err(ParallelIndexError::MissingWorkerResult);
                    }
                }
                // A panic anywhere takes precedence, matching the original
                // windowed and one-team scanner's join-failure semantics.
                if gathered.iter().any(|slot| {
                    matches!(
                        slot,
                        Some(WeightedWindowResult {
                            rows: Err(ParallelIndexError::WorkerPanicked),
                            ..
                        })
                    )
                }) {
                    return Err(ParallelIndexError::WorkerPanicked);
                }
                for (claim, received) in claims.into_iter().zip(gathered) {
                    let rows = received
                        .ok_or(ParallelIndexError::MissingWorkerResult)?
                        .rows?;
                    let row_count = rows.len();
                    for (offset, row) in rows.into_iter().enumerate() {
                        let index = claim.start + offset;
                        append_observation(
                            &mut report,
                            &mut seen,
                            index,
                            transactions[index].raw.len(),
                            row?,
                        )?;
                    }
                    if row_count != claim.end - claim.start {
                        return Err(ParallelIndexError::MissingWorkerResult);
                    }
                }
            }
            Ok(())
        })();

        // Closing the queue lets persistent workers exit cleanly after the
        // final window (or after an early error) before the scope is left.
        drop(task_sender);
        let mut panicked = false;
        for handle in handles {
            if handle.join().is_err() {
                panicked = true;
            }
        }
        if panicked {
            Err(ParallelIndexError::WorkerPanicked)
        } else {
            scanned
        }
    })?;
    debug_assert_eq!(report.raw_bytes_inspected, total_bytes);
    Ok(report)
}

/// Exactly one bounded, source-ordered window of CPU-only observations.
/// Workers claim variable-sized estimated-byte chunks via one atomic index.
/// All output slots and completed work are discarded after this window.
struct BarrierHashWindow {
    first: usize,
    claims: Vec<Range<usize>>,
    next_claim: AtomicUsize,
    slots: Vec<OnceLock<Result<TransactionObservation, ParallelIndexError>>>,
    panicked: std::sync::atomic::AtomicBool,
}

/// EXPERIMENTAL second bounded-pool scheduler. One reusable scoped worker
/// team runs for the whole block. The coordinator publishes a 1..=512-TX
/// window, releases a start barrier, and waits on a completion barrier.
/// Each worker atomically takes its next SHA256d-byte-weighted microclaim.
/// It then writes directly to immutable-position OnceLock slots, so there
/// are no per-claim mpsc messages and no per-TX allocation across the block.
///
/// Every worker catches a panic within the window's scan before signaling
/// completion; the panic is reported as WorkerPanicked. The coordinator
/// consumes source-ordered results only when all workers finish the window.
/// Any early scan/duplicate failure still performs the final stop barrier
/// and joins the worker team; no consensus or ledger state is touched.
pub(crate) fn inspect_prepared_block_pooled_barrier(
    transactions: &[PreparedRawTransaction<'_>],
    config: ParallelIndexConfig,
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
    let workers = config.workers.min(transactions.len());
    let published = Mutex::<Option<Arc<BarrierHashWindow>>>::new(None);
    let start_barrier = Barrier::new(workers + 1);
    let done_barrier = Barrier::new(workers + 1);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let shared_fault = std::sync::atomic::AtomicBool::new(false);
    let mut report = blank_index(transactions.len());
    let mut seen = BTreeMap::new();

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let current = &published;
            let start = &start_barrier;
            let done = &done_barrier;
            let stopping = &stop;
            let failed = &shared_fault;
            handles.push(scope.spawn(move || loop {
                start.wait();
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                // A poisoned publication lock or missing window must not
                // strand the coordinator at the completion barrier.
                let next = match current.lock() {
                    Ok(guard) => guard.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                let Some(window) = next else {
                    failed.store(true, Ordering::Release);
                    done.wait();
                    continue;
                };
                let worked = catch_unwind(AssertUnwindSafe(|| loop {
                    let index = window.next_claim.fetch_add(1, Ordering::Relaxed);
                    let Some(claim) = window.claims.get(index) else {
                        break;
                    };
                    for (offset, row) in transactions[claim.start..claim.end].iter().enumerate() {
                        let position = claim.start + offset;
                        let observation = inspect_prepared_transaction(row.raw, row.layout)
                            .map_err(|source| ParallelIndexError::RawScan {
                                index: position,
                                source,
                            });
                        // Disjoint claims own each source position exactly once.
                        let _ = window.slots[position - window.first].set(observation);
                    }
                }));
                if worked.is_err() {
                    window.panicked.store(true, Ordering::Release);
                }
                // Drop this Arc before meeting the completion barrier so the
                // coordinator may unwrap and consume slots without cloning.
                drop(window);
                done.wait();
            }));
        }

        let scanned = (|| {
            for (window_id, transactions_window) in
                transactions.chunks(config.window_transactions).enumerate()
            {
                let first = window_id * config.window_transactions;
                let last = first + transactions_window.len();
                let window = Arc::new(BarrierHashWindow {
                    first,
                    claims: weighted_window_claims(transactions, first, last, workers),
                    next_claim: AtomicUsize::new(0),
                    slots: (0..transactions_window.len())
                        .map(|_| OnceLock::new())
                        .collect(),
                    panicked: std::sync::atomic::AtomicBool::new(false),
                });
                {
                    let mut slot = published
                        .lock()
                        .map_err(|_| ParallelIndexError::WorkerPanicked)?;
                    *slot = Some(window.clone());
                }
                start_barrier.wait();
                done_barrier.wait();
                // All worker-held copies were dropped BEFORE done_barrier.
                *published
                    .lock()
                    .map_err(|_| ParallelIndexError::WorkerPanicked)? = None;
                let completed = Arc::try_unwrap(window)
                    .map_err(|_| ParallelIndexError::WorkerPanicked)?;
                if shared_fault.load(Ordering::Acquire)
                    || completed.panicked.load(Ordering::Acquire)
                {
                    return Err(ParallelIndexError::WorkerPanicked);
                }
                for (offset, cell) in completed.slots.into_iter().enumerate() {
                    let position = first + offset;
                    let observation = cell
                        .into_inner()
                        .ok_or(ParallelIndexError::MissingWorkerResult)??;
                    append_observation(
                        &mut report,
                        &mut seen,
                        position,
                        transactions[position].raw.len(),
                        observation,
                    )?;
                }
            }
            Ok(())
        })();

        // Always unblock workers parked at the next start barrier, including
        // when source-order reduction failed halfway through the raw block.
        stop.store(true, Ordering::Release);
        start_barrier.wait();
        let mut any_panicked = false;
        for handle in handles {
            if handle.join().is_err() {
                any_panicked = true;
            }
        }
        if any_panicked {
            Err(ParallelIndexError::WorkerPanicked)
        } else {
            scanned
        }
    })?;
    debug_assert_eq!(report.raw_bytes_inspected, bytes);
    Ok(report)
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
