// Stage 25F, deliberately minimal: a SINGLE synthetic bilateral reduction.
//
// This is not a unilateral liquidation, AMM fill, orderbook match, or
// authenticated trade. No maker precedence, bankruptcy, insurer waterfall,
// ADL, fee, or newly chosen rounding rule is accepted here.
//
// Both already-open parties MUST reduce strictly less than their existing
// opposite signed contract positions. PnL uses the EXISTING frozen
// MarkPolicy, measured between last global mark and a caller-declared
// positive execution price. This reference-mark convention is a research
// choice, NOT an asserted historical TradeLayer liquidation fill rule.
// All winning atoms must come from the counterparty's REAL margin escrow
// in the same SpotExchange-owned Ledger. No insurer or spot reserve debit.
//
// A single clone/check/swap covers the escrow transfer, both position
// reductions, the cross-lane replay marker and the immutable snapshot fence.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticBilateralReduction {
    pub long: AccountId,
    pub short: AccountId,
    pub contracts: u64,
    pub execution_price_e8: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticPartialReceipt {
    pub execution_height: u64,
    pub execution_price_e8: u64,
    pub reference_mark_price_e8: u64,
    pub contracts_reduced: u64,
    pub payer: Option<AccountId>,
    pub receiver: Option<AccountId>,
    pub realized_transfer_atoms: u128,
    pub transfer_legs: usize,
    pub remaining_long_contracts: i64,
    pub remaining_short_contracts: i64,
}

