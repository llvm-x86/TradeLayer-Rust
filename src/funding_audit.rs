//! Stage 24B: offline funding-flow characterization for legacy clearing.js.
//!
//! Pinned legacy path: tradelayer.js@dca0888b src/clearing.js:670-768.
//! Its negative-rate branch passes a NEGATIVE total to the payer distributor.
//! The distributor negates that amount for supposed payers, so short accounts
//! receive funds while long accounts are debited. Each side also rounds each
//! share separately, potentially breaking supply conservation.
//!
//! Two intentionally distinct, READ-ONLY projections:
//! * LegacySigned: project the old attempted Tally.updateBalance deltas.
//! * ConservingProposal: positive debits for either funding direction,
//!   per-payer rounded charges, deterministic Hamilton receiver allocation,
//!   and all-account available-balance checks before calculating a preview.
//!
//! This is bounded integer-input research, not verified JS runtime equivalence.
//! Historical code permits fractional contracts, runtime BigNumber settings
//! and JS Number conversion. No wallet, chain adapter, ledger writes, external
//! crates, proof acceptance, or authorization for settlement types 3/23/31.

use std::collections::BTreeSet;

const BPS_DENOMINATOR_E4: u128 = 100_000_000;
pub const MAX_AUDIT_POSITIONS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingMode {
    LegacySigned,
    ConservingProposal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingPosition {
    pub account: String,
    pub contracts: i64,
    pub available_atoms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingDelta {
    pub account: String,
    pub delta_atoms: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingProjection {
    pub mode: FundingMode,
    /// Sum of separately rounded funding amounts on the payer side, in atoms.
    pub nominal_atoms: u128,
    /// Sum of all projected account deltas. This need not be zero for legacy.
    pub net_atoms: i128,
    pub deltas: Vec<FundingDelta>,
    /// Pure preview only. Legacy projections cannot claim successful database
    /// execution because Tally.updateBalance may throw mid-sequence.
    pub simulated_available_atoms: Option<Vec<u128>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingAuditError {
    TooManyPositions,
    EmptyAccount,
    DuplicateAccount,
    ArithmeticOverflow,
    MissingCounterparty,
    InsufficientAvailable(String),
}

impl core::fmt::Display for FundingAuditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FundingAuditError {}

fn multiply(a: u128, b: u128) -> Result<u128, FundingAuditError> {
    a.checked_mul(b).ok_or(FundingAuditError::ArithmeticOverflow)
}

fn half_up(numerator: u128, denominator: u128) -> Result<u128, FundingAuditError> {
    if denominator == 0 {
        return Err(FundingAuditError::ArithmeticOverflow);
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder >= denominator - remainder {
        quotient.checked_add(1).ok_or(FundingAuditError::ArithmeticOverflow)
    } else {
        Ok(quotient)
    }
}

fn weight(position: &FundingPosition) -> u128 {
    u128::from(position.contracts.unsigned_abs())
}

fn add_signed(deltas: &mut [i128], idx: usize, atoms: u128, negative: bool) -> Result<(), FundingAuditError> {
    let amount = i128::try_from(atoms).map_err(|_| FundingAuditError::ArithmeticOverflow)?;
    let amount = if negative { -amount } else { amount };
    deltas[idx] = deltas[idx]
        .checked_add(amount)
        .ok_or(FundingAuditError::ArithmeticOverflow)?;
    Ok(())
}

/// Legacy applies two INDEPENDENT 8-decimal rounded weighted splits.
/// This can create or destroy available atoms. Source BigNumber divides
/// weights to 20 decimal places first; the bounded integer cases chosen for
/// this oracle are insensitive to that intermediate rounding.
fn independently_rounded(
    total: u128,
    side: &[usize],
    positions: &[FundingPosition],
) -> Result<Vec<(usize, u128)>, FundingAuditError> {
    let denominator = side.iter().try_fold(0u128, |sum, &idx| {
        sum.checked_add(weight(&positions[idx]))
            .ok_or(FundingAuditError::ArithmeticOverflow)
    })?;
    if denominator == 0 {
        return Ok(Vec::new());
    }
    side.iter()
        .map(|&idx| {
            let amount = half_up(multiply(total, weight(&positions[idx]))?, denominator)?;
            Ok((idx, amount))
        })
        .collect()
}

/// Allocates exactly `total` atoms to a positive-weight receiver side.
/// Ties use ascending ASCII account names rather than encounter order.
/// This is a NEW proposed rule, never assumed for historical JS replay.
fn hamilton_distribution(
    total: u128,
    side: &[usize],
    positions: &[FundingPosition],
) -> Result<Vec<(usize, u128)>, FundingAuditError> {
    let denominator = side.iter().try_fold(0u128, |sum, &idx| {
        sum.checked_add(weight(&positions[idx]))
            .ok_or(FundingAuditError::ArithmeticOverflow)
    })?;
    if denominator == 0 {
        return Err(FundingAuditError::MissingCounterparty);
    }
    let mut shares: Vec<(usize, u128, u128)> = Vec::with_capacity(side.len());
    let mut allocated = 0u128;
    for &idx in side {
        let numerator = multiply(total, weight(&positions[idx]))?;
        let quotient = numerator / denominator;
        let remainder = numerator % denominator;
        allocated = allocated
            .checked_add(quotient)
            .ok_or(FundingAuditError::ArithmeticOverflow)?;
        shares.push((idx, quotient, remainder));
    }
    let remaining = total
        .checked_sub(allocated)
        .ok_or(FundingAuditError::ArithmeticOverflow)?;
    let remaining = usize::try_from(remaining).map_err(|_| FundingAuditError::ArithmeticOverflow)?;
    if remaining > shares.len() {
        return Err(FundingAuditError::ArithmeticOverflow);
    }
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by(|&a, &b| {
        shares[b]
            .2
            .cmp(&shares[a].2)
            .then_with(|| positions[shares[a].0].account.cmp(&positions[shares[b].0].account))
    });
    for &idx in order.iter().take(remaining) {
        shares[idx].1 = shares[idx]
            .1
            .checked_add(1)
            .ok_or(FundingAuditError::ArithmeticOverflow)?;
    }
    Ok(shares.into_iter().map(|(idx, atoms, _)| (idx, atoms)).collect())
}

fn projection(
    mode: FundingMode,
    positions: &[FundingPosition],
    nominal_atoms: u128,
    deltas: &[i128],
) -> Result<FundingProjection, FundingAuditError> {
    let net_atoms = deltas.iter().try_fold(0i128, |sum, amount| {
        sum.checked_add(*amount)
            .ok_or(FundingAuditError::ArithmeticOverflow)
    })?;
    let mut simulated = None;
    if mode == FundingMode::ConservingProposal {
        if net_atoms != 0 {
            return Err(FundingAuditError::ArithmeticOverflow);
        }
        let mut preview = Vec::with_capacity(positions.len());
        for (position, &delta) in positions.iter().zip(deltas) {
            let balance = if delta < 0 {
                position
                    .available_atoms
                    .checked_sub(delta.unsigned_abs())
                    .ok_or_else(|| FundingAuditError::InsufficientAvailable(position.account.clone()))?
            } else {
                position
                    .available_atoms
                    .checked_add(u128::try_from(delta).map_err(|_| FundingAuditError::ArithmeticOverflow)?)
                    .ok_or(FundingAuditError::ArithmeticOverflow)?
            };
            preview.push(balance);
        }
        simulated = Some(preview);
    }
    Ok(FundingProjection {
        mode,
        nominal_atoms,
        net_atoms,
        deltas: positions
            .iter()
            .zip(deltas)
            .map(|(position, &delta_atoms)| FundingDelta {
                account: position.account.clone(),
                delta_atoms,
            })
            .collect(),
        simulated_available_atoms: simulated,
    })
}

/// Model attempted balance deltas for one funding application.
///
/// `rate_bps_e4` is a signed basis-point rate scaled by 10,000, so
/// `100_000` represents +10 bps and `-125_000` represents -12.5 bps.
/// `notional_atoms` is collateral currency in 1e-8 units per contract.
/// The source first rounds each paying position's amount to 8 decimals;
/// legacy then rounds all payer and receiver shares independently.
///
/// `LegacySigned` makes no claims about whether JS Tally.updateBalance
/// would succeed or fail, and never reads/mutates an actual balance ledger.
/// `ConservingProposal` previews atomically and refuses absent counterparties
/// or balances insufficient to pay the full debit.
pub fn audit_funding(
    positions: &[FundingPosition],
    notional_atoms: u64,
    rate_bps_e4: i64,
    mode: FundingMode,
) -> Result<FundingProjection, FundingAuditError> {
    if positions.len() > MAX_AUDIT_POSITIONS {
        return Err(FundingAuditError::TooManyPositions);
    }
    let mut seen = BTreeSet::new();
    for position in positions {
        if position.account.is_empty() {
            return Err(FundingAuditError::EmptyAccount);
        }
        if !seen.insert(&position.account) {
            return Err(FundingAuditError::DuplicateAccount);
        }
    }
    let mut deltas = vec![0i128; positions.len()];
    if rate_bps_e4 == 0 || notional_atoms == 0 {
        return projection(mode, positions, 0, &deltas);
    }
    let positive = rate_bps_e4 > 0;
    let mut payers = Vec::new();
    let mut receivers = Vec::new();
    for (index, position) in positions.iter().enumerate() {
        if (positive && position.contracts > 0) || (!positive && position.contracts < 0) {
            payers.push(index);
        } else if position.contracts != 0 {
            receivers.push(index);
        }
    }
    if payers.is_empty() {
        return projection(mode, positions, 0, &deltas);
    }
    let mut per_payer = Vec::new();
    let mut total = 0u128;
    for &idx in &payers {
        let product = multiply(weight(&positions[idx]), u128::from(notional_atoms))?;
        let product = multiply(product, u128::from(rate_bps_e4.unsigned_abs()))?;
        let charge = half_up(product, BPS_DENOMINATOR_E4)?;
        total = total
            .checked_add(charge)
            .ok_or(FundingAuditError::ArithmeticOverflow)?;
        per_payer.push((idx, charge));
    }
    if total == 0 {
        return projection(mode, positions, 0, &deltas);
    }
    match mode {
        FundingMode::LegacySigned => {
            // JS calculates *negative* shortFunding for negative rates and
            // supplies it unchanged. Negation at Tally flips debit/credit.
            for (idx, atoms) in independently_rounded(total, &payers, positions)? {
                add_signed(&mut deltas, idx, atoms, positive)?;
            }
            for (idx, atoms) in independently_rounded(total, &receivers, positions)? {
                add_signed(&mut deltas, idx, atoms, !positive)?;
            }
        }
        FundingMode::ConservingProposal => {
            if receivers.is_empty() {
                return Err(FundingAuditError::MissingCounterparty);
            }
            for (idx, atoms) in per_payer {
                add_signed(&mut deltas, idx, atoms, true)?;
            }
            for (idx, atoms) in hamilton_distribution(total, &receivers, positions)? {
                add_signed(&mut deltas, idx, atoms, false)?;
            }
        }
    }
    projection(mode, positions, total, &deltas)
}
