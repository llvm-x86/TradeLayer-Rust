//! Stage 21: opt-in, network-bound wrappers around two existing *independent*
//! replay engines. Each Bitcoin, Litecoin or Dogecoin TradeLayer has its own
//! ChainId, genesis, registry/ledger, replay IDs and checkpoint files.
//!
//! This layer only prevents accidental namespace/checkpoint cross-loading.
//! CRC32 is NOT authentication; anchors above genesis, raw transaction inputs
//! and economic state transitions remain UNVERIFIED unless external chain
//! consensus and an authorized protocol adapter establish them.
//!
//! The unified state and spot state are still DISTINCT experimental execution
//! lanes with separate ledgers. Never run both lanes against real funds or
//! treat one as a shared spendable state. They need a single-ledger migration.

use crate::chain::{CandidateBlock, ChainId, Tip};
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::hash256::sha256;
use crate::ledger::{AccountId, PropertyId};
use crate::network_profile::{NetworkProfileError, TradeLayerMainnet};
use crate::spot::SpotExchange;
use crate::spot_chain::{
    SpotChainCheckpointError, SpotChainError, SpotChainState, UnverifiedSpotBlock,
};
use crate::unified_chain::{
    TrustedProtocolAdapter, UnifiedChainError, UnifiedChainState, UnifiedCheckpointError,
};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 5] = b"TLNP\x01";
const FINGERPRINT_DOMAIN: &[u8] = b"TLR_PROFILE_CHECKPOINT_AUDIT_V1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReplayLane {
    UnifiedProtocol = 1,
    SpotResearch = 2,
}
impl ReplayLane {
    const fn tag(self) -> u8 {
        self as u8
    }
}

#[derive(Debug)]
pub enum ProfileReplayError {
    Profile(NetworkProfileError),
    Unified(UnifiedChainError),
    Spot(SpotChainError),
    UnifiedCheckpoint(UnifiedCheckpointError),
    SpotCheckpoint(SpotChainCheckpointError),
    Checkpoint(&'static str),
    Io(std::io::Error),
}
impl std::fmt::Display for ProfileReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProfileReplayError {}
impl From<NetworkProfileError> for ProfileReplayError {
    fn from(value: NetworkProfileError) -> Self {
        Self::Profile(value)
    }
}
impl From<UnifiedChainError> for ProfileReplayError {
    fn from(value: UnifiedChainError) -> Self {
        Self::Unified(value)
    }
}
impl From<SpotChainError> for ProfileReplayError {
    fn from(value: SpotChainError) -> Self {
        Self::Spot(value)
    }
}
impl From<UnifiedCheckpointError> for ProfileReplayError {
    fn from(value: UnifiedCheckpointError) -> Self {
        Self::UnifiedCheckpoint(value)
    }
}
impl From<SpotChainCheckpointError> for ProfileReplayError {
    fn from(value: SpotChainCheckpointError) -> Self {
        Self::SpotCheckpoint(value)
    }
}
impl From<std::io::Error> for ProfileReplayError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn invalid(reason: &'static str) -> ProfileReplayError {
    ProfileReplayError::Checkpoint(reason)
}

/// Checkpoint envelope includes its canonical network tag, pinned genesis,
/// derived ChainId, *lane*, original full checkpoint and CRC32.
fn wrap(
    network: TradeLayerMainnet,
    lane: ReplayLane,
    nested: &[u8],
) -> Result<Vec<u8>, ProfileReplayError> {
    let mut bytes = MAGIC.to_vec();
    bytes.push(network.tag());
    bytes.push(lane.tag());
    bytes.extend_from_slice(&network.genesis().0);
    bytes.extend_from_slice(&network.chain_id().0);
    bytes.extend_from_slice(
        &u32::try_from(nested.len())
            .map_err(|_| invalid("nested checkpoint too large"))?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(nested);
    with_crc(bytes).map_err(|_| invalid("profile checkpoint too large"))
}

fn unwrap<'a>(
    bytes: &'a [u8],
    expected_network: TradeLayerMainnet,
    expected_lane: ReplayLane,
) -> Result<&'a [u8], ProfileReplayError> {
    let body = verify_crc(bytes).map_err(|_| invalid("checkpoint CRC/length"))?;
    let mut reader = Cursor::new(body);
    if reader
        .fixed::<5>()
        .map_err(|_| invalid("missing profile magic"))?
        != *MAGIC
    {
        return Err(invalid("profile checkpoint version"));
    }
    let tag = reader.u8().map_err(|_| invalid("missing network tag"))?;
    let network = TradeLayerMainnet::from_tag(tag).map_err(|_| invalid("unknown network tag"))?;
    if network != expected_network {
        return Err(invalid("network checkpoint mismatch"));
    }
    let lane = reader.u8().map_err(|_| invalid("missing replay lane"))?;
    if lane != expected_lane.tag() {
        return Err(invalid("replay lane mismatch"));
    }
    let genesis = reader
        .fixed::<32>()
        .map_err(|_| invalid("genesis truncated"))?;
    let namespace = reader
        .fixed::<32>()
        .map_err(|_| invalid("chain namespace truncated"))?;
    if genesis != expected_network.genesis().0 || namespace != expected_network.chain_id().0 {
        return Err(invalid("invalid pinned genesis or chain namespace"));
    }
    let length = reader
        .u32()
        .map_err(|_| invalid("nested length truncated"))? as usize;
    if length == 0 || length > MAX_CHECKPOINT_BYTES {
        return Err(invalid("invalid nested checkpoint length"));
    }
    let nested = reader
        .take(length)
        .map_err(|_| invalid("nested checkpoint truncated"))?;
    reader
        .end()
        .map_err(|_| invalid("trailing profile checkpoint bytes"))?;
    if wrap(network, expected_lane, nested)?.as_slice() != bytes {
        return Err(invalid("noncanonical network envelope"));
    }
    Ok(nested)
}

