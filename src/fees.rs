//! Standalone, deterministic maker/taker fee arithmetic in integer atoms.
//!
//! This module is NOT wired into the spot matcher, the ledger, or any
//! consensus path. It moves no balances and authorizes nothing. Callers pass a
//! schedule and a quote amount; the result is a set of integer fee amounts.
//!
//! Conventions:
//! - Fees are denominated in the quote asset of the fill.
//! - Charged fees (taker fee, positive maker fee) round UP, against the payer.
//! - Rebates (negative maker fee) round DOWN, against the recipient.
//! - A schedule whose maker rebate exceeds its taker fee is refused. Then
//!   protocol net is never negative and a self-trade can never be profitable.

use crate::ledger::PropertyId;
use std::collections::BTreeMap;

/// Basis-point denominator: 1 bps = 1 / 10_000.
pub const BPS_DENOMINATOR: u128 = 10_000;
/// Upper bound on the taker fee and on the absolute value of the maker fee.
pub const MAX_FEE_BPS: u32 = 1_000;
/// Upper bound on the number of tiers in one table.
pub const MAX_FEE_TIERS: usize = 16;

/// A fee schedule in basis points. A negative `maker_fee_bps` is a rebate
/// paid to the maker. Fields are public for reading. Settlement and tier
/// construction re-validate every schedule, so a struct literal cannot bypass
/// the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeSchedule {
    pub taker_fee_bps: u32,
    pub maker_fee_bps: i32,
}

impl FeeSchedule {
    /// Validates and constructs a schedule.
    pub fn new(taker_fee_bps: u32, maker_fee_bps: i32) -> Result<Self, FeeError> {
        let schedule = Self {
            taker_fee_bps,
            maker_fee_bps,
        };
        schedule.validate()?;
        Ok(schedule)
    }

    /// Checks the schedule invariants. Called by [`FeeSchedule::new`],
    /// [`settle_fill`] and [`FeeTierTable::new`].
    pub fn validate(&self) -> Result<(), FeeError> {
        if self.taker_fee_bps > MAX_FEE_BPS {
            return Err(FeeError::TakerFeeOutOfRange {
                bps: self.taker_fee_bps,
            });
        }
        if self.maker_fee_bps.unsigned_abs() > MAX_FEE_BPS {
            return Err(FeeError::MakerFeeOutOfRange {
                bps: self.maker_fee_bps,
            });
        }
        if self.maker_fee_bps < 0 {
            let rebate_bps = self.maker_fee_bps.unsigned_abs();
            if rebate_bps > self.taker_fee_bps {
                return Err(FeeError::RebateExceedsTakerFee {
                    rebate_bps,
                    taker_fee_bps: self.taker_fee_bps,
                });
            }
        }
        Ok(())
    }
}

/// Fee amounts for one fill, in atoms of the quote asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillFees {
    /// Charged to the taker, rounded up.
    pub taker_fee_atoms: u128,
    /// Positive: charged to the maker, rounded up. Negative: rebate paid to
    /// the maker, rounded down.
    pub maker_fee_atoms: i128,
    /// `taker_fee_atoms + maker_fee_atoms`. Never negative for a validated
    /// schedule.
    pub protocol_net_atoms: u128,
}

/// Errors from fee validation, settlement and accrual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeError {
    TakerFeeOutOfRange { bps: u32 },
    MakerFeeOutOfRange { bps: i32 },
    RebateExceedsTakerFee { rebate_bps: u32, taker_fee_bps: u32 },
    ZeroQuoteAmount,
    ArithmeticOverflow,
    NegativeProtocolNet,
    EmptyTierTable,
    TooManyTiers { count: usize },
    FirstTierThresholdNonZero { threshold_atoms: u128 },
    DuplicateTierThreshold { index: usize },
    NonMonotonicTierThreshold { index: usize },
}

impl std::fmt::Display for FeeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for FeeError {}

/// Settles one fill. `quote_amount_atoms` must be non-zero.
pub fn settle_fill(schedule: &FeeSchedule, quote_amount_atoms: u64) -> Result<FillFees, FeeError> {
    schedule.validate()?;
    if quote_amount_atoms == 0 {
        return Err(FeeError::ZeroQuoteAmount);
    }
    let quote = u128::from(quote_amount_atoms);
    let taker_fee_atoms = charged_atoms(quote, schedule.taker_fee_bps)?;
    let maker_fee_atoms: i128 = if schedule.maker_fee_bps >= 0 {
        let fee = charged_atoms(quote, schedule.maker_fee_bps.unsigned_abs())?;
        to_signed(fee)?
    } else {
        let rebate = rebate_atoms(quote, schedule.maker_fee_bps.unsigned_abs())?;
        to_signed(rebate)?
            .checked_neg()
            .ok_or(FeeError::ArithmeticOverflow)?
    };
    let net = to_signed(taker_fee_atoms)?
        .checked_add(maker_fee_atoms)
        .ok_or(FeeError::ArithmeticOverflow)?;
    let protocol_net_atoms = u128::try_from(net).map_err(|_| FeeError::NegativeProtocolNet)?;
    Ok(FillFees {
        taker_fee_atoms,
        maker_fee_atoms,
        protocol_net_atoms,
    })
}

