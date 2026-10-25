//! Bounded parallel **pre-execution** Bitcoin-style block transaction scanner.
//!
//! This implementation addresses the per-block TXID hashing and TradeLayer
//! carrier inspection bottleneck. It uses ONLY Rust std threads, no Rayon,
//! async runtime or external crates. Workers inspect raw transaction bytes
//! independently; a bounded atomic chunk cursor spreads large/small TXs
//! among workers without spawning one OS thread per transaction.
//!
//! ABSOLUTE CONSENSUS BOUNDARY:
//! - Caller-supplied raw transaction ordering, block/header and network are
//!   NOT authenticated by this module. No Merkle, best chain, UTXO, script,
//!   ECDSA, historical activation, list authority or economic state proof.
//! - Returned TXIDs are local SHA256d fingerprints, not a trusted active
//!   chain. Bitcoin-style parsing does NOT establish Litecoin/Dogecoin rules.
//! - The scanner NEVER executes or reorders Ledger, Registry, orderbook,
//!   clearing or settlement operations. These MUST remain in canonical
//!   tx_index order under a separately authenticated protocol adapter.
//! - No partial output is published on any worker parse error, duplicate,
//!   panic, input limit or ambiguous config. The lowest TX index always
//!   wins a failure, not the fastest worker's completion time.
//! - For historical txIndex.js source behavior, see the distinct legacy
//!   index audit. This scanner is a future Rust implementation facility,
//!   intentionally NOT a port of the source's incorrect decodePayload call.

use crate::chain::MAX_TRANSACTIONS_PER_BLOCK;
use crate::read_only::{inspect_transaction, ScanError, TransactionObservation, MAX_RAW_TRANSACTION};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

