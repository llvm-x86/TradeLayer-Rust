//! Core protocol types for the incremental TradeLayer Rust migration.
//!
//! This crate intentionally implements no consensus transitions yet. In
//! particular, constructing a transaction type does not validate a transaction.

#![forbid(unsafe_code)]

pub mod wire;

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
