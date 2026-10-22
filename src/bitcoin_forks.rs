//! Bounded, offline selection of the greatest-work *observed* Bitcoin-mainnet
//! header branch. Every inserted header is independently checked against its
//! parent's Stage 17 work/MTP/difficulty state before any insertion commits.
//!
//! This is an audit-only header graph. It does not discover unseen branches,
//! authenticate an external checkpoint, validate complete blocks, or determine
//! the active Bitcoin Core chain. It must never implement TrustedChainAdapter
//! or TrustedProtocolAdapter or authorize TradeLayer settlement.

use crate::bitcoin_chainwork::{AnchorStatus, MainnetWorkChain, Work256, WorkError};
use crate::bitcoin_inclusion::MAX_HEADER_SEGMENT;
use crate::chain::{BlockId, Tip};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_OBSERVED_MAINNET_HEADERS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkError {
    InvalidCapacity,
    InvalidSegmentSize,
    CapacityExceeded,
    UnknownParent,
    DuplicateHeader,
    Header(WorkError),
}

impl std::fmt::Display for ForkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ForkError {}

impl From<WorkError> for ForkError {
    fn from(value: WorkError) -> Self {
        Self::Header(value)
    }
}

/// A local change in the greatest-work branch among the headers seen by this
/// one graph. The disconnected path runs tip to fork point; the connected path
/// runs fork point to tip. Neither path carries block economic validity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedBestChange {
    previous: Tip,
    selected: Tip,
    common_ancestor: Tip,
    disconnected: Vec<Tip>,
    connected: Vec<Tip>,
}

impl ObservedBestChange {
    pub const fn previous(&self) -> Tip {
        self.previous
    }

    pub const fn selected(&self) -> Tip {
        self.selected
    }

    pub const fn common_ancestor(&self) -> Tip {
        self.common_ancestor
    }

    pub fn disconnected(&self) -> &[Tip] {
        &self.disconnected
    }

    pub fn connected(&self) -> &[Tip] {
        &self.connected
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForkNode {
    parent: Option<BlockId>,
    chain: MainnetWorkChain,
}

/// A capped graph rooted in one MainnetWorkChain state. A published-genesis
/// root retains its known historical origin. A claimed external root stays
/// explicitly untrusted even if its descendants have substantial work.
///
/// The chosen tip is only greatest-work among *observed* valid headers.
/// Equal-work branches keep the incumbent selection deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMainnetForks {
    root: Tip,
    source: AnchorStatus,
    best: BlockId,
    max_headers: usize,
    nodes: BTreeMap<BlockId, ForkNode>,
}

impl ObservedMainnetForks {
    /// The root may be genesis, an already checked contiguous history, or an
    /// untrusted caller checkpoint. Provenance is inherited without promotion.
    /// The limit counts inserted descendants, excluding the root itself.
    pub fn new(root_chain: MainnetWorkChain, max_headers: usize) -> Result<Self, ForkError> {
        if !(1..=MAX_OBSERVED_MAINNET_HEADERS).contains(&max_headers) {
            return Err(ForkError::InvalidCapacity);
        }
        let root = root_chain.tip();
        let source = root_chain.anchor_status();
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root.id,
            ForkNode {
                parent: None,
                chain: root_chain,
            },
        );
        Ok(Self {
            root,
            source,
            best: root.id,
            max_headers,
            nodes,
        })
    }

    pub const fn root(&self) -> Tip {
        self.root
    }

    pub const fn anchor_status(&self) -> AnchorStatus {
        self.source
    }

