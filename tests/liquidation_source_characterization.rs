//! Pinned JS liquidation SOURCE characterization, not live execution.
//! All depth, marks, accounts and priority data here are synthetic inputs.
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::liquidation_source_characterization::{
    characterize_price_routes, characterize_weighted_avg_entry_e8,
    compare_avg_accounting_to_mark_pnl, estimate_amm_first_synthetic,
    historical_safe_price, synthetic_order_priority, FundingReferenceRoute,
    LiquidationSide, MarkRoute, SourceAuditError, SourceContractKind,
    SyntheticDepthLevel, SyntheticLevelOrigin, MAX_DIAGNOSTIC_BOOK_LEVELS,
    SOURCE_FUNDING_LOOKBACK_BLOCKS,
};

const ONE: u64 = 100_000_000;
fn amm(id: u64, px: u64, qty: u64, time: u64) -> SyntheticDepthLevel {
    SyntheticDepthLevel {
        id,
        origin: SyntheticLevelOrigin::Amm,
        price_e8: px,
        quantity_contracts: qty,
        block_time: time,
    }
}
fn ordinary(id: u64, px: u64, qty: u64, time: u64) -> SyntheticDepthLevel {
    SyntheticDepthLevel {
        id,
        origin: SyntheticLevelOrigin::OrdinaryOrder,
        price_e8: px,
        quantity_contracts: qty,
        block_time: time,
    }
}

#[test]
fn oracle_and_native_mark_events_are_distinct_from_funding_twap_and_vwap() {
    let oracle = characterize_price_routes(SourceContractKind::Oracle);
    let native = characterize_price_routes(SourceContractKind::Native);
    assert_eq!(oracle.mark, MarkRoute::OraclePriceEventAtBlock);
    assert_eq!(oracle.funding_reference, FundingReferenceRoute::OracleEqualWeightTwap);
    assert_eq!(native.mark, MarkRoute::NativeVolumeIndexBlockPrice);
    assert_eq!(native.funding_reference, FundingReferenceRoute::NativeVolumeWeightedVwap);
    assert_eq!(oracle.funding_lookback_blocks, SOURCE_FUNDING_LOOKBACK_BLOCKS);
    assert_eq!(native.funding_lookback_blocks, 192);
}

#[test]
fn safe_bankruptcy_boundary_is_inclusive_with_inverse_sides_reversed() {
    // Pinned clearing.js::_sweepAmmLiquidityFirst:
    // linear sell >= boundary, linear buy <=; inverse reverses both.
    for (side, inverse, lower_safe, higher_safe) in [
        (LiquidationSide::SellLong, false, false, true),
        (LiquidationSide::BuyShort, false, true, false),
        (LiquidationSide::SellLong, true, true, false),
        (LiquidationSide::BuyShort, true, false, true),
    ] {
        assert_eq!(historical_safe_price(side, inverse, 99 * ONE, 100 * ONE), lower_safe);
        assert_eq!(historical_safe_price(side, inverse, 101 * ONE, 100 * ONE), higher_safe);
        assert!(historical_safe_price(side, inverse, 100 * ONE, 100 * ONE));
        assert!(!historical_safe_price(side, inverse, 0, 100 * ONE));
        assert!(!historical_safe_price(side, inverse, 100 * ONE, 0));
    }
}

#[test]
fn amm_safe_price_sweep_precedes_better_ordinary_orders_without_consuming_them() {
    let levels = [
        ordinary(1, 130 * ONE, 25, 1), // better price but NOT AMM
        amm(2, 101 * ONE, 1, 6),
        amm(3, 105 * ONE, 2, 9),
        amm(4, 105 * ONE, 3, 2),
        amm(5, 98 * ONE, 8, 1), // unsafe against boundary
        ordinary(6, 110 * ONE, 10, 1),
    ];
    let prior = levels;
    let result = estimate_amm_first_synthetic(
        &levels, LiquidationSide::SellLong, false, 100 * ONE, 4
    ).unwrap();
    assert_eq!(result.estimated_amm_contracts, 4);
    assert_eq!(result.unmet_contracts, 0);
    assert_eq!(
        result.attempted_levels.iter().map(|t| (t.level_id,t.quantity_contracts)).collect::<Vec<_>>(),
        vec![(4, 3), (3, 1)],
    );
    assert_eq!(levels, prior); // immutable, never a market execution
    assert_eq!(
        synthetic_order_priority(&levels, LiquidationSide::SellLong).unwrap(),
        vec![1, 6],
    );
}

#[test]
fn inverse_safe_amm_can_leave_a_real_unfilled_remainder_for_unratified_matching() {
    let depth = [
        amm(1, 90 * ONE, 1, 1), // inverse short buy: unsafe
        amm(2, 101 * ONE, 2, 7), // safe
        amm(3, 100 * ONE, 1, 3), // exactly safe
        ordinary(4, 110 * ONE, 100, 1), // not converted into liquidity
    ];
    let r = estimate_amm_first_synthetic(
        &depth, LiquidationSide::BuyShort, true, 100 * ONE, 5
    ).unwrap();
    assert_eq!(r.estimated_amm_contracts, 3);
    assert_eq!(r.unmet_contracts, 2);
    assert_eq!(r.attempted_levels.iter().map(|l|l.level_id).collect::<Vec<_>>(),vec![3,2]);
}

