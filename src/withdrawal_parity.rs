//! Read-only comparison model for the legacy JavaScript withdrawal queue.
//!
//! This independent, single-property/single-column projection exists to find
//! differences against the Stage 10 reserving engine. It does not mutate the
//! protocol ledger, decode transactions, or prove historical consensus.
//!
//! Source-derived subset:
//! - validity.js checks the current column balance when a request is accepted;
//! - channels.js queues the request WITHOUT deducting or reserving the amount;
//! - at H+7 or later, processWithdrawals iterates requests in queue order;
//! - each request settles if the CURRENT balance covers it, otherwise it is
//!   discarded; the block's channel operations run before the maturity pass.
//!
//! The JS has further TallyMap reserve-shortfall and withdraw-all behavior.
//! This model assumes the aggregate reserve covers the column balance and
//! intentionally leaves the problematic special cases outside its scope.

use crate::ledger::TxId;
use std::collections::{BTreeSet, VecDeque};

pub const WITHDRAWAL_DELAY: u64 = 7;
pub const MAX_SHADOW_QUEUE: usize = 100_000;
pub const MAX_SHADOW_BLOCK_EVENTS: usize = 10_000;

/// Events have already been normalized to atomic units for one token and one
/// channel participant. This model never trusts, signs, or broadcasts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowEvent {
    /// Deposit available balance into the modeled channel, like type 4.
    Commit(u128),
    /// Move escrow to a distinct channel, like type 22.
    TransferOut(u128),
    /// Move previously transferred funds back into this channel.
    TransferBack(u128),
    /// Type 21 requests only enqueue intent; overlapping claims are possible.
    Request { txid: TxId, amount_atoms: u128 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowVerdict {
    Paid,
    DiscardedInsufficient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowResolution {
    pub txid: TxId,
    pub amount_atoms: u128,
    pub verdict: ShadowVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedRequest {
    txid: TxId,
    amount_atoms: u128,
    maturity_height: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowError {
    InvalidHeight,
    NonSequentialBlock,
    ZeroAmount,
    InsufficientAvailable,
    InsufficientEscrow,
    InsufficientOutside,
    DuplicateRequest,
    QueueLimit,
    EventLimit,
    ArithmeticOverflow,
    SupplyMismatch,
}

impl core::fmt::Display for ShadowError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ShadowError {}

/// Independent projection. The outside balance counts the modeled owner's funds
/// moved to another channel. All three quantities must sum to issued_atoms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyWithdrawalShadow {
    issued_atoms: u128,
    available_atoms: u128,
    channel_atoms: u128,
    outside_atoms: u128,
    pending: VecDeque<QueuedRequest>,
    seen: BTreeSet<TxId>,
    last_block: Option<u64>,
}

impl LegacyWithdrawalShadow {
    /// Start with an issued balance in a single account and no channel funds.
    /// This standalone test oracle is not a real token issuance handler.
    pub fn new(issued_atoms: u128) -> Self {
        Self {
            issued_atoms,
            available_atoms: issued_atoms,
            channel_atoms: 0,
            outside_atoms: 0,
            pending: VecDeque::new(),
            seen: BTreeSet::new(),
            last_block: None,
        }
    }
    pub fn available_atoms(&self) -> u128 {
        self.available_atoms
    }
    pub fn channel_atoms(&self) -> u128 {
        self.channel_atoms
    }
    pub fn outside_atoms(&self) -> u128 {
        self.outside_atoms
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn last_block(&self) -> Option<u64> {
        self.last_block
    }

    pub fn verify_conservation(&self) -> Result<(), ShadowError> {
        let sum = self
            .available_atoms
            .checked_add(self.channel_atoms)
            .and_then(|v| v.checked_add(self.outside_atoms))
            .ok_or(ShadowError::ArithmeticOverflow)?;
        if sum != self.issued_atoms {
            return Err(ShadowError::SupplyMismatch);
        }
        Ok(())
    }

    /// Advance one complete L1 block. All events execute before matured
    /// requests are evaluated, matching the ordering in src/main.js.
    /// Errors leave the previous state untouched. This atomicity is a safety
    /// property of the oracle, NOT a claim about JS's actual DB transactions.
    pub fn apply_block(
        &mut self,
        height: u64,
        events: &[ShadowEvent],
    ) -> Result<Vec<ShadowResolution>, ShadowError> {
        if height == 0 {
            return Err(ShadowError::InvalidHeight);
        }
        if self
            .last_block
            .is_some_and(|previous| previous.checked_add(1) != Some(height))
        {
            return Err(ShadowError::NonSequentialBlock);
        }
        if events.len() > MAX_SHADOW_BLOCK_EVENTS {
            return Err(ShadowError::EventLimit);
        }
        let mut staged = self.clone();
        for event in events {
            staged.apply_one(height, *event)?;
        }
        let mut resolutions = Vec::new();
        let pending_count = staged.pending.len();
        for _ in 0..pending_count {
            let request = staged.pending.pop_front().expect("bounded queue length");
            if height < request.maturity_height {
                staged.pending.push_back(request);
                continue;
            }
            let verdict = if staged.channel_atoms >= request.amount_atoms {
                staged.channel_atoms -= request.amount_atoms;
                staged.available_atoms = staged
                    .available_atoms
                    .checked_add(request.amount_atoms)
                    .ok_or(ShadowError::ArithmeticOverflow)?;
                ShadowVerdict::Paid
            } else {
                ShadowVerdict::DiscardedInsufficient
            };
            resolutions.push(ShadowResolution {
                txid: request.txid,
                amount_atoms: request.amount_atoms,
                verdict,
            });
        }
        staged.last_block = Some(height);
        staged.verify_conservation()?;
        *self = staged;
        Ok(resolutions)
    }

    fn apply_one(&mut self, height: u64, event: ShadowEvent) -> Result<(), ShadowError> {
        let amount = match event {
            ShadowEvent::Commit(atoms)
            | ShadowEvent::TransferOut(atoms)
            | ShadowEvent::TransferBack(atoms) => atoms,
            ShadowEvent::Request { amount_atoms, .. } => amount_atoms,
        };
        if amount == 0 {
            return Err(ShadowError::ZeroAmount);
        }
        match event {
            ShadowEvent::Commit(atoms) => {
                if self.available_atoms < atoms {
                    return Err(ShadowError::InsufficientAvailable);
                }
                self.available_atoms -= atoms;
                self.channel_atoms = self
                    .channel_atoms
                    .checked_add(atoms)
                    .ok_or(ShadowError::ArithmeticOverflow)?;
            }
            ShadowEvent::TransferOut(atoms) => {
                if self.channel_atoms < atoms {
                    return Err(ShadowError::InsufficientEscrow);
                }
                self.channel_atoms -= atoms;
                self.outside_atoms = self
                    .outside_atoms
                    .checked_add(atoms)
                    .ok_or(ShadowError::ArithmeticOverflow)?;
            }
            ShadowEvent::TransferBack(atoms) => {
                if self.outside_atoms < atoms {
                    return Err(ShadowError::InsufficientOutside);
                }
                self.outside_atoms -= atoms;
                self.channel_atoms = self
                    .channel_atoms
                    .checked_add(atoms)
                    .ok_or(ShadowError::ArithmeticOverflow)?;
            }
            ShadowEvent::Request { txid, amount_atoms } => {
                // Legacy validity.js checks escrow at submission but permits
                // multiple requests against the same tokens.
                if self.channel_atoms < amount_atoms {
                    return Err(ShadowError::InsufficientEscrow);
                }
                if self.pending.len() >= MAX_SHADOW_QUEUE {
                    return Err(ShadowError::QueueLimit);
                }
                let maturity_height =
                    height.checked_add(WITHDRAWAL_DELAY).ok_or(ShadowError::ArithmeticOverflow)?;
                if !self.seen.insert(txid) {
                    return Err(ShadowError::DuplicateRequest);
                }
                self.pending.push_back(QueuedRequest {
                    txid,
                    amount_atoms,
                    maturity_height,
                });
            }
        }
        Ok(())
    }
}
