//! Core protocol types for the incremental TradeLayer Rust migration.
//!
//! This crate intentionally implements no consensus transitions yet. In
//! particular, constructing a transaction type does not validate a transaction.

#![forbid(unsafe_code)]

mod atomic_batch; // Clone-validate-commit boundary and Kani rollback harnesses.
pub mod bitcoin_chainwork; // Bitcoin mainnet nBits/MTP/chainwork from explicit checkpoints; not full consensus.
pub mod bitcoin_forks; // Bounded greatest-work selection among observed header forks; no active-chain trust.
pub mod bitcoin_inclusion; // Bitcoin SHA256d header and tx Merkle checks; no full consensus.
mod canonical_columns; // Canonical nonzero map mutations for wallet and channel columns.
pub mod chain; // Trust-boundary and reversible block progression, prototype only.
mod checked_amounts; // Bit-precise checked debit/credit arithmetic; Kani proof harnesses.
pub mod clearing_cadence; // Stage 24C read-only JS clearing callback and funding schedule observations.
pub mod clearing_math; // Stage 24A diagnostic loss, funding and PnL equations; no ledger writes.
pub mod clearing_pnl; // Stage 25C read-only per-address PnL and residual projection.
pub mod clearing_sandbox; // Stage 25A isolated single-ledger derivative margin and funding research.
mod codec; // Bounded, checksummed snapshot codec.
pub mod execution; // Prototype unified registry+ledger replay; input unverified.
pub mod funding_audit; // Stage 24B read-only signed funding and conservation projections.
mod hash256; // Offline SHA-256d txid fingerprints; not trusted consensus.
pub mod joint; // Stage 22 independent registry + sole spot-owned ledger in one mixed state.
pub mod joint_chain; // Stage 22 mixed unverified block replay and TLJC checkpoints.
mod journal_move; // Fail-closed single-use settlement journal movement; Kani harness.
pub mod ledger; // In-memory atomic accounting primitives; not chain consensus.
pub mod network_bound; // Stage 21 profile-bound replay and versioned checkpoint envelopes; unverified.
pub mod network_profile; // Independent Bitcoin/Litecoin/Dogecoin mainnet identities and proof capabilities.
mod ordered_columns; // Sorted Vec backend for wallet and channel accounting columns.
pub mod protocol; // Audited legacy transaction catalog, not consensus execution.
pub mod read_only; // Offline transaction scanning; cannot authorize ledger state.
pub mod registry; // Standalone tx0/7-9/13-15 registry state; authentication external.
pub mod root_reconciliation; // Read-only TLPT and zkTL metadata consistency.
pub mod spot; // Stage 19 isolated type-5/6 rational matching experiment; unverified.
pub mod spot_chain; // Stage 20 typed unverified spot block replay and complete TLSP/TLSC checkpoints.
pub mod state_commitment; // Experimental domain-separated state and transition metadata; no proof verifier.
pub mod unified_chain; // Offline reversible registry+ledger block projection; adapter external.
pub mod wire; // Read-only legacy ASCII format analysis; never auto-fallback.
pub mod wire_v1;
pub mod withdrawal_parity; // Read-only JS-like deferred queue oracle for differential tests.
pub mod zk_batch; // zkTL/Cairo claim inspection; no proof verification or mutation. // Proposed versioned binary format.

/// Highest transaction type dispatched by the historical JavaScript
/// implementation when this scaffold was created.
pub const MAX_LEGACY_TX_TYPE: u8 = 35;

/// A transaction type known to the historical JavaScript dispatcher.
///
/// This is only a type-range check. It makes no claim about whether a
/// transaction is activated, authorized, well formed, or valid on-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionType(u8);

impl TransactionType {
    /// Returns the numeric transaction type identifier.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// Transaction type outside the range supported by the legacy dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownTransactionType(pub u8);

impl core::fmt::Display for UnknownTransactionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unknown legacy transaction type: {}", self.0)
    }
}

impl std::error::Error for UnknownTransactionType {}

impl TryFrom<u8> for TransactionType {
    type Error = UnknownTransactionType;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= MAX_LEGACY_TX_TYPE {
            Ok(Self(value))
        } else {
            Err(UnknownTransactionType(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_each_legacy_transaction_type() {
        for value in 0..=MAX_LEGACY_TX_TYPE {
            assert_eq!(TransactionType::try_from(value).unwrap().as_u8(), value);
        }
    }

    #[test]
    fn rejects_unknown_transaction_types() {
        assert_eq!(
            TransactionType::try_from(36),
            Err(UnknownTransactionType(36))
        );
        assert_eq!(
            TransactionType::try_from(u8::MAX),
            Err(UnknownTransactionType(u8::MAX))
        );
    }
}
