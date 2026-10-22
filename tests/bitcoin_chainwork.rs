//! Stage 17: actual Bitcoin-mainnet genesis and block 1 plus explicitly
//! UNTRUSTED checkpoint/retarget metadata. No best-chain or script verification.

use std::cmp::Ordering;
use tradelayer_core::bitcoin_chainwork::{
    mainnet_block_proof_work, mainnet_retarget_bits, AnchorStatus, ClaimedMainnetCheckpoint,
    MainnetWorkChain, Work256, WorkError, MAINNET_ADJUSTMENT_INTERVAL, MAINNET_TARGET_TIMESPAN,
    MTP_WINDOW,
};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::read_only::decode_hex;

// Genuine, publicly documented Bitcoin mainnet genesis block header.
const GENESIS: &str = concat!(
    "01000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
    "29ab5f49ffff001d1dac2b7c",
);
// Genuine mainnet height 1 header published in the Bitcoin developer docs.
// Hash: 00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048.
const BLOCK_ONE: &str = concat!(
    "01000000",
    "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000",
    "982051fd1e4ba744bbbe680e1fee14677ba1a3c3540bf7b1cdb606e857233e0e",
    "61bc6649ffff001d01e36299",
);
fn header(s: &str) -> [u8; 80] {
    decode_hex(s).unwrap().try_into().unwrap()
}
fn hex(raw: &[u8]) -> String {
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn claimed_epoch_checkpoint(first_time: u32, last_time: u32) -> ClaimedMainnetCheckpoint {
    let mut work = [0u8; 32];
    work[24..].copy_from_slice(&4_295_032_833u64.to_be_bytes());
    ClaimedMainnetCheckpoint {
        tip: Tip {
            height: 2015,
            id: BlockId([0x88; 32]),
        },
        bits: 0x1d00ffff,
        timestamp: last_time,
        first_period_timestamp: first_time,
        prior_timestamps: vec![last_time; MTP_WINDOW],
        cumulative_work: Work256::from_be_bytes(work),
    }
}

#[test]
fn actual_genesis_and_block_one_validate_pow_link_mtp_and_accumulated_work() {
    let mut chain = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    assert_eq!(chain.anchor_status(), AnchorStatus::PublishedGenesis);
    assert_eq!(chain.tip().height, 0);
    assert_eq!(chain.checkpoint().bits, 0x1d00ffff);
    assert_eq!(chain.checkpoint().timestamp, 1231006505);
    assert_eq!(chain.checkpoint().prior_timestamps, vec![1231006505]);
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1d00ffff);
    assert_eq!(
        hex(&chain.checkpoint().cumulative_work.to_be_bytes()),
        "0000000000000000000000000000000000000000000000000000000100010001"
    );

    let previous = chain.clone();
    let seg = chain.extend_checked(&[header(BLOCK_ONE)]).unwrap();
    assert_eq!(seg.anchor(), previous.tip());
    assert_eq!(seg.tip(), chain.tip());
    assert_eq!(seg.tip().height, 1);
    assert_eq!(
        hex(&seg.tip().id.0),
        "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048"
    );
    assert_eq!(seg.headers()[0].n_bits(), 0x1d00ffff);
    assert_eq!(chain.checkpoint().timestamp, 1231469665);
    assert_eq!(chain.checkpoint().first_period_timestamp, 1231006505);
    assert_eq!(chain.checkpoint().prior_timestamps.len(), 2);
    assert_eq!(
        hex(&chain.checkpoint().cumulative_work.to_be_bytes()),
        "0000000000000000000000000000000000000000000000000000000200020002"
    );
    assert_eq!(
        previous.compare_work_since_same_anchor(&chain).unwrap(),
        Ordering::Less
    );
    assert_eq!(
        chain.compare_work_since_same_anchor(&previous).unwrap(),
        Ordering::Greater
    );
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1d00ffff);

    let same = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    assert_eq!(
        previous.compare_work_since_same_anchor(&same).unwrap(),
        Ordering::Equal
    );
    assert!(matches!(
        chain.compare_work_since_same_anchor(
            &MainnetWorkChain::from_claimed_checkpoint(claimed_epoch_checkpoint(
                1_000_000_000,
                1_000_604_800
            ))
            .unwrap()
        ),
        Err(WorkError::NotCommonCheckpoint)
    ));
}