impl ClearingSandbox {
    fn synthetic_bilateral_inner(
        &mut self,
        external: TxId,
        height: u64,
        fill: &SyntheticBilateralReduction,
        fail_before_leg: Option<usize>,
    ) -> Result<SyntheticPartialReceipt, ClearingError> {
        // This boundary is intentionally NOT part of
        // settle_distressed_closeout_unverified or a trusted chain adapter.
        self.transaction(external, |staged| {
            Self::validate_owner(&fill.long)?;
            Self::validate_owner(&fill.short)?;
            if fill.long == fill.short {
                return Err(ClearingError::SelfTrade);
            }
            if fill.contracts == 0 {
                return Err(ClearingError::ZeroContracts);
            }
            if fill.execution_price_e8 == 0 {
                return Err(ClearingError::InvalidMarkPolicy);
            }
            let policy = staged.mark_policy.ok_or(ClearingError::MarkPolicyMissing)?;
            if height <= policy.height
                || staged
                    .spot
                    .processed_through()
                    .is_some_and(|tip| height <= tip)
            {
                return Err(ClearingError::StaleMark);
            }
            // No implicit spot/derivative maker priority is inferred.
            // Pending spot events at or before this height make a local
            // end-of-block price order ambiguous and must reject.
            if staged
                .spot
                .has_queued_spot_at_or_before_unverified(height)
            {
                return Err(ClearingError::PendingSpotOrders);
            }

            let long_before = staged
                .positions
                .get(&fill.long)
                .copied()
                .ok_or(ClearingError::MissingOppositePosition)?;
            let short_before = staged
                .positions
                .get(&fill.short)
                .copied()
                .ok_or(ClearingError::MissingOppositePosition)?;
            if long_before <= 0 || short_before >= 0 {
                return Err(ClearingError::MissingOppositePosition);
            }
            let q = i64::try_from(fill.contracts)
                .map_err(|_| ClearingError::PositionOverflow)?;
            // Exactly one strict partial pair; full closures, flips,
            // opening synthetic liquidity, or oversized claims do not
            // belong to the admitted Stage25F slice.
            if q >= long_before || fill.contracts >= short_before.unsigned_abs() {
                return Err(ClearingError::NotStrictPartial);
            }

            let pnl_long = crate::clearing_math::mark_to_mark_pnl_atoms(
                q,
                policy.price_e8,
                fill.execution_price_e8,
                policy.notional_e8,
                policy.inverse,
                policy.rounding,
            )
            .map_err(|err| ClearingError::Pnl(PnlError::Calculation(err)))?;
            let pnl_short = crate::clearing_math::mark_to_mark_pnl_atoms(
                -q,
                policy.price_e8,
                fill.execution_price_e8,
                policy.notional_e8,
                policy.inverse,
                policy.rounding,
            )
            .map_err(|err| ClearingError::Pnl(PnlError::Calculation(err)))?;
            let residual = pnl_long
                .checked_add(pnl_short)
                .ok_or(ClearingError::ArithmeticOverflow)?;
            if residual != 0 {
                return Err(ClearingError::PnlRoundingImbalance(residual));
            }
            let amount = pnl_long.unsigned_abs();
            let (payer, receiver) = if pnl_long < 0 {
                (Some(fill.long.clone()), Some(fill.short.clone()))
            } else if pnl_long > 0 {
                (Some(fill.short.clone()), Some(fill.long.clone()))
            } else {
                (None, None)
            };
            let legs = usize::from(amount != 0);
            if amount > 0 {
                let owner = payer.as_ref().ok_or(ClearingError::ArithmeticOverflow)?;
                let available = staged.margin_atoms(owner);
                if available < amount {
                    return Err(ClearingError::InsufficientMargin {
                        account: owner.clone(),
                        required_atoms: amount,
                        available_atoms: available,
                    });
                }
                let from = staged
                    .margin_escrows
                    .get(owner)
                    .ok_or(ClearingError::UnknownMarginOwner)?
                    .clone();
                let to = staged
                    .margin_escrows
                    .get(receiver.as_ref().ok_or(ClearingError::ArithmeticOverflow)?)
                    .ok_or(ClearingError::UnknownMarginOwner)?
                    .clone();
                if fail_before_leg == Some(0) {
                    return Err(ClearingError::InjectedFailure { before_leg: 0 });
                }
                staged.spot.apply_clearing_transfer_unverified(
                    staged.internal_txid(external, b"stage25f-synthetic-pair", 0)?,
                    staged.collateral,
                    from,
                    to,
                    amount,
                )?;
            }
            // After the sole possible transfer (or zero transfer), do
            // not publish even positions/replay markers on late failure.
            if fail_before_leg == Some(legs) {
                return Err(ClearingError::InjectedFailure { before_leg: legs });
            }
            let long_after = long_before
                .checked_sub(q)
                .ok_or(ClearingError::PositionOverflow)?;
            let short_after = short_before
                .checked_add(q)
                .ok_or(ClearingError::PositionOverflow)?;
            if long_after <= 0 || short_after >= 0 {
                return Err(ClearingError::NotStrictPartial);
            }
            staged.positions.insert(fill.long.clone(), long_after);
            staged.positions.insert(fill.short.clone(), short_after);
            // transaction() runs verify() on the staged SAME Ledger
            // before committing: maintenance floors, spot reserves,
            // net open interest, supply and escrow ownership.
            Ok(SyntheticPartialReceipt {
                execution_height: height,
                execution_price_e8: fill.execution_price_e8,
                reference_mark_price_e8: policy.price_e8,
                contracts_reduced: fill.contracts,
                payer,
                receiver,
                realized_transfer_atoms: amount,
                transfer_legs: legs,
                remaining_long_contracts: long_after,
                remaining_short_contracts: short_after,
            })
        })
    }

    /// Strictly bounded research-only counterparty matching with explicit
    /// price and two already existing opposite signed lots. Not a
    /// liquidator privilege; the counterparty match is entirely synthetic.
    pub fn reduce_bilateral_positions_unverified(
        &mut self,
        external: TxId,
        execution_height: u64,
        fill: &SyntheticBilateralReduction,
    ) -> Result<SyntheticPartialReceipt, ClearingError> {
        self.synthetic_bilateral_inner(external, execution_height, fill, None)
    }

    /// Research fault injection at boundary zero and, if a positive payout
    /// exists, after the transfer but before the position updates.
    pub fn reduce_bilateral_fault_injected_unverified(
        &mut self,
        external: TxId,
        execution_height: u64,
        fill: &SyntheticBilateralReduction,
        fail_before_leg: usize,
    ) -> Result<SyntheticPartialReceipt, ClearingError> {
        self.synthetic_bilateral_inner(
            external,
            execution_height,
            fill,
            Some(fail_before_leg),
        )
    }
}
