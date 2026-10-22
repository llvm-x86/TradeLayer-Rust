//! Canonical read-only state and transition commitments for future proving.
//!
//! These hashes bind local snapshots and consecutive reversible projections.
//! They do NOT authenticate L1 consensus, the adapter, settlement economics,
//! Cairo/STWO proof bytes, or zkTL's separately defined state-root namespaces.
//! The dependency-free SHA-256 implementation is not externally audited.

use crate::chain::{BlockId, ChainId, Tip};
use crate::codec::Cursor;
use crate::execution::{ProtocolSnapshotError, ProtocolState};
use crate::hash256::sha256;
use crate::unified_chain::UnifiedChainState;

const PROTOCOL_DOMAIN: &[u8] = b"TLR_PROTOCOL_STATE_V1\0";
const HEAD_DOMAIN: &[u8] = b"TLR_CHAIN_HEAD_V1\0";
const TRANSITION_MAGIC: &[u8; 5] = b"TLPT\x01";
pub const TRANSITION_BYTES: usize = 277;

#[derive(Debug)]
pub enum CommitmentError {
    Snapshot(ProtocolSnapshotError),
    Invalid(&'static str),
    NotSuccessive,
    WrongChain,
    MissingRollbackAncestor,
}
impl core::fmt::Display for CommitmentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CommitmentError {}

impl From<ProtocolSnapshotError> for CommitmentError {
    fn from(value: ProtocolSnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

/// Direct hashing over an explicitly domain-separated canonical snapshot.
/// Includes byte length to make changes of embedded snapshot framing explicit.
fn protocol_snapshot_root(snapshot: &[u8]) -> Result<[u8; 32], CommitmentError> {
    let size = u32::try_from(snapshot.len())
        .map_err(|_| CommitmentError::Invalid("oversized protocol snapshot"))?;
    let mut payload = Vec::with_capacity(PROTOCOL_DOMAIN.len() + 4 + snapshot.len());
    payload.extend_from_slice(PROTOCOL_DOMAIN);
    payload.extend_from_slice(&size.to_be_bytes());
    payload.extend_from_slice(snapshot);
    Ok(sha256(&payload))
}

/// Hash a bound network, block tip, and canonical TLPS root. The chain ID
/// must ultimately come from verified genesis configuration, not user input.
pub fn chain_head_root_unverified(
    chain: ChainId,
    tip: Tip,
    protocol_root: [u8; 32],
) -> [u8; 32] {
    let mut payload = Vec::with_capacity(HEAD_DOMAIN.len() + 32 + 8 + 32 + 32);
    payload.extend_from_slice(HEAD_DOMAIN);
    payload.extend_from_slice(&chain.0);
    payload.extend_from_slice(&tip.height.to_be_bytes());
    payload.extend_from_slice(&tip.id.0);
    payload.extend_from_slice(&protocol_root);
    sha256(&payload)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateCommitment {
    pub chain: ChainId,
    pub tip: Tip,
    pub protocol_root: [u8; 32],
    pub head_root: [u8; 32],
}

impl ProtocolState {
    /// Commits complete TLPS bytes, including registry, ledger, replay sets,
    /// finalized height, and channel/settlement journals when present.
    pub fn state_root_unverified(&self) -> Result<[u8; 32], CommitmentError> {
        protocol_snapshot_root(&self.snapshot_bytes()?)
    }
}

impl UnifiedChainState {
    /// Read-only projection. Calling this method signs or verifies nothing.
    pub fn state_commitment_unverified(&self) -> Result<StateCommitment, CommitmentError> {
        let chain = self.chain_id();
        let tip = self.tip();
        let protocol_root = self.state().state_root_unverified()?;
        Ok(StateCommitment {
            chain,
            tip,
            protocol_root,
            head_root: chain_head_root_unverified(chain, tip, protocol_root),
        })
    }
}

/// TLPT v1 fixed-length public-input candidate. All fields are private, and
/// decoding checks internal root agreement and SHA-256, but the entire record
/// could still be created by an attacker without a consensus/proof verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionStatement {
    chain: ChainId,
    parent: Tip,
    child: Tip,
    before: [u8; 32],
    after: [u8; 32],
    before_head: [u8; 32],
    after_head: [u8; 32],
    statement_id: [u8; 32],
}

impl TransitionStatement {
    /// Requires that successor.rollback_tip restores the exact preceding tip
    /// and protocol state, preserving the reversible projection's ancestry.
    /// This proves local rollback lineage, not L1 legitimacy.
    pub fn from_reversible_states_unverified(
        prior: &UnifiedChainState,
        successor: &UnifiedChainState,
    ) -> Result<Self, CommitmentError> {
        if prior.chain_id() != successor.chain_id() {
            return Err(CommitmentError::WrongChain);
        }
        if prior.tip().height.checked_add(1) != Some(successor.tip().height)
            || prior.tip().id == successor.tip().id
        {
            return Err(CommitmentError::NotSuccessive);
        }
        let mut restored = successor.clone();
        restored
            .rollback_tip(successor.tip())
            .map_err(|_| CommitmentError::MissingRollbackAncestor)?;
        if restored.tip() != prior.tip() || restored.state() != prior.state() {
            return Err(CommitmentError::MissingRollbackAncestor);
        }
        let before = prior.state_commitment_unverified()?;
        let after = successor.state_commitment_unverified()?;
        let mut statement = Self {
            chain: before.chain,
            parent: before.tip,
            child: after.tip,
            before: before.protocol_root,
            after: after.protocol_root,
            before_head: before.head_root,
            after_head: after.head_root,
            statement_id: [0; 32],
        };
        statement.statement_id = sha256(&statement.body_bytes());
        Ok(statement)
    }

    /// Versioned, fixed-layout binary metadata for proving experiments.
    /// This is not a STWO proof or authorization token.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = self.body_bytes();
        bytes.extend_from_slice(&self.statement_id);
        bytes
    }

    pub fn id(&self) -> [u8; 32] {
        self.statement_id
    }
    pub fn chain(&self) -> ChainId {
        self.chain
    }
    pub fn parent(&self) -> Tip {
        self.parent
    }
    pub fn child(&self) -> Tip {
        self.child
    }
    pub fn old_protocol_root(&self) -> [u8; 32] {
        self.before
    }
    pub fn new_protocol_root(&self) -> [u8; 32] {
        self.after
    }

    /// Parses only internally consistent metadata. Callers must still prove
    /// all source statements independently before relying on this record.
    pub fn from_canonical_bytes_unverified(input: &[u8]) -> Result<Self, CommitmentError> {
        if input.len() != TRANSITION_BYTES {
            return Err(CommitmentError::Invalid("transition byte length"));
        }
        let mut reader = Cursor::new(input);
        if reader
            .fixed::<5>()
            .map_err(|_| CommitmentError::Invalid("truncated transition magic"))?
            != *TRANSITION_MAGIC
        {
            return Err(CommitmentError::Invalid("transition magic/version"));
        }
        let chain = ChainId(reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("chain"))?);
        let parent = read_tip(&mut reader)?;
        let child = read_tip(&mut reader)?;
        let before = reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("old root"))?;
        let after = reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("new root"))?;
        let before_head = reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("old head"))?;
        let after_head = reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("new head"))?;
        let statement_id = reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("statement id"))?;
        reader.end().map_err(|_| CommitmentError::Invalid("trailing bytes"))?;

        if parent.height.checked_add(1) != Some(child.height) || parent.id == child.id {
            return Err(CommitmentError::NotSuccessive);
        }
        if before_head != chain_head_root_unverified(chain, parent, before)
            || after_head != chain_head_root_unverified(chain, child, after)
        {
            return Err(CommitmentError::Invalid("head root mismatch"));
        }
        let expected = sha256(&input[..TRANSITION_BYTES - 32]);
        if statement_id != expected {
            return Err(CommitmentError::Invalid("statement digest mismatch"));
        }
        Ok(Self {
            chain, parent, child, before, after,
            before_head, after_head, statement_id,
        })
    }

    fn body_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(TRANSITION_BYTES - 32);
        bytes.extend_from_slice(TRANSITION_MAGIC);
        bytes.extend_from_slice(&self.chain.0);
        push_tip(&mut bytes, self.parent);
        push_tip(&mut bytes, self.child);
        bytes.extend_from_slice(&self.before);
        bytes.extend_from_slice(&self.after);
        bytes.extend_from_slice(&self.before_head);
        bytes.extend_from_slice(&self.after_head);
        bytes
    }
}

fn push_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}

fn read_tip(reader: &mut Cursor<'_>) -> Result<Tip, CommitmentError> {
    let height = reader.u64().map_err(|_| CommitmentError::Invalid("tip height"))?;
    let id = BlockId(reader.fixed::<32>().map_err(|_| CommitmentError::Invalid("tip hash"))?);
    Ok(Tip { height, id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_root_binds_chain_tip_and_state() {
        let p = [9; 32];
        let root = chain_head_root_unverified(ChainId([1; 32]), Tip {
            height: 123, id: BlockId([2; 32]),
        }, p);
        assert_ne!(root, chain_head_root_unverified(ChainId([3; 32]), Tip {
            height: 123, id: BlockId([2; 32]),
        }, p));
        assert_ne!(root, chain_head_root_unverified(ChainId([1; 32]), Tip {
            height: 124, id: BlockId([2; 32]),
        }, p));
        assert_ne!(root, chain_head_root_unverified(ChainId([1; 32]), Tip {
            height: 123, id: BlockId([2; 32]),
        }, [8; 32]));
    }
}