    pub fn observed_header_count(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn best_observed_tip(&self) -> Tip {
        self.nodes[&self.best].chain.tip()
    }

    pub fn best_observed_work(&self) -> Work256 {
        self.nodes[&self.best].chain.checkpoint().cumulative_work
    }

    /// Check the audit-only chain state for a known header. A caller cannot
    /// inject arbitrary state here; only checked descendants are stored.
    pub fn chain_at(&self, id: BlockId) -> Option<&MainnetWorkChain> {
        self.nodes.get(&id).map(|node| &node.chain)
    }

    /// Adds a bounded, linked run of headers from any previously observed
    /// parent. Any PoW, MTP, difficulty, duplicate, or capacity rejection
    /// leaves the whole graph and chosen tip untouched.
    ///
    /// Returns a change only on strictly greater cumulative work. In a tie,
    /// the previously selected tip remains selected.
    pub fn observe_segment(
        &mut self,
        parent: BlockId,
        raw_headers: &[[u8; 80]],
    ) -> Result<Option<ObservedBestChange>, ForkError> {
        if raw_headers.is_empty() || raw_headers.len() > MAX_HEADER_SEGMENT {
            return Err(ForkError::InvalidSegmentSize);
        }
        if raw_headers.len() > self.max_headers - self.observed_header_count() {
            return Err(ForkError::CapacityExceeded);
        }
        let mut chain = self
            .nodes
            .get(&parent)
            .ok_or(ForkError::UnknownParent)?
            .chain
            .clone();
        let mut staged = Vec::with_capacity(raw_headers.len());
        let mut staged_ids = BTreeSet::new();
        let mut parent_id = parent;

        // Validation happens entirely in scratch state. Rechecking each
        // single header is bounded and preserves precise intermediate states
        // needed for descendants at a later fork point.
        for raw in raw_headers {
            let checked = chain.extend_checked(std::slice::from_ref(raw))?;
            let id = checked.tip().id;
            if self.nodes.contains_key(&id) || !staged_ids.insert(id) {
                return Err(ForkError::DuplicateHeader);
            }
            staged.push((
                id,
                ForkNode {
                    parent: Some(parent_id),
                    chain: chain.clone(),
                },
            ));
            parent_id = id;
        }

        let current_work = self.best_observed_work();
        let candidate_work = chain.checkpoint().cumulative_work;
        let change = if prefers_candidate(candidate_work, current_work) {
            let old_path = self.path_from_root(self.best);
            let mut new_path = self.path_from_root(parent);
            new_path.extend(staged.iter().map(|(_, node)| node.chain.tip()));
            Some(describe_change(&old_path, &new_path))
        } else {
            None
        };

        for (id, node) in staged {
            self.nodes.insert(id, node);
        }
        if change.is_some() {
            self.best = parent_id;
        }
        Ok(change)
    }

    fn path_from_root(&self, id: BlockId) -> Vec<Tip> {
        let mut result = Vec::new();
        let mut cursor = Some(id);
        while let Some(block) = cursor {
            let node = &self.nodes[&block];
            result.push(node.chain.tip());
            cursor = node.parent;
        }
        result.reverse();
        debug_assert_eq!(result.first(), Some(&self.root));
        result
    }
}

fn prefers_candidate(candidate: Work256, incumbent: Work256) -> bool {
    candidate > incumbent
}

fn describe_change(old: &[Tip], new: &[Tip]) -> ObservedBestChange {
    // Both paths are built from the same privately held graph root. No
    // external claim or unauthenticated ancestor enters this comparison.
    let common = old
        .iter()
        .zip(new)
        .take_while(|(a, b)| a == b)
        .count();
    assert!(common > 0, "paths must share their observed root");
    ObservedBestChange {
        previous: *old.last().expect("path has root"),
        selected: *new.last().expect("path has root"),
        common_ancestor: old[common - 1],
        disconnected: old[common..].iter().rev().copied().collect(),
        connected: new[common..].to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(height: u64, byte: u8) -> Tip {
        Tip {
            height,
            id: BlockId([byte; 32]),
        }
    }

    #[test]
    fn fork_point_and_detach_attach_order_are_unambiguous() {
        let base = tip(10, 1);
        let left = tip(11, 2);
        let right = tip(11, 3);
        let later = tip(12, 4);
        let reorg = describe_change(&[base, left], &[base, right, later]);
        assert_eq!(reorg.previous(), left);
        assert_eq!(reorg.selected(), later);
        assert_eq!(reorg.common_ancestor(), base);
        assert_eq!(reorg.disconnected(), &[left]);
        assert_eq!(reorg.connected(), &[right, later]);
        let extending = describe_change(&[base], &[base, right]);
        assert!(extending.disconnected().is_empty());
        assert_eq!(extending.connected(), &[right]);
    }

    #[test]
    fn strictly_greater_work_is_required_even_for_higher_height() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[23] = 1;
        b[24] = 255;
        let strong = Work256::from_be_bytes(a);
        let weak = Work256::from_be_bytes(b);
        assert!(prefers_candidate(strong, weak));
        assert!(!prefers_candidate(weak, strong));
        assert!(!prefers_candidate(strong, strong));
    }
}