fn write_new(path: &Path, data: &[u8]) -> Result<(), ProfileReplayError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(ProfileReplayError::Io(error));
    }
    Ok(())
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, ProfileReplayError> {
    let file = File::open(path)?;
    let size = usize::try_from(file.metadata()?.len())
        .map_err(|_| invalid("checkpoint file metadata oversized"))?;
    if size > MAX_CHECKPOINT_BYTES {
        return Err(invalid("checkpoint file oversized"));
    }
    let mut bytes = Vec::with_capacity(size);
    file.take((MAX_CHECKPOINT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(invalid("checkpoint file read oversized"));
    }
    Ok(bytes)
}

/// Local audit fingerprint, domain-separated by network, genesis and lane.
/// Like all TLPT/TLUC hashes, this grants NO economic or consensus authority.
fn fingerprint_unverified(checkpoint: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(FINGERPRINT_DOMAIN.len() + checkpoint.len());
    input.extend_from_slice(FINGERPRINT_DOMAIN);
    input.extend_from_slice(checkpoint);
    sha256(&input)
}

/// Independently owned registry + token ledger for exactly one TradeLayer.
/// The wrapped existing executor still needs a real trusted chain adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkBoundUnified {
    network: TradeLayerMainnet,
    inner: UnifiedChainState,
}
impl NetworkBoundUnified {
    pub fn new_unverified(
        network: TradeLayerMainnet,
        anchor: Tip,
        admin: AccountId,
        history_limit: usize,
    ) -> Result<Self, ProfileReplayError> {
        network.check_genesis_if_applicable(anchor)?;
        let inner =
            UnifiedChainState::new_unverified(network.chain_id(), anchor, admin, history_limit)?;
        Ok(Self { network, inner })
    }

    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub fn chain_id(&self) -> ChainId {
        self.network.chain_id()
    }
    pub fn tip(&self) -> Tip {
        self.inner.tip()
    }
    pub fn state(&self) -> &UnifiedChainState {
        &self.inner
    }
    pub fn rollback_depth(&self) -> usize {
        self.inner.rollback_depth()
    }

    pub fn apply_block<A: TrustedProtocolAdapter>(
        &mut self,
        block: &CandidateBlock,
        adapter: &A,
    ) -> Result<(), ProfileReplayError> {
        self.inner.apply_block(block, adapter)?;
        Ok(())
    }
    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), ProfileReplayError> {
        self.inner.rollback_tip(expected)?;
        Ok(())
    }
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, ProfileReplayError> {
        wrap(
            self.network,
            ReplayLane::UnifiedProtocol,
            &self.inner.checkpoint_bytes()?,
        )
    }
    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        network: TradeLayerMainnet,
        expected_anchor: Tip,
        expected_admin: &AccountId,
        expected_history_limit: usize,
    ) -> Result<Self, ProfileReplayError> {
        network.check_genesis_if_applicable(expected_anchor)?;
        let inner_bytes = unwrap(bytes, network, ReplayLane::UnifiedProtocol)?;
        let inner = UnifiedChainState::from_checkpoint_bytes(
            inner_bytes,
            network.chain_id(),
            expected_anchor,
            expected_admin,
            expected_history_limit,
        )?;
        Ok(Self { network, inner })
    }

    pub fn checkpoint_fingerprint_unverified(&self) -> Result<[u8; 32], ProfileReplayError> {
        Ok(fingerprint_unverified(&self.checkpoint_bytes()?))
    }
    pub fn write_new_checkpoint(&self, path: &Path) -> Result<(), ProfileReplayError> {
        write_new(path, &self.checkpoint_bytes()?)
    }
    pub fn load_checkpoint(
        path: &Path,
        network: TradeLayerMainnet,
        expected_anchor: Tip,
        expected_admin: &AccountId,
        history_limit: usize,
    ) -> Result<Self, ProfileReplayError> {
        Self::from_checkpoint_bytes(
            &read_bounded(path)?,
            network,
            expected_anchor,
            expected_admin,
            history_limit,
        )
    }
    pub fn apply_block_and_checkpoint<A: TrustedProtocolAdapter>(
        &mut self,
        block: &CandidateBlock,
        adapter: &A,
        path: &Path,
    ) -> Result<(), ProfileReplayError> {
        let mut staged = self.clone();
        staged.apply_block(block, adapter)?;
        staged.write_new_checkpoint(path)?;
        *self = staged;
        Ok(())
    }
}