pub const MAX_PARALLEL_WORKERS: usize = 16;
pub const MAX_PARALLEL_CHUNK: usize = 128;
pub const MAX_PARALLEL_BLOCK_BYTES: usize = 32 * 1024 * 1024;
pub const DEFAULT_CHUNK_SIZE: usize = 16;
pub const AUTO_PARALLEL_THRESHOLD: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelScanConfig {
    /// 1..=16 OS workers. Explicit to keep CPU use bounded; can be chosen
    /// using recommended_for_block() based on actual available parallelism.
    pub workers: usize,
    /// How many contiguous transaction indices one worker claims per
    /// fetch_add. 1..=128, never one thread per transaction.
    pub chunk_size: usize,
}
impl ParallelScanConfig {
    pub const fn serial() -> Self {
        Self { workers: 1, chunk_size: DEFAULT_CHUNK_SIZE }
    }
    /// Use the scheduler's CPU allotment, NOT logical hardware-thread count.
    /// For blocks smaller than 64 TX, serial scan avoids threading overhead.
    pub fn recommended_for_block(count: usize) -> Self {
        let workers = if count < AUTO_PARALLEL_THRESHOLD {
            1
        } else {
            thread::available_parallelism().map_or(1, |n| n.get()).min(MAX_PARALLEL_WORKERS)
        };
        Self { workers, chunk_size: DEFAULT_CHUNK_SIZE }
    }
    fn check(self) -> Result<(), ParallelIndexError> {
        if self.workers == 0 || self.workers > MAX_PARALLEL_WORKERS
            || self.chunk_size == 0 || self.chunk_size > MAX_PARALLEL_CHUNK {
            return Err(ParallelIndexError::InvalidWorkerConfig);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelIndexError {
    InvalidWorkerConfig,
    EmptyBlock,
    TooManyTransactions,
    TotalRawBytesTooLarge,
    BadTransactionSize { tx_index: usize },
    BadTransaction { tx_index: usize, source: ScanError },
    DuplicateTxid { first_index: usize, duplicate_index: usize },
    WorkerPanicked,
    InternalMissingResult { tx_index: usize },
    /// This untrusted research output is NOT an accepted block candidate.
    HistoricalBlockAndUtxoUnverified,
    /// No canonical signed off-chain TradeLayer channel state is defined.
    CanonicalSignedRevisionUnavailable,
    /// No trusted state mutation can be derived from this scanner.
    TrustedConsensusExecutionUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedObservation {
    tx_index: u32,
    observation: TransactionObservation,
}
impl IndexedObservation {
    pub const fn tx_index(&self) -> u32 {
        self.tx_index
    }
    pub fn observation(&self) -> &TransactionObservation {
        &self.observation
    }
    pub const fn txid_raw(&self) -> [u8; 32] {
        self.observation.txid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanStatistics {
    pub transaction_count: usize,
    pub total_raw_bytes: usize,
    pub carrier_candidate_transactions: usize,
    /// Actual number of std worker threads used, not requested CPUs.
    pub workers_used: usize,
    pub chunk_size: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedIndexedBlock {
    /// Every input TX has one result, not just TL candidates; this binds
    /// candidate indices to the immutable source-provided block ordering.
    ordered: Vec<IndexedObservation>,
    statistics: ScanStatistics,
}
impl UnverifiedIndexedBlock {
    pub fn ordered(&self) -> &[IndexedObservation] {
        &self.ordered
    }
    pub const fn statistics(&self) -> ScanStatistics {
        self.statistics
    }
    pub fn trade_layer_candidates(&self) -> impl Iterator<Item=&IndexedObservation> {
        self.ordered.iter().filter(|row| !row.observation.carriers.is_empty())
    }
    pub const fn require_authenticated_chain_and_utxo(
        &self,
    ) -> Result<(), ParallelIndexError> {
        Err(ParallelIndexError::HistoricalBlockAndUtxoUnverified)
    }
    pub const fn require_signed_channel_revision(&self) -> Result<(), ParallelIndexError> {
        Err(ParallelIndexError::CanonicalSignedRevisionUnavailable)
    }
    pub const fn require_trusted_consensus_execution(&self)
        -> Result<(), ParallelIndexError> {
        Err(ParallelIndexError::TrustedConsensusExecutionUnavailable)
    }
}

type LocalResult = (usize, Result<TransactionObservation, ScanError>);

fn preflight(
    raws: &[Vec<u8>],
    config: ParallelScanConfig,
) -> Result<usize, ParallelIndexError> {
    config.check()?;
    if raws.is_empty() {
        return Err(ParallelIndexError::EmptyBlock);
    }
    if raws.len() > MAX_TRANSACTIONS_PER_BLOCK {
        return Err(ParallelIndexError::TooManyTransactions);
    }
    let mut bytes = 0usize;
    for (index, raw) in raws.iter().enumerate() {
        if raw.is_empty() || raw.len() > MAX_RAW_TRANSACTION {
            return Err(ParallelIndexError::BadTransactionSize { tx_index:index });
        }
        bytes = bytes.checked_add(raw.len())
            .filter(|total| *total <= MAX_PARALLEL_BLOCK_BYTES)
            .ok_or(ParallelIndexError::TotalRawBytesTooLarge)?;
    }
    Ok(bytes)
}

/// Independently inspect and SHA256d-hash every serialized transaction in a
/// candidate block using bounded, dynamically claimed chunks. All results
/// are reassembled strictly in original source order; return all-or-nothing.
/// This is NOT a Bitcoin full-block consensus or TradeLayer admission call.
pub fn scan_transactions_parallel_unverified(
    raw_transactions: &[Vec<u8>],
    config: ParallelScanConfig,
) -> Result<UnverifiedIndexedBlock, ParallelIndexError> {
    let total_bytes = preflight(raw_transactions, config)?;
    let workers = config.workers.min(raw_transactions.len());
    let mut positions: Vec<Option<Result<TransactionObservation,ScanError>>> =
        vec![None; raw_transactions.len()];
    if workers == 1 {
        for (i, raw) in raw_transactions.iter().enumerate() {
            positions[i] = Some(inspect_transaction(raw));
        }
    } else {
        let next = AtomicUsize::new(0);
        // Scoped threads borrow the input slices. Each thread has its OWN
        // result Vec, so hashing/decoding never contends on an output mutex.
        // The main thread alone assembles results AFTER joins.
        let batches = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                let next = &next;
                handles.push(scope.spawn(move || {
                    let mut local = Vec::<LocalResult>::new();
                    loop {
                        let begin = next.fetch_add(config.chunk_size, Ordering::Relaxed);
                        if begin >= raw_transactions.len() { break; }
                        let end = begin.saturating_add(config.chunk_size).min(raw_transactions.len());
                        for (offset,raw) in raw_transactions[begin..end].iter().enumerate() {
                            local.push((begin + offset,inspect_transaction(raw)));
                        }
                    }
                    local
                }));
            }
            // Joining in fixed worker creation order cannot affect the
            // transaction-order result or the chosen first error index.
            let mut outputs = Vec::with_capacity(workers);
            let mut panicked = false;
            for handle in handles {
                match handle.join() {
                    Ok(local) => outputs.push(local),
                    Err(_) => panicked = true,
                }
            }
            if panicked { Err(ParallelIndexError::WorkerPanicked) }
            else { Ok(outputs) }
        })?;
        for local in batches {
            for (index,result) in local {
                if positions[index].replace(result).is_some() {
                    // A duplicate claim is impossible with fetch_add, but
                    // never publish a partial block if violated.
                    return Err(ParallelIndexError::InternalMissingResult { tx_index:index });
                }
            }
        }
    }

    let mut ordered = Vec::with_capacity(raw_transactions.len());
    let mut seen = BTreeMap::<[u8;32], usize>::new();
    let mut candidate_count=0usize;
    for (index,item) in positions.into_iter().enumerate() {
        let observation = item.ok_or(ParallelIndexError::InternalMissingResult {tx_index:index})?
            .map_err(|source| ParallelIndexError::BadTransaction {
                tx_index:index,source
            })?;
        if let Some(first_index) = seen.insert(observation.txid,index) {
            return Err(ParallelIndexError::DuplicateTxid {
                first_index,duplicate_index:index
            });
        }
        if !observation.carriers.is_empty() {candidate_count+=1;}
        ordered.push(IndexedObservation {
            tx_index:u32::try_from(index).expect("bounded 10000 tx limit"),
            observation
        });
    }
    Ok(UnverifiedIndexedBlock {
        ordered,
        statistics:ScanStatistics {
            transaction_count:raw_transactions.len(),
            total_raw_bytes:total_bytes,
            carrier_candidate_transactions:candidate_count,
            workers_used:workers,
            chunk_size:config.chunk_size,
        }
    })
}

/// Convenience entry point with bounded CPU allotment; the caller still
/// provides the serialized block's actual ordered transaction list.
pub fn scan_transactions_auto_unverified(
    raw_transactions: &[Vec<u8>],
) -> Result<UnverifiedIndexedBlock,ParallelIndexError> {
    scan_transactions_parallel_unverified(
        raw_transactions,ParallelScanConfig::recommended_for_block(raw_transactions.len())
    )
}
