//! TradeLayer transaction census, derived from the legacy JS dispatcher and
//! zkTL's May Cairo/STWO protocol sweep. Descriptions are *migration planning*
//! metadata. Neither JS prototype coverage nor Rust type recognition implies
//! an activated consensus handler or a proven state transition.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStage {
    /// Only standalone accounting operations, not chain consensus rules.
    RustLedgerPrimitive,
    /// Standalone registry state transitions, without chain authentication.
    RustRegistryPrimitive,
    /// zkTL has a JavaScript transition prototype; full Cairo proof absent.
    ZkTlJsPrototype,
    /// Source-mapping or unported logic.
    Planned,
    /// zkTL and legacy TradeLayer assign incompatible meanings to the ID.
    SemanticConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionSpec {
    pub number: u8,
    pub name: &'static str,
    pub stage: PortStage,
}

pub const TRANSACTION_CATALOG: [TransactionSpec; 36] = [
    TransactionSpec {
        number: 0,
        name: "Activate TradeLayer",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 1,
        name: "Token Issue",
        stage: PortStage::RustLedgerPrimitive,
    },
    TransactionSpec {
        number: 2,
        name: "Send",
        stage: PortStage::RustLedgerPrimitive,
    },
    TransactionSpec {
        number: 3,
        name: "Trade Token for UTXO",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 4,
        name: "Commit Token",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 5,
        name: "On-chain Token for Token",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 6,
        name: "Cancel Order",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 7,
        name: "Create Whitelist",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 8,
        name: "Update Admin",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 9,
        name: "Issue or Revoke Attestation",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 10,
        name: "AMM Pool",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 11,
        name: "Grant Managed Token",
        stage: PortStage::RustLedgerPrimitive,
    },
    TransactionSpec {
        number: 12,
        name: "Redeem Managed Token",
        stage: PortStage::RustLedgerPrimitive,
    },
    TransactionSpec {
        number: 13,
        name: "Create Oracle",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 14,
        name: "Publish Oracle Data",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 15,
        name: "Close Oracle",
        stage: PortStage::RustRegistryPrimitive,
    },
    TransactionSpec {
        number: 16,
        name: "Create Futures Contract Series",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 17,
        name: "Exercise Derivative",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 18,
        name: "Trade Contract On-chain",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 19,
        name: "Trade Contract Channel",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 20,
        name: "Trade Tokens Channel",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 21,
        name: "Withdrawal",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 22,
        name: "Transfer",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 23,
        name: "Settle Channel PNL",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 24,
        name: "Mint Synthetic",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 25,
        name: "Redeem Synthetic",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 26,
        name: "Pay to Tokens",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 27,
        name: "Option Trade",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 28,
        name: "Bai Urbun",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 29,
        name: "Murabaha",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 30,
        name: "Stake Fraud Proof / State Oracle Relay",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 31,
        name: "King Settle",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 32,
        name: "Batch Move ZK Rollup",
        stage: PortStage::Planned,
    },
    TransactionSpec {
        number: 33,
        name: "Colored Coin",
        stage: PortStage::ZkTlJsPrototype,
    },
    TransactionSpec {
        number: 34,
        name: "Cross-Layer Bridge (legacy) / ZK Batch Move (zkTL)",
        stage: PortStage::SemanticConflict,
    },
    TransactionSpec {
        number: 35,
        name: "Bind Smart Contract",
        stage: PortStage::Planned,
    },
];

pub fn lookup_transaction(number: u8) -> Option<&'static TransactionSpec> {
    TRANSACTION_CATALOG.get(usize::from(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_legacy_type_has_one_entry() {
        assert_eq!(TRANSACTION_CATALOG.len(), 36);
        for (id, entry) in TRANSACTION_CATALOG.iter().enumerate() {
            assert_eq!(usize::from(entry.number), id);
            assert!(!entry.name.is_empty());
        }
        assert!(lookup_transaction(36).is_none());
    }

    #[test]
    fn semantic_collision_remains_blocked() {
        assert_eq!(
            lookup_transaction(34).unwrap().stage,
            PortStage::SemanticConflict
        );
        assert_eq!(lookup_transaction(32).unwrap().name, "Batch Move ZK Rollup");
    }
}
