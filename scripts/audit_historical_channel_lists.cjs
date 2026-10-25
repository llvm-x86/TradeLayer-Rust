#!/usr/bin/env node
'use strict';

/*
 * Stage25M: independent Node source-PREDICATE characterization for the
 * direct-only subset of pinned tradelayer.js@dca0888b.
 *
 * These are NOT executions of full legacy Validity: no DB, xpub, country,
 * property registry, oracle, activation, channel state or blockchain proof.
 * Independent checks of source semantics only:
 *   src/validity.js:552-563, 610-653  (type 4 sender-property and commit)
 *   src/validity.js:1898-1928         (type 19 OR across A and B lists)
 */
const assert = require('node:assert/strict');

function directState(events, at, listId, address) {
    let active = false;
    for (const event of events) {
        const before = event.height < at.height ||
            (event.height === at.height && event.index < at.index);
        if (before && event.listId === listId && event.address === address) {
            active = event.action === 'attest';
        }
    }
    return active;
}

function type4Direct(sender, propertyWhitelists, optionalCommitList, events, at) {
    // Scalar 0 is a source bypass. This model deliberately forbids coercion
    // of ambiguous arrays containing 0 into equivalent consensus rules.
    if (Array.isArray(propertyWhitelists) && propertyWhitelists.includes(0)) {
        throw new Error('ambiguous property whitelist array containing zero');
    }
    const propertyOK = propertyWhitelists === 0 ||
        (Array.isArray(propertyWhitelists) &&
         propertyWhitelists.some(id => directState(events, at, id, sender)));
    const commitOK = optionalCommitList == null || optionalCommitList === 0 ||
        directState(events, at, optionalCommitList, sender);
    return propertyOK && commitOK;
}

function type19Direct(a, b, listsA, listsB, events, at) {
    const ids = [...listsA, ...listsB];
    return ids.length === 0 || ids.some(id =>
        directState(events, at, id, a) || directState(events, at, id, b));
}

function main() {
    const history = [
        {height: 40, index: 1, listId: 7, address: 'alice', action: 'attest'},
        {height: 40, index: 2, listId: 9, address: 'alice', action: 'attest'},
        {height: 40, index: 3, listId: 11, address: 'bob', action: 'attest'},
        {height: 41, index: 1, listId: 7, address: 'alice', action: 'revoke'},
        {height: 41, index: 2, listId: 7, address: 'bob', action: 'attest'},
    ];
    assert.equal(type4Direct('alice', [7], 9, history, {height: 41, index: 1}), true);
    assert.equal(type4Direct('alice', [7], 9, history, {height: 41, index: 2}), false);
    assert.equal(type4Direct('bob', [7], 9, history, {height: 41, index: 3}), false);
    assert.equal(type4Direct('alice', 0, null, [], {height: 41, index: 3}), true);
    assert.equal(type4Direct('alice', 0, 7, [], {height: 41, index: 3}), false);
    assert.throws(() => type4Direct('alice', [0, 7], null, [], {height: 41, index: 3}));
    assert.equal(type19Direct('alice', 'bob', [7], [], history, {height: 41, index: 1}), true);
    assert.equal(type19Direct('alice', 'bob', [7], [], history, {height: 41, index: 2}), false);
    assert.equal(type19Direct('alice', 'bob', [7], [], history, {height: 41, index: 3}), true);
    assert.equal(type19Direct('alice', 'bob', [17], [11], history, {height: 41, index: 3}), true);
    assert.equal(type19Direct('alice', 'bob', [17], [18], history, {height: 41, index: 3}), false);
    assert.equal(type19Direct('alice', 'bob', [], [], history, {height: 41, index: 3}), true);
    console.log('PASS: pinned type4 sender-direct vs type19 either-party-direct predicates');
    console.log('PASS: same-block position, direct revoke, optional commit list and zero bypass');
    console.log('BOUNDARY: derived xpub/country/activation/history/channel revision unverified');
}

main();