#[test]
fn wrong_parent_or_pow_fails_atomically_after_real_genesis() {
    let mut chain = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    let before = chain.clone();
    let mut bad = header(BLOCK_ONE);
    bad[4] ^= 1; // header parent
    assert!(chain.extend_checked(&[bad]).is_err());
    assert_eq!(before, chain);

    let wrong = header(BLOCK_ONE);
    assert!(chain.extend_checked(&[wrong, wrong]).is_err());
    assert_eq!(before, chain);
    assert!(chain.extend_checked(&[]).is_err());
    assert_eq!(before, chain);
    assert!(chain.extend_checked(&[header(GENESIS)]).is_err());
    assert_eq!(before, chain);
    assert!(MainnetWorkChain::from_published_genesis(&header(BLOCK_ONE)).is_err());
}

#[test]
fn bitcoin_core_mainnet_retarget_and_clamp_regression() {
    assert_eq!(MAINNET_ADJUSTMENT_INTERVAL, 2016);
    assert_eq!(MAINNET_TARGET_TIMESPAN, 1_209_600);
    assert_eq!(MTP_WINDOW, 11);
    assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 0).unwrap(), 0x1c3fffc0);
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 302_400).unwrap(),
        0x1c3fffc0
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 604_800).unwrap(),
        0x1c7fff80
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 1_209_600).unwrap(),
        0x1d00ffff
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 9_676_800).unwrap(),
        0x1d00ffff
    );
    // For a harder *previous* epoch target, retargeting allows a 2x
    // reduction in difficulty and a maximum 4x adjustment.
    assert_eq!(
        mainnet_retarget_bits(0x1c00ffff, 0, 2_419_200).unwrap(),
        0x1c01fffe
    );
    assert_eq!(
        mainnet_retarget_bits(0x1c00ffff, 0, 4_838_400).unwrap(),
        0x1c03fffc
    );

    let anchored = claimed_epoch_checkpoint(1_000_000_000, 1_000_604_800);
    let chain = MainnetWorkChain::from_claimed_checkpoint(anchored.clone()).unwrap();
    assert_eq!(chain.anchor_status(), AnchorStatus::ClaimedExternal);
    assert_eq!(chain.tip().height, 2015);
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1c7fff80);
    assert_eq!(chain.checkpoint(), &anchored);
    assert_eq!(
        mainnet_block_proof_work(0x1d00ffff).unwrap().to_be_bytes()[24..],
        4_295_032_833u64.to_be_bytes()
    );
}

#[test]
fn forged_checkpoint_shape_and_cross_anchor_comparison_fail_closed() {
    let checkpoint = claimed_epoch_checkpoint(1_000_000_000, 1_000_604_800);
    let mut bogus = checkpoint.clone();
    bogus.prior_timestamps.pop();
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.first_period_timestamp = 0;
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.bits = 0x207fffff;
    assert!(MainnetWorkChain::from_claimed_checkpoint(bogus).is_err());
    bogus = checkpoint.clone();
    bogus.cumulative_work = Work256::from_be_bytes([0; 32]);
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.tip.height = 2016;
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.tip.id.0[0] ^= 1;
    let left = MainnetWorkChain::from_claimed_checkpoint(checkpoint).unwrap();
    let right = MainnetWorkChain::from_claimed_checkpoint(bogus).unwrap();
    assert!(matches!(
        left.compare_work_since_same_anchor(&right),
        Err(WorkError::NotCommonCheckpoint)
    ));
}


#[test]
fn twenty_five_python_bigint_vectors_cover_full_u256_chainwork_width() {
    let mut checked = 0;
    for row in include_str!("../fixtures/bitcoin_chainwork_vectors.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let (bits, expected) = row.split_once('\t').expect("two columns");
        let bits = u32::from_str_radix(bits, 16).unwrap();
        let actual = mainnet_block_proof_work(bits).unwrap();
        assert_eq!(hex(&actual.to_be_bytes()), expected);
        checked += 1;
    }
    assert_eq!(checked, 25);
    for bits in [0u32, 0x1d80ffff, 0x1d000000, 0x1e00ffff, 0x2100ffff] {
        assert!(mainnet_block_proof_work(bits).is_err(), "invalid bits: {bits:08x}");
    }
}
