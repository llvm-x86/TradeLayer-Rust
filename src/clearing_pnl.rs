//! Stage 25C: source-characterized mark-to-market PnL projection.
//!
//! No ledger or state ownership. The caller supplies signed open-interest,
//! price updates, scale, notional and a deliberately selected rounding mode.
//! This is an unverified numeric observation, not an economic authorization.
//! A nonzero rounded residual must be rejected by any spending caller;
//! silently minting, burning or allocating that remainder would introduce
//! an unratified economic consensus rule.

use crate::clearing_math::{mark_to_mark_pnl_atoms, ClearingMathError, PnlRounding};
use crate::ledger::AccountId;
use std::collections::BTreeMap;

pub const MAX_PNL_PARTICIPANTS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PnlError {
    InvalidMark,
    TooManyPositions,
    ArithmeticOverflow,
    Calculation(ClearingMathError),
}

impl core::fmt::Display for PnlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PnlError {}

impl From<ClearingMathError> for PnlError {
    fn from(error: ClearingMathError) -> Self {
        Self::Calculation(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PnlAccountDelta {
    pub account: AccountId,
    pub delta_atoms: i128,
}

/// \`net_atoms\` can be nonzero even with exactly zero net contract count:
/// separate per-address rounding is not an additive map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PnlProjection {
    pub from_price_e8: u64,
    pub to_price_e8: u64,
    pub notional_e8: u64,
    pub inverse: bool,
    pub rounding: PnlRounding,
    pub deltas: Vec<PnlAccountDelta>,
    pub payer_atoms: u128,
    pub receiver_atoms: u128,
    pub net_atoms: i128,
}

/// Compute a READ-ONLY price-change projection using the Stage 24A kernel.
///
/// \`contracts\` may be negative for short positions. \`price_e8\` and
/// \`notional_e8\` represent quantities with an explicit 1e-8 decimal scale.
/// All intermediate numeric errors fail closed. The caller must separately
/// verify the total OI, price authority and funding/collateral constraints.
pub fn project_pnl_unverified(
    positions: &BTreeMap<AccountId, i64>,
    from_price_e8: u64,
    to_price_e8: u64,
    notional_e8: u64,
    inverse: bool,
    rounding: PnlRounding,
) -> Result<PnlProjection, PnlError> {
    if positions.len() > MAX_PNL_PARTICIPANTS {
        return Err(PnlError::TooManyPositions);
    }
    if from_price_e8 == 0 || to_price_e8 == 0 || notional_e8 == 0 {
        return Err(PnlError::InvalidMark);
    }
    let mut net_atoms = 0i128;
    let mut payer_atoms = 0u128;
    let mut receiver_atoms = 0u128;
    let mut deltas = Vec::with_capacity(positions.len());
    for (account, &contracts) in positions {
        if contracts == 0 {
            return Err(PnlError::InvalidMark);
        }
        let delta_atoms = mark_to_mark_pnl_atoms(
            contracts,
            from_price_e8,
            to_price_e8,
            notional_e8,
            inverse,
            rounding,
        )?;
        net_atoms = net_atoms
            .checked_add(delta_atoms)
            .ok_or(PnlError::ArithmeticOverflow)?;
        if delta_atoms < 0 {
            payer_atoms = payer_atoms
                .checked_add(delta_atoms.unsigned_abs())
                .ok_or(PnlError::ArithmeticOverflow)?;
        } else {
            receiver_atoms = receiver_atoms
                .checked_add(u128::try_from(delta_atoms).map_err(|_| PnlError::ArithmeticOverflow)?)
                .ok_or(PnlError::ArithmeticOverflow)?;
        }
        deltas.push(PnlAccountDelta {
            account: account.clone(),
            delta_atoms,
        });
    }
    Ok(PnlProjection {
        from_price_e8,
        to_price_e8,
        notional_e8,
        inverse,
        rounding,
        deltas,
        payer_atoms,
        receiver_atoms,
        net_atoms,
    })
}