#[test]
fn price_time_ordering_is_side_specific_and_stable_for_exact_ties() {
    let b = [
        ordinary(6, 100 * ONE, 1, 4),
        ordinary(5, 101 * ONE, 1, 9),
        ordinary(4, 101 * ONE, 1, 2),
        ordinary(3, 101 * ONE, 1, 2),
        ordinary(2, 99 * ONE, 1, 1),
    ];
    assert_eq!(
        synthetic_order_priority(&b, LiquidationSide::SellLong).unwrap(),
        vec![4,3,5,6,2],
    );
    assert_eq!(
        synthetic_order_priority(&b, LiquidationSide::BuyShort).unwrap(),
        vec![2,6,4,3,5],
    );
}

#[test]
fn one_synthetic_level_cannot_be_counted_twice_or_bypass_bounds() {
    let duplicated = [amm(1, 100 * ONE, 1, 1),amm(1, 101 * ONE, 1, 2)];
    assert_eq!(
        estimate_amm_first_synthetic(
            &duplicated, LiquidationSide::SellLong, false, 100 * ONE, 1,
        ),
        Err(SourceAuditError::DuplicateLevelId)
    );
    assert_eq!(
        estimate_amm_first_synthetic(&[],LiquidationSide::SellLong,false,100 * ONE,0),
        Err(SourceAuditError::ZeroQuantity)
    );
    assert_eq!(
        estimate_amm_first_synthetic(&[],LiquidationSide::SellLong,false,0,1),
        Err(SourceAuditError::MissingPrice)
    );
    let over = vec![amm(1, ONE, 1, 1); MAX_DIAGNOSTIC_BOOK_LEVELS+1];
    assert_eq!(
        estimate_amm_first_synthetic(&over,LiquidationSide::SellLong,false,ONE,1),
        Err(SourceAuditError::TooManyLevels)
    );
}

#[test]
fn existing_weighted_average_entry_works_without_any_per_lot_tracking() {
    let avg = characterize_weighted_avg_entry_e8(2,90*ONE,1,120*ONE).unwrap();
    assert_eq!(avg,100*ONE);
    // 0.5 atomic price tick, the JS decimalPlaces(8) HALF_UP path.
    assert_eq!(
        characterize_weighted_avg_entry_e8(1,100*ONE,1,100*ONE+1).unwrap(),
        100*ONE+1
    );
    assert_eq!(
        characterize_weighted_avg_entry_e8(0,100*ONE,1,100*ONE),
        Err(SourceAuditError::ZeroQuantity)
    );
    assert_eq!(
        characterize_weighted_avg_entry_e8(1,100*ONE,1,0),
        Err(SourceAuditError::MissingPrice)
    );
    assert_eq!(
        characterize_weighted_avg_entry_e8(u64::MAX,u64::MAX,u64::MAX,u64::MAX),
        Err(SourceAuditError::ArithmeticOverflow)
    );
}

#[test]
fn avg_accounting_pnl_differs_from_incremental_mark_pnl_and_legacy_linear_notional() {
    // Pinned orderbook.js captures weighted avg before partial-close updates.
    // marginMap.realizePnl: avg->fill; settlePNL: lastMark->fill.
    // In the legacy LINEAR settlePNL branch, notional was not multiplied.
    let result = compare_avg_accounting_to_mark_pnl(
        1,90*ONE,100*ONE,101*ONE,2*ONE,false,PnlRounding::ExactRational
    ).unwrap();
    assert_eq!(result.avg_entry_accounting_pnl_atoms,22 * i128::from(ONE));
    assert_eq!(result.incremental_mark_pnl_atoms,2 * i128::from(ONE));
    assert_eq!(result.js_linear_settle_pnl_atoms,Some(i128::from(ONE)));
    let short = compare_avg_accounting_to_mark_pnl(
        -1,90*ONE,100*ONE,101*ONE,2*ONE,false,PnlRounding::ExactRational
    ).unwrap();
    assert_eq!(short.avg_entry_accounting_pnl_atoms,-result.avg_entry_accounting_pnl_atoms);
    assert_eq!(short.incremental_mark_pnl_atoms,-result.incremental_mark_pnl_atoms);
}

#[test]
fn inverse_and_sat_half_up_comparisons_remain_diagnostic_only() {
    let inverse = compare_avg_accounting_to_mark_pnl(
        1,100*ONE,110*ONE,120*ONE,100*ONE,true,PnlRounding::Legacy20DecimalPlaces
    ).unwrap();
    assert_eq!(inverse.js_linear_settle_pnl_atoms,None);
    assert!(inverse.avg_entry_accounting_pnl_atoms>inverse.incremental_mark_pnl_atoms);
    let half_sat = compare_avg_accounting_to_mark_pnl(
        1,100*ONE,100*ONE,100*ONE+1,ONE/2,false,PnlRounding::ExactRational
    ).unwrap();
    assert_eq!(half_sat.incremental_mark_pnl_atoms,1);
    assert_eq!(half_sat.avg_entry_accounting_pnl_atoms,1);
    // The pinned LINEAR settlePNL omits notional and returns one atom
    // here; the correct nominal-size mode is a different computation.
    assert_eq!(half_sat.js_linear_settle_pnl_atoms,Some(1));
    let negative = compare_avg_accounting_to_mark_pnl(
        -1,100*ONE,100*ONE,100*ONE+1,ONE/2,false,PnlRounding::ExactRational
    ).unwrap();
    assert_eq!(negative.incremental_mark_pnl_atoms,-1);
}
