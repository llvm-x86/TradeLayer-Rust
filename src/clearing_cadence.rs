//! Stage 24C: read-only legacy clearing call-cadence characterization.
//!
//! Pinned source: tradelayer.js@dca0888b src/main.js (catch-up/realtime),
//! src/clearing.js:400-415,553-575. This model describes which end-of-block
//! callbacks the two JavaScript paths ATTEMPT. It never changes history,
//! balances, contracts, funding records, or any authenticated chain state.
//!
//! A crucial naming mismatch exists: the catch-up bucket contains types 4/20;
//! realtime uses literal payload startsWith('4')/startsWith('m'), where 'm'
//! is base36 transaction type 22, not type 20 ('k').

pub const MAX_OBSERVED_TRANSACTIONS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPath {
    CatchUp,
    RealTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxObservation {
    pub tx_type: u8,
    pub already_processed: bool,
    /// The realtime processedAny flag is set before Logic.typeSwitch, when
    /// decodedParams.valid is true. Logic may still fail afterwards.
    pub valid_before_logic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndBlockProjection {
    pub funding_bucket: usize,
    pub other_bucket: usize,
    pub clearing_called: bool,
    pub funding_height_eligible: bool,
    pub funding_attempted: bool,
    pub withdrawals_attempted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadenceError {
    InvalidHeight,
    TooManyInputs,
    UnknownTransactionType,
}

impl core::fmt::Display for CadenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CadenceError {}

pub const fn funding_bucket(path: ReplayPath, ty: u8) -> bool {
    match path {
        ReplayPath::CatchUp => ty == 4 || ty == 20,
        ReplayPath::RealTime => ty == 4 || ty == 22,
    }
}

/// Models catch-up's new/unprocessed count (including invalid transactions)
/// and realtime's `processedAny` indicator from ONLY non-funding payloads.
/// Both call `Clearing.applyFundingRates` from `clearingFunction`,
/// and that function gates on `height.is_multiple_of(24)`.
pub fn project_end_block(
    path: ReplayPath,
    height: u64,
    transactions: &[TxObservation],
) -> Result<EndBlockProjection, CadenceError> {
    if height == 0 {
        return Err(CadenceError::InvalidHeight);
    }
    if transactions.len() > MAX_OBSERVED_TRANSACTIONS {
        return Err(CadenceError::TooManyInputs);
    }
    if transactions.iter().any(|t| t.tx_type > 35) {
        return Err(CadenceError::UnknownTransactionType);
    }
    let funding_bucket_count = transactions
        .iter()
        .filter(|t| funding_bucket(path, t.tx_type))
        .count();
    let other_bucket = transactions.len() - funding_bucket_count;

    let clearing_called = match path {
        ReplayPath::CatchUp => {
            // constructConsensusFromIndex: skip=false when ANY indexed tx
            // has not already been marked processed, or height % 10000 == 0.
            height.is_multiple_of(10_000) || transactions.iter().any(|t| !t.already_processed)
        }
        ReplayPath::RealTime => transactions.iter().any(|t| {
            !funding_bucket(path, t.tx_type) && !t.already_processed && t.valid_before_logic
        }),
    };
    let funding_height_eligible = height.is_multiple_of(24);
    Ok(EndBlockProjection {
        funding_bucket: funding_bucket_count,
        other_bucket,
        clearing_called,
        funding_height_eligible,
        funding_attempted: clearing_called && funding_height_eligible,
        // Catch-up only runs Channels.processWithdrawals inside the postblock
        // skip=false branch. Realtime invokes it after blockHandlerMid either
        // way, including blocks with no valid non-funding TradeLayer work.
        withdrawals_attempted: path == ReplayPath::RealTime || clearing_called,
    })
}
