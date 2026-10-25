#!/usr/bin/env node
"use strict";

/*
 * Standalone source-behavior oracle for pinned
 * tradelayer.js@dca0888b src/activation.js:35-43,117-226,230-268.
 *
 * This reproduces exact relevant JavaScript expressions with NO network,
 * wallet, DB, imported legacy dependencies or side effects. It does not
 * claim the saved activation list was accepted by any historical chain.
 */
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const tsv = fs.readFileSync(
    path.join(__dirname, "../fixtures/legacy_activation_js_names.tsv"), "utf8"
);
const rows = tsv.split("\n")
    .filter(line => line && !line.startsWith("#"))
    .map(line => {
        const [id, name] = line.split("\t");
        assert.match(id, /^(0|[1-9][0-9]*)$/);
        return [Number(id), name];
    });

const admins = {
    "BTC:main": "bc1qktknrnx2jcchjht9anz0uy8ae02xryxq2vxeem",
    "BTC:test": "tb1q8f84erfegxhaylmvpfll9m5rgwymqy4akjnnvq",
    "LTC:main": "MTmoypkhRQoJ172ZqxcsVumPZfJ8KCrQCB",
    "LTC:test": "tltc1qa0kd2d39nmeph3hvcx8ytv65ztcywg5sazhtw8",
    "DOGE:main": "DLSfu9qvEggkeXAgCAwBBw5BVLvMCtkewz",
    "DOGE:test": "nop27JQWbGr95ySHXZMzCg8XXxYzbCBZAW"
};

function registryFromPinnedLabels() {
    return Object.fromEntries(rows.map(([id, name]) => [
        id, {name, active: false}
    ]));
}
function projectedActivate(reg, type, block, codeHash) {
    // Reproduces the NON-GENESIS arm of Activation.activate().
    // Type zero is intentionally excluded: JS also calls native-token init.
    assert.notEqual(type, 0);
    if (!reg[type]) return;
    reg[type].active = true;
    reg[type].activationBlock = block;
    if (codeHash) reg[type].codeHash = codeHash;
    return reg[type];
}
function isActive(reg, type) {
    return Boolean(reg[type]?.active === true);
}
function getActivationBlock(reg, type) {
    return reg.hasOwnProperty(type) ? reg[type].activationBlock || null : null;
}
function checkActivationBlock(reg, type) {
    if (!reg.hasOwnProperty(type)) return null;
    return reg[type].activationBlock === undefined ? null : reg[type].activationBlock;
}
function aboveThreshold(reg) {
    const total = Object.keys(reg).length;
    if (total === 0) return false;
    const activeCount = Object.values(reg)
        .reduce((n, tx) => tx.active ? n + 1 : n, 0);
    return activeCount / total * 100 > 90;
}

function main() {
    assert.equal(rows.length, 36);
    for (let index = 0; index < rows.length; index++) {
        assert.equal(rows[index][0], index);
        assert(rows[index][1]);
    }
    assert.deepEqual(Object.keys(admins).sort(), [
        "BTC:main", "BTC:test", "DOGE:main", "DOGE:test", "LTC:main", "LTC:test"
    ]);
    assert.equal(rows[31][1], "Batch Move Zk Rollup");
    assert.equal(rows[32][1], "Publish New Tx");

    const reg = registryFromPinnedLabels();
    assert.equal(aboveThreshold(reg), false);
    projectedActivate(reg, 7, 10, "abc123");
    assert.equal(isActive(reg, 7), true);
    assert.equal(reg[7].activationBlock, 10);
    projectedActivate(reg, 7, 11, undefined);
    assert.equal(reg[7].activationBlock, 11);
    assert.equal(reg[7].codeHash, "abc123");
    projectedActivate(reg, 7, 12, "");
    assert.equal(reg[7].codeHash, "abc123");
    projectedActivate(reg, 7, 13, "def456");
    assert.equal(reg[7].codeHash, "def456");

    projectedActivate(reg, 9, 0, "0");
    assert.equal(isActive(reg, 9), true);
    assert.equal(getActivationBlock(reg, 9), null);
    assert.equal(checkActivationBlock(reg, 9), 0);
    assert.equal(getActivationBlock(reg, 36), null);

    const threshold = registryFromPinnedLabels();
    for (let type = 1; type <= 32; type++) projectedActivate(threshold, type, 1);
    assert.equal(aboveThreshold(threshold), false);
    projectedActivate(threshold, 33, 1);
    assert.equal(aboveThreshold(threshold), true);
    assert.equal(Object.values(threshold).filter(x => x.active).length, 33);
    assert.equal(isActive(threshold, 0), false);
    assert.equal(isActive(threshold, 34), false);

    const sourceGenesisAdmin = admins["LTC:test"];
    assert(sourceGenesisAdmin.startsWith("tltc1"));
    assert.notEqual(admins["BTC:main"], sourceGenesisAdmin);

    console.log("PASS: pinned 36 registry names and six BTC/LTC/DOGE main/test admins");
    console.log("PASS: reactivation overwrites height, sticky codeHash and block-zero getter");
    console.log("PASS: strict >90% threshold flips at exactly 33/36 active flags");
    console.log("BOUNDARY: no native-genesis tokens, historical activation state or chain permission");
}
main();
