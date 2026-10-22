//! Stage 18: observed mainnet header branches, atomic admission and explicit
//! provenance. Published genesis through height 3 comes from public Bitcoin
//! block headers and is checked by an independent Python SHA256d audit.

use tradelayer_core::bitcoin_chainwork::{AnchorStatus, MainnetWorkChain};
use tradelayer_core::bitcoin_forks::{
    ForkError, ObservedMainnetForks, MAX_OBSERVED_MAINNET_HEADERS,
};
use tradelayer_core::chain::BlockId;
use tradelayer_core::read_only::decode_hex;

fn historical_headers() -> Vec<[u8; 80]> {
    include_str!("../fixtures/bitcoin_mainnet_genesis_block3_headers.txt")
        .lines()
        .map(|line| decode_hex(line).unwrap().try_into().unwrap())
        .collect()
}

fn from_genesis(capacity: usize) -> ObservedMainnetForks {
    let headers = historical_headers();
    let genesis = MainnetWorkChain::from_published_genesis(&headers[0]).unwrap();
    ObservedMainnetForks::new(genesis, capacity).unwrap()
}

#[test]
fn historical_blocks_1_to_3_select_strictly_more_observed_header_work() {
    let headers = historical_headers();
    assert_eq!(headers.len(), 4);
    let mut graph = from_genesis(3);
    assert_eq!(graph.anchor_status(), AnchorStatus::PublishedGenesis);
    assert_eq!(graph.root().height, 0);
    assert_eq!(graph.observed_header_count(), 0);
    let before = graph.best_observed_work();

    let first = graph
        .observe_segment(graph.root().id, &headers[1..2])
        .unwrap()
        .unwrap();
    assert_eq!(first.previous().height, 0);
    assert_eq!(first.selected().height, 1);
    assert_eq!(first.common_ancestor(), graph.root());
    assert!(first.disconnected().is_empty());
    assert_eq!(first.connected(), &[first.selected()]);
    assert!(graph.best_observed_work() > before);

    let second = graph
        .observe_segment(first.selected().id, &headers[2..4])
        .unwrap()
        .unwrap();
    assert_eq!(second.previous(), first.selected());
    assert_eq!(second.selected().height, 3);
    assert!(second.disconnected().is_empty());
    assert_eq!(second.connected().len(), 2);
    assert_eq!(second.connected()[0].height, 2);
    assert_eq!(second.connected()[1].height, 3);
    assert_eq!(graph.observed_header_count(), 3);
    assert_eq!(graph.best_observed_tip(), second.selected());
    assert_eq!(
        graph.chain_at(second.connected()[0].id).unwrap().tip().height,
        2
    );
    assert_eq!(
        graph.chain_at(second.connected()[1].id).unwrap().tip().height,
        3
    );
    assert_eq!(
        graph.chain_at(graph.root().id).unwrap().tip().height,
        0
    );
    assert!(graph.chain_at(BlockId([0x45; 32])).is_none());
    assert_eq!(
        graph.best_observed_work().to_be_bytes()[24..],
        (4 * 4_295_032_833u64).to_be_bytes()
    );
}

#[test]
fn incomplete_or_repeated_headers_leave_the_graph_atomic() {
    let headers = historical_headers();
    let mut graph = from_genesis(3);
    let snapshot = graph.clone();

    let mut bad_third = headers[3];
    bad_third[76] ^= 1; // genuine hash invalidated after two valid predecessors
    assert!(matches!(
        graph.observe_segment(graph.root().id, &[headers[1], headers[2], bad_third]),
        Err(ForkError::Header(_))
    ));
    assert_eq!(graph, snapshot);

    let mut wrong_parent = headers[1];
    wrong_parent[4] ^= 1;
    assert!(matches!(
        graph.observe_segment(graph.root().id, &[wrong_parent]),
        Err(ForkError::Header(_))
    ));
    assert_eq!(graph, snapshot);

    graph
        .observe_segment(graph.root().id, &headers[1..3])
        .unwrap();
    let saved = graph.clone();
    assert_eq!(
        graph.observe_segment(graph.root().id, &headers[1..2]),
        Err(ForkError::DuplicateHeader)
    );
    assert_eq!(graph, saved);
    assert_eq!(
        graph.observe_segment(graph.best_observed_tip().id, &headers[2..3]),
        Err(ForkError::Header(
            tradelayer_core::bitcoin_chainwork::WorkError::Bitcoin(
                tradelayer_core::bitcoin_inclusion::InclusionError::InvalidHeaderChain
            )
        ))
    );
    assert_eq!(graph, saved);
}

#[test]
fn unknown_parent_empty_input_and_capacity_limits_fail_closed() {
    let headers = historical_headers();
    assert_eq!(
        ObservedMainnetForks::new(
            MainnetWorkChain::from_published_genesis(&headers[0]).unwrap(),
            0
        ),
        Err(ForkError::InvalidCapacity)
    );
    assert_eq!(
        ObservedMainnetForks::new(
            MainnetWorkChain::from_published_genesis(&headers[0]).unwrap(),
            MAX_OBSERVED_MAINNET_HEADERS + 1
        ),
        Err(ForkError::InvalidCapacity)
    );

    let mut graph = from_genesis(2);
    let saved = graph.clone();
    assert_eq!(
        graph.observe_segment(graph.root().id, &[]),
        Err(ForkError::InvalidSegmentSize)
    );
    assert_eq!(
        graph.observe_segment(graph.root().id, &vec![headers[1]; 501]),
        Err(ForkError::InvalidSegmentSize)
    );
    assert_eq!(
        graph.observe_segment(BlockId([0x99; 32]), &headers[1..2]),
        Err(ForkError::UnknownParent)
    );
    assert_eq!(
        graph.observe_segment(graph.root().id, &headers[1..4]),
        Err(ForkError::CapacityExceeded)
    );
    assert_eq!(graph, saved);
    graph
        .observe_segment(graph.root().id, &headers[1..3])
        .unwrap();
    let saved = graph.clone();
    assert_eq!(
        graph.observe_segment(graph.best_observed_tip().id, &headers[3..4]),
        Err(ForkError::CapacityExceeded)
    );
    assert_eq!(graph, saved);
}

#[test]
fn claimed_external_checkpoint_stays_untrusted_after_honest_descendants() {
    let headers = historical_headers();
    let genuine = MainnetWorkChain::from_published_genesis(&headers[0]).unwrap();
    let claimed = MainnetWorkChain::from_claimed_checkpoint(genuine.checkpoint().clone()).unwrap();
    let mut graph = ObservedMainnetForks::new(claimed, 2).unwrap();
    graph
        .observe_segment(graph.root().id, &headers[1..3])
        .unwrap();
    assert_eq!(graph.best_observed_tip().height, 2);
    assert_eq!(graph.anchor_status(), AnchorStatus::ClaimedExternal);
    assert_eq!(
        graph.chain_at(graph.best_observed_tip().id)
            .unwrap()
            .anchor_status(),
        AnchorStatus::ClaimedExternal
    );
    assert!(graph.best_observed_work() > genuine.checkpoint().cumulative_work);
}