/// `ceil(quote * bps / 10_000)`, for fees the payer owes.
fn charged_atoms(quote: u128, bps: u32) -> Result<u128, FeeError> {
    quote
        .checked_mul(u128::from(bps))
        .map(|product| product.div_ceil(BPS_DENOMINATOR))
        .ok_or(FeeError::ArithmeticOverflow)
}

/// `floor(quote * bps / 10_000)`, for rebates the recipient receives.
fn rebate_atoms(quote: u128, bps: u32) -> Result<u128, FeeError> {
    quote
        .checked_mul(u128::from(bps))
        .map(|product| product / BPS_DENOMINATOR)
        .ok_or(FeeError::ArithmeticOverflow)
}

fn to_signed(value: u128) -> Result<i128, FeeError> {
    i128::try_from(value).map_err(|_| FeeError::ArithmeticOverflow)
}

/// Per-quote-property accumulator of protocol net fees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeeAccrual {
    protocol_net_by_property: BTreeMap<PropertyId, u128>,
}

impl FeeAccrual {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one fill's protocol net to `quote_property`. Checked arithmetic:
    /// on overflow nothing is changed.
    pub fn accrue(&mut self, quote_property: PropertyId, fees: &FillFees) -> Result<(), FeeError> {
        let next = self
            .total(quote_property)
            .checked_add(fees.protocol_net_atoms)
            .ok_or(FeeError::ArithmeticOverflow)?;
        self.protocol_net_by_property.insert(quote_property, next);
        Ok(())
    }

    /// Accumulated protocol net for `quote_property`, zero if none.
    pub fn total(&self, quote_property: PropertyId) -> u128 {
        self.protocol_net_by_property
            .get(&quote_property)
            .copied()
            .unwrap_or(0)
    }
}

/// One volume tier. The threshold is the trailing volume (atoms) at which the
/// tier starts to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeTier {
    pub min_trailing_volume_atoms: u128,
    pub schedule: FeeSchedule,
}

impl FeeTier {
    /// Validates the schedule (including rebate <= taker fee) and constructs a
    /// tier.
    pub fn new(
        min_trailing_volume_atoms: u128,
        taker_fee_bps: u32,
        maker_fee_bps: i32,
    ) -> Result<Self, FeeError> {
        Ok(Self {
            min_trailing_volume_atoms,
            schedule: FeeSchedule::new(taker_fee_bps, maker_fee_bps)?,
        })
    }
}

/// Validated, non-empty tier table with strictly increasing thresholds. The
/// first threshold is zero, so every trailing volume maps to exactly one tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeTierTable {
    tiers: Vec<FeeTier>,
}

impl FeeTierTable {
    pub fn new(tiers: Vec<FeeTier>) -> Result<Self, FeeError> {
        if tiers.is_empty() {
            return Err(FeeError::EmptyTierTable);
        }
        if tiers.len() > MAX_FEE_TIERS {
            return Err(FeeError::TooManyTiers { count: tiers.len() });
        }
        if tiers[0].min_trailing_volume_atoms != 0 {
            return Err(FeeError::FirstTierThresholdNonZero {
                threshold_atoms: tiers[0].min_trailing_volume_atoms,
            });
        }
        for (index, pair) in tiers.windows(2).enumerate() {
            let (previous, current) = (
                pair[0].min_trailing_volume_atoms,
                pair[1].min_trailing_volume_atoms,
            );
            if current == previous {
                return Err(FeeError::DuplicateTierThreshold { index: index + 1 });
            }
            if current < previous {
                return Err(FeeError::NonMonotonicTierThreshold { index: index + 1 });
            }
        }
        for tier in &tiers {
            tier.schedule.validate()?;
        }
        Ok(Self { tiers })
    }

    pub fn tiers(&self) -> &[FeeTier] {
        &self.tiers
    }

    /// Returns the schedule of the highest tier whose threshold is at or below
    /// `trailing_volume_atoms`. A pure lookup with no account state.
    pub fn schedule_for_volume(&self, trailing_volume_atoms: u128) -> &FeeSchedule {
        let index = self
            .tiers
            .partition_point(|tier| tier.min_trailing_volume_atoms <= trailing_volume_atoms);
        // The first threshold is zero, so `index >= 1`. saturating_sub keeps
        // the lookup panic-free even if that invariant were broken.
        &self.tiers[index.saturating_sub(1)].schedule
    }
}
