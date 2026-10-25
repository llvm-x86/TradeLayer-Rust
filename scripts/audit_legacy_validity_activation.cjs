#!/usr/bin/env node
"use strict";

/*
 * Pinned historical source-expression comparison, NOT a consensus adapter.
 * tradelayer.js@dca0888b:
 *   src/validity.js:83-94 -- the former per-height block check is COMMENTED;
 *                            active body always returns true.
 *   src/activation.js:245-255 -- isTxTypeActive reads a flag, no height.
 *   src/activation.js:257-263 -- checkActivationBlock distinguishes height 0.
 * Rust current: src/registry.rs:280-286 strictly requires tx height > activation.
 *
 * These controls execute the literal minimal source expressions but do NOT
 * instantiate the historical NeDB, singleton, chain client or dispatcher.
 * They demonstrate why these three distinct predicates are NOT exchangeable.
 */
const assert = require("node:assert/strict");

async function pinnedValidityIsActivated(_block, _txid, _txType) {
    // The earlier code in Validity.isActivated is a block comment.
    return true;
}

async function pinnedRegistryIsTxTypeActive(txRegistry, txTypeNumber) {
    const txType = txRegistry[txTypeNumber];
    if (txType === undefined) return false;
    if (txType.active == true) return true;
    return false;
}

function pinnedCheckActivationBlock(txRegistry, txTypeNumber) {
    const txType = txRegistry[txTypeNumber];
    if (txType.activationBlock == undefined) return null;
    return txType.activationBlock;
}

function stricterRustHeightRule(activation, height) {
    // Source characterisation only, not a call into Rust or full registry.
    return activation !== undefined && height > activation.activationBlock;
}

async function main() {
    const registry = {
        0: {active: false, activationBlock: 0},
        1: {active: false, activationBlock: 200},
        2: {active: true, activationBlock: 200},
        3: {active: true, activationBlock: 0}
    };

    for (const type of [0, 1, 2, 3, 35, 255]) {
        for (const height of [0, 199, 200, 201]) {
            assert.equal(
                await pinnedValidityIsActivated(height, "synthetic-txid", type),
                true,
                "pinned Validity helper returned false"
            );
        }
    }

    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 1), false);
    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 2), true);
    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 3), true);
    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 35), false);
    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 255), false);

    assert.equal(pinnedCheckActivationBlock(registry, 3), 0);
    assert.equal(stricterRustHeightRule(registry[2], 199), false);
    assert.equal(stricterRustHeightRule(registry[2], 200), false);
    assert.equal(stricterRustHeightRule(registry[2], 201), true);

    // Changing/overwriting the activation height reopens the discrepancy.
    registry[2].activationBlock = 250;
    assert.equal(await pinnedRegistryIsTxTypeActive(registry, 2), true);
    assert.equal(await pinnedValidityIsActivated(201, "synthetic-txid", 2), true);
    assert.equal(stricterRustHeightRule(registry[2], 201), false);

    // A naïve shortcut (registry flag OR universal Validity helper) would
    // silently turn a read-only source model into a permissive consensus rule.
    assert.equal(
        (await pinnedRegistryIsTxTypeActive(registry, 1)) ||
          (await pinnedValidityIsActivated(199, "synthetic-txid", 1)),
        true
    );

    console.log("PASS: pinned Validity.isActivated always true despite inactive/unknown registry type");
    console.log("PASS: registry flags and activation-by-height have different source semantics");
    console.log("PASS: same-block, prior-height and reactivation differences preserved");
    console.log("BOUNDARY: not a historical transaction validity or deployment oracle");
    console.log("BOUNDARY: Rust trusted registry remains separate and stricter");
}
main().catch(error => { console.error(error); process.exitCode = 1; });