/// Independent, experimental spot replay for exactly one TradeLayer.
/// It cannot be combined with NetworkBoundUnified as two live spending ledgers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkBoundSpot {
    network: TradeLayerMainnet,
    inner: SpotChainState,
}
impl NetworkBoundSpot {
    pub fn new_unverified(
        network: TradeLayerMainnet,
        anchor: Tip,
        spot: SpotExchange,
        history_limit: usize,
    ) -> Result<Self, ProfileReplayError> {
        network.check_genesis_if_applicable(anchor)?;
        let inner =
            SpotChainState::new_unverified(network.chain_id(), anchor, spot, history_limit)?;
        Ok(Self { network, inner })
    }
    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub fn chain_id(&self) -> ChainId {
        self.network.chain_id()
    }
    pub fn tip(&self) -> Tip {
        self.inner.tip()
    }
    pub fn state(&self) -> &SpotChainState {
        &self.inner
    }
    pub fn rollback_depth(&self) -> usize {
        self.inner.rollback_depth()
    }
    pub fn apply_unverified_block(
        &mut self,
        block: &UnverifiedSpotBlock,
    ) -> Result<(), ProfileReplayError> {
        self.inner.apply_unverified_block(block)?;
        Ok(())
    }
    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), ProfileReplayError> {
        self.inner.rollback_tip(expected)?;
        Ok(())
    }
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, ProfileReplayError> {
        wrap(
            self.network,
            ReplayLane::SpotResearch,
            &self.inner.checkpoint_bytes()?,
        )
    }
    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        network: TradeLayerMainnet,
        expected_anchor: Tip,
        history_limit: usize,
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, ProfileReplayError> {
        network.check_genesis_if_applicable(expected_anchor)?;
        let inner_bytes = unwrap(bytes, network, ReplayLane::SpotResearch)?;
        let inner = SpotChainState::from_checkpoint_bytes(
            inner_bytes,
            network.chain_id(),
            expected_anchor,
            history_limit,
            expected_permitted,
        )?;
        Ok(Self { network, inner })
    }
    pub fn checkpoint_fingerprint_unverified(&self) -> Result<[u8; 32], ProfileReplayError> {
        Ok(fingerprint_unverified(&self.checkpoint_bytes()?))
    }
    pub fn write_new_checkpoint(&self, path: &Path) -> Result<(), ProfileReplayError> {
        write_new(path, &self.checkpoint_bytes()?)
    }
    pub fn load_checkpoint(
        path: &Path,
        network: TradeLayerMainnet,
        expected_anchor: Tip,
        history_limit: usize,
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, ProfileReplayError> {
        Self::from_checkpoint_bytes(
            &read_bounded(path)?,
            network,
            expected_anchor,
            history_limit,
            expected_permitted,
        )
    }
    pub fn apply_unverified_block_and_checkpoint(
        &mut self,
        block: &UnverifiedSpotBlock,
        path: &Path,
    ) -> Result<(), ProfileReplayError> {
        let mut staged = self.clone();
        staged.apply_unverified_block(block)?;
        staged.write_new_checkpoint(path)?;
        *self = staged;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_envelope_versions_lanes_and_damaged_lengths() {
        let network = TradeLayerMainnet::Bitcoin;
        let bytes = wrap(network, ReplayLane::UnifiedProtocol, b"test").unwrap();
        assert_eq!(
            unwrap(&bytes, network, ReplayLane::UnifiedProtocol).unwrap(),
            b"test"
        );
        assert!(unwrap(&bytes, network, ReplayLane::SpotResearch).is_err());
        assert!(unwrap(
            &bytes,
            TradeLayerMainnet::Litecoin,
            ReplayLane::UnifiedProtocol
        )
        .is_err());
        let mut damaged = bytes.clone();
        damaged[7] ^= 1;
        assert!(unwrap(&damaged, network, ReplayLane::UnifiedProtocol).is_err());
        assert!(unwrap(
            &bytes[..bytes.len() - 1],
            network,
            ReplayLane::UnifiedProtocol
        )
        .is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(unwrap(&trailing, network, ReplayLane::UnifiedProtocol).is_err());
    }
}
