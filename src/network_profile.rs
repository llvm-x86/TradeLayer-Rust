//! Stage 21: independent Bitcoin, Litecoin and Dogecoin TradeLayer mainnets.
//!
//! Each network is a separate TradeLayer with its own registry, balances,
//! protocol activation, transaction replay domain, state roots and checkpoints.
//! This module never constructs a shared multi-network ledger or bridges assets.
//!
//! Genesis block hashes are pinned in Bitcoin/Litecoin/Dogecoin Core's mainnet
//! chainparams (in standard big-endian display order). A high-height anchor
//! still requires independent historical authentication. This file only
//! identifies networks; it is neither a full-node validator nor a proof verifier.

use crate::chain::{BlockId, ChainId, Tip};
use crate::ledger::PropertyId;

/// The supported TradeLayer instances, each on its own mainnet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TradeLayerMainnet {
    Bitcoin = 1,
    Litecoin = 2,
    Dogecoin = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowFamily {
    BitcoinSha256d,
    LitecoinScrypt,
    DogecoinScryptAuxPow,
}

/// Describes available input paths in this repository, not full consensus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderEvidence {
    /// Stage 17/18: offline header-only work and observed-fork experiments.
    BitcoinHeaderResearchOnly,
    /// Requires a separately trusted node; no native Scrypt validation here.
    ExternalFullNodeRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaitoCairoStatus {
    /// Research/proving integration is Bitcoin-only; this crate does not
    /// verify Raito outputs or prove transaction economic validity.
    BitcoinSha256dResearchOnly,
    /// Scrypt/ROMix and Dogecoin AuxPoW lack a Raito/Cairo L1 verifier.
    UnavailableForScrypt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EconomicProofStatus {
    /// zkTL metadata can be audited, but there is no authenticated
    /// per-transaction Cairo/STWO economic proof verifier.
    MetadataAuditOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkProfileError {
    InvalidNetworkTag,
    WrongGenesisAnchor,
    BitcoinRaitoOnly,
}
impl std::fmt::Display for NetworkProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NetworkProfileError {}

/// Bitcoin Core src/kernel/chainparams.cpp at 152a5539 (mainnet).
pub const BITCOIN_MAINNET_GENESIS: BlockId = BlockId([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x19, 0xd6, 0x68, 0x9c, 0x08, 0x5a, 0xe1, 0x65, 0x83, 0x1e, 0x93, 0x4f, 0xf7, 0x63, 0xae, 0x46, 0xa2, 0xa6, 0xc1, 0x72, 0xb3, 0xf1, 0xb6, 0x0a, 0x8c, 0xe2, 0x6f,
]);

/// Litecoin Core src/chainparams.cpp at ec1b6489 (mainnet).
pub const LITECOIN_MAINNET_GENESIS: BlockId = BlockId([
    0x12, 0xa7, 0x65, 0xe3, 0x1f, 0xfd, 0x40, 0x59, 0xba, 0xda, 0x1e, 0x25, 0x19, 0x0f, 0x6e, 0x98, 0xc9, 0x9d, 0x97, 0x14, 0xd3, 0x34, 0xef, 0xa4, 0x1a, 0x19, 0x5a, 0x7e, 0x7e, 0x04, 0xbf, 0xe2,
]);

/// Dogecoin Core src/chainparams.cpp at 3c1679e0 (mainnet).
pub const DOGECOIN_MAINNET_GENESIS: BlockId = BlockId([
    0x1a, 0x91, 0xe3, 0xda, 0xce, 0x36, 0xe2, 0xbe, 0x3b, 0xf0, 0x30, 0xa6, 0x56, 0x79, 0xfe, 0x82, 0x1a, 0xa1, 0xd6, 0xef, 0x92, 0xe7, 0xc9, 0x90, 0x2e, 0xb3, 0x18, 0x18, 0x2c, 0x35, 0x56, 0x91,
]);

impl TradeLayerMainnet {
    /// Stable wire tag for the opt-in Stage 21 profile-bound checkpoint.
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Result<Self, NetworkProfileError> {
        match tag {
            1 => Ok(Self::Bitcoin),
            2 => Ok(Self::Litecoin),
            3 => Ok(Self::Dogecoin),
            _ => Err(NetworkProfileError::InvalidNetworkTag),
        }
    }

    pub const fn genesis(self) -> BlockId {
        match self {
            Self::Bitcoin => BITCOIN_MAINNET_GENESIS,
            Self::Litecoin => LITECOIN_MAINNET_GENESIS,
            Self::Dogecoin => DOGECOIN_MAINNET_GENESIS,
        }
    }

    /// Genesis-derived namespace; local property IDs can coincide numerically
    /// on different TradeLayers without identifying the same asset.
    /// BlockId stores canonical display-order bytes, like ChainId.
    pub const fn chain_id(self) -> ChainId {
        ChainId(self.genesis().0)
    }

    pub const fn pow(self) -> PowFamily {
        match self {
            Self::Bitcoin => PowFamily::BitcoinSha256d,
            Self::Litecoin => PowFamily::LitecoinScrypt,
            Self::Dogecoin => PowFamily::DogecoinScryptAuxPow,
        }
    }

    pub const fn header_evidence(self) -> HeaderEvidence {
        match self {
            Self::Bitcoin => HeaderEvidence::BitcoinHeaderResearchOnly,
            Self::Litecoin | Self::Dogecoin => HeaderEvidence::ExternalFullNodeRequired,
        }
    }

    pub const fn raito_cairo(self) -> RaitoCairoStatus {
        match self {
            Self::Bitcoin => RaitoCairoStatus::BitcoinSha256dResearchOnly,
            Self::Litecoin | Self::Dogecoin => RaitoCairoStatus::UnavailableForScrypt,
        }
    }

    pub const fn economic_proof(self) -> EconomicProofStatus {
        EconomicProofStatus::MetadataAuditOnly
    }

    /// This is eligibility for the external Bitcoin-only Raito research
    /// pipeline, not an assertion that a proof has been verified.
    pub fn require_raito_research_eligibility(self) -> Result<(), NetworkProfileError> {
        match self.raito_cairo() {
            RaitoCairoStatus::BitcoinSha256dResearchOnly => Ok(()),
            RaitoCairoStatus::UnavailableForScrypt => Err(NetworkProfileError::BitcoinRaitoOnly),
        }
    }

    /// Validates a genesis-height anchor against the pinned mainnet. A
    /// non-genesis anchor is always an UNTRUSTED caller declaration.
    pub fn check_genesis_if_applicable(self, anchor: Tip) -> Result<(), NetworkProfileError> {
        if anchor.height == 0 && anchor.id != self.genesis() {
            return Err(NetworkProfileError::WrongGenesisAnchor);
        }
        Ok(())
    }

    /// No profile in Stage 21 can turn a metadata hash into settlement
    /// authority. This returns false even on Bitcoin.
    pub const fn economic_proof_authorizes_settlement(self) -> bool {
        false
    }
}

/// Fully qualified local protocol asset identity. '7' on Litecoin and '7'
/// on Bitcoin remain distinct properties with independent issuer/supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkAssetId {
    network: TradeLayerMainnet,
    property: PropertyId,
}
impl NetworkAssetId {
    pub const fn new(network: TradeLayerMainnet, property: PropertyId) -> Self {
        Self { network, property }
    }
    pub const fn network(self) -> TradeLayerMainnet {
        self.network
    }
    pub const fn property(self) -> PropertyId {
        self.property
    }
    pub const fn belongs_to(self, network: TradeLayerMainnet) -> bool {
        self.network as u8 == network as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_identity_and_raito_policy_are_disjoint() {
        let all = [
            TradeLayerMainnet::Bitcoin,
            TradeLayerMainnet::Litecoin,
            TradeLayerMainnet::Dogecoin,
        ];
        for network in all {
            assert_eq!(TradeLayerMainnet::from_tag(network.tag()), Ok(network));
            assert_eq!(network.chain_id().0, network.genesis().0);
            assert_eq!(
                network.check_genesis_if_applicable(Tip {
                    height: 0,
                    id: network.genesis(),
                }),
                Ok(())
            );
            assert!(!network.economic_proof_authorizes_settlement());
        }
        assert_ne!(all[0].genesis(), all[1].genesis());
        assert_ne!(all[1].genesis(), all[2].genesis());
        assert_ne!(all[0].genesis(), all[2].genesis());
        assert_eq!(all[0].require_raito_research_eligibility(), Ok(()));
        assert_eq!(
            all[1].require_raito_research_eligibility(),
            Err(NetworkProfileError::BitcoinRaitoOnly)
        );
        assert_eq!(
            all[2].require_raito_research_eligibility(),
            Err(NetworkProfileError::BitcoinRaitoOnly)
        );
        assert_eq!(TradeLayerMainnet::from_tag(4), Err(NetworkProfileError::InvalidNetworkTag));
    }
}
