// Stage 25E2B: bounded, whole-book distressed closeout for an OFFLINE,
// unverified economic experiment. Included by clearing_sandbox.rs.
//
// This is NOT market liquidation: no orders are filled and no AMM/ADL is
// invoked. It cancels matched synthetic positions by simultaneously closing
// every open lot at an externally asserted mark. Full funding from existing
// margin and the ONE real insurer reserve is mandatory; unresolved losses
// default-deny. No payout can mint/burn or spend spot inventory.
//
// Economic policy note: this is an explicitly separate research rule,
// not the old JS's deployed treatment of seized margin, bankruptcy, IOUs,
// insurance or counterparty selection. Do not attach to authenticated replay
// without a ratified activation rule and proven price/position provenance.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DistressedCloseoutReceipt {
    pub from_price_e8: u64,
    pub to_price_e8: u64,
    pub new_height: u64,
    pub losing_margin_debited_atoms: u128,
    pub insurance_debited_atoms: u128,
    pub creditor_paid_atoms: u128,
    pub ledger_transfer_legs: usize,
    pub closed_owner_count: usize,
    pub distressed_owner_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistressedCloseoutPreview {
    pub losing_margin_debited_atoms: u128,
    pub insurance_required_atoms: u128,
    pub insurance_available_atoms: u128,
    pub creditor_paid_atoms: u128,
    pub distressed_owner_count: usize,
    pub closed_owner_count: usize,
    /// Nonzero means the requirement cannot be fulfilled, but the preview
    /// itself never changes any reserve or pledges insurance for a claim.
    pub uncovered_insurance_atoms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DistressedLeg {
    from: AccountId,
    to: AccountId,
    atoms: u128,
    insured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DistressedPlan {
    preview: DistressedCloseoutPreview,
    legs: Vec<DistressedLeg>,
}

impl ClearingSandbox {
    /// Public diagnostic only. Exposing this plan does NOT reserve insurer
    /// atoms or authorize any spend. The execution path must recompute the
    /// complete plan on its own staged single-ledger state.
    pub fn preview_distressed_closeout_unverified(
        &self,
        new_height: u64,
        new_price_e8: u64,
    ) -> Result<DistressedCloseoutPreview, ClearingError> {
        Ok(self
            .plan_distressed_closeout(new_height, new_price_e8, false)?
            .preview)
    }

    fn plan_distressed_closeout(
        &self,
        new_height: u64,
        new_price_e8: u64,
        require_insured_funding: bool,
    ) -> Result<DistressedPlan, ClearingError> {
        if self.insurance_policy.is_none() {
            return Err(ClearingError::InsurancePolicyMissing);
        }
        // The exact same Stage25C read-only mark and Stage25E1 maintenance
        // projections are used, including old-mark, queued-spot and rounding
        // guards. No separate market quote or loss estimate is trusted.
        let mark = self.preview_mark_to_market_unverified(new_height, new_price_e8)?;
        if mark.net_atoms != 0 {
            return Err(ClearingError::PnlRoundingImbalance(mark.net_atoms));
        }
        let risk = self.preview_liquidation_unverified(new_height, new_price_e8)?;
        if risk.candidates_by_severity.is_empty() {
            return Err(ClearingError::NoDistress);
        }

        // Deterministic payer and receiver ordering comes from the sorted
        // signed-position map. No commission, fee or synthetic PnL is made.
        let mut payers: Vec<(AccountId, u128, bool)> = Vec::new();
        let mut receivers: Vec<(AccountId, u128)> = Vec::new();
        let mut margins_debited = 0u128;
        let mut insurance_needed = 0u128;
        let mut credits = 0u128;

        for entry in &mark.deltas {
            if entry.delta_atoms == 0 {
                continue;
            }
            let escrow = self
                .margin_escrows
                .get(&entry.account)
                .ok_or(ClearingError::UnknownMarginOwner)?
                .clone();
            if entry.delta_atoms < 0 {
                let loss = entry.delta_atoms.unsigned_abs();
                let margin_charge = loss.min(self.margin_atoms(&entry.account));
                let shortfall = loss - margin_charge;
                margins_debited = margins_debited
                    .checked_add(margin_charge)
                    .ok_or(ClearingError::ArithmeticOverflow)?;
                insurance_needed = insurance_needed
                    .checked_add(shortfall)
                    .ok_or(ClearingError::ArithmeticOverflow)?;
                if margin_charge > 0 {
                    payers.push((escrow, margin_charge, false));
                }
            } else {
                let credit = u128::try_from(entry.delta_atoms)
                    .map_err(|_| ClearingError::ArithmeticOverflow)?;
                credits = credits
                    .checked_add(credit)
                    .ok_or(ClearingError::ArithmeticOverflow)?;
                receivers.push((escrow, credit));
            }
        }
        let available_insurance = self.insurance_reserve_atoms();
        let uncovered = insurance_needed.saturating_sub(available_insurance);
        let combined = margins_debited
            .checked_add(insurance_needed)
            .ok_or(ClearingError::ArithmeticOverflow)?;
        if combined != credits || credits != mark.receiver_atoms {
            return Err(ClearingError::OpenInterestImbalance);
        }
        let preview = DistressedCloseoutPreview {
            losing_margin_debited_atoms: margins_debited,
            insurance_required_atoms: insurance_needed,
            insurance_available_atoms: available_insurance,
            creditor_paid_atoms: credits,
            distressed_owner_count: risk.candidates_by_severity.len(),
            closed_owner_count: self.positions.len(),
            uncovered_insurance_atoms: uncovered,
        };
        // A diagnostic preview can describe uncovered claims. The mutation
        // cannot, under any conditions, underpay winners or create an IOU.
        if require_insured_funding && uncovered != 0 {
            return Err(ClearingError::InsufficientInsurance {
                required_atoms: insurance_needed,
                available_atoms: available_insurance,
            });
        }
        if insurance_needed > 0 {
            payers.push((self.derived_insurance_escrow()?, insurance_needed, true));
        }

        // Split loss-bearing escrows (sorted) then the real insurer (last)
        // into deterministic, strictly positive transfer edges to winners.
        // No source can also appear as a receiver for this same mark.
        let mut legs = Vec::new();
        let mut payer = 0usize;
        let mut receiver = 0usize;
        while payer < payers.len() && receiver < receivers.len() {
            let amount = payers[payer].1.min(receivers[receiver].1);
            if amount == 0 || payers[payer].0 == receivers[receiver].0 {
                return Err(ClearingError::EscrowCollision);
            }
            legs.push(DistressedLeg {
                from: payers[payer].0.clone(),
                to: receivers[receiver].0.clone(),
                atoms: amount,
                insured: payers[payer].2,
            });
            payers[payer].1 -= amount;
            receivers[receiver].1 -= amount;
            if payers[payer].1 == 0 {
                payer += 1;
            }
            if receivers[receiver].1 == 0 {
                receiver += 1;
            }
        }
        if payer != payers.len()
            || receiver != receivers.len()
            || legs.len() > (MAX_CLEARING_OWNERS * 2 + 1)
        {
            return Err(ClearingError::OpenInterestImbalance);
        }
        Ok(DistressedPlan { preview, legs })
    }

    fn distressed_closeout_inner(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
        failure_before_leg: Option<usize>,
    ) -> Result<DistressedCloseoutReceipt, ClearingError> {
        self.transaction(external, |staged| {
            let plan = staged.plan_distressed_closeout(new_height, new_price_e8, true)?;
            for (leg_number, leg) in plan.legs.iter().enumerate() {
                if failure_before_leg == Some(leg_number) {
                    return Err(ClearingError::InjectedFailure {
                        before_leg: leg_number,
                    });
                }
                let id = staged.internal_txid(
                    external,
                    b"insured-distressed-full-closeout",
                    leg_number,
                )?;
                if leg.insured {
                    staged.spot.apply_insured_closeout_leg_unverified(
                        id,
                        staged.collateral,
                        leg.from.clone(),
                        leg.to.clone(),
                        leg.atoms,
                    )?;
                } else {
                    staged.spot.apply_clearing_transfer_unverified(
                        id,
                        staged.collateral,
                        leg.from.clone(),
                        leg.to.clone(),
                        leg.atoms,
                    )?;
                }
            }
            if failure_before_leg == Some(plan.legs.len()) {
                return Err(ClearingError::InjectedFailure {
                    before_leg: plan.legs.len(),
                });
            }
            // All synthetic lots close together. Remaining posted margin is
            // still real locked collateral; it may be withdrawn only when the
            // owner is flat, via the pre-existing replay-checked path.
            staged.positions.clear();
            let old = staged.mark_policy.ok_or(ClearingError::MarkPolicyMissing)?;
            staged.mark_policy = Some(MarkPolicy {
                height: new_height,
                price_e8: new_price_e8,
                ..old
            });
            Ok(DistressedCloseoutReceipt {
                from_price_e8: old.price_e8,
                to_price_e8: new_price_e8,
                new_height,
                losing_margin_debited_atoms: plan.preview.losing_margin_debited_atoms,
                insurance_debited_atoms: plan.preview.insurance_required_atoms,
                creditor_paid_atoms: plan.preview.creditor_paid_atoms,
                ledger_transfer_legs: plan.legs.len(),
                closed_owner_count: plan.preview.closed_owner_count,
                distressed_owner_count: plan.preview.distressed_owner_count,
            })
        })
    }

    /// E2B full-book closeout at a synthetic mark. This is not an orderbook
    /// fill, authority to liquidate, or an interpretation of historical JS.
    /// Requires the ENTIRE shortfall covered by actual ~ins: atoms; the
    /// resulting one-ledger state must conserve supply and zero global OI.
    pub fn settle_distressed_closeout_unverified(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
    ) -> Result<DistressedCloseoutReceipt, ClearingError> {
        self.distressed_closeout_inner(external, new_height, new_price_e8, None)
    }

    /// Test-only boundary injection: faults before every transfer and just
    /// after the last transfer must leave all balances and markers unchanged.
    pub fn settle_distressed_closeout_fault_injected_unverified(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
        failure_before_leg: usize,
    ) -> Result<DistressedCloseoutReceipt, ClearingError> {
        self.distressed_closeout_inner(external, new_height, new_price_e8, Some(failure_before_leg))
    }
}
