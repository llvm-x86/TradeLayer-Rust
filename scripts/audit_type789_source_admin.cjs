#!/usr/bin/env node
"use strict";
/*
 * Stage25P pinned source diagnostic, without external modules or network.
 * Pinned tradelayer.js@dca0888b, exact source expressions:
 * clearlist.js:10-30 (create under 'clearlists' with admin/backup),
 * clearlist.js:85-96 (getList reads 'attestations'),
 * clearlist.js:112-130 (updateAdmin writes 'clearlists'),
 * validity.js:945-964 (Type8 validator uses getList and expects
 * adminAddress/backupAddress), validity.js:1050-1061 (Type9 validator
 * instead reads getClearlistById and adminAddress || admin).
 */
const assert = require("node:assert/strict");

function createClearlist(sender, backup, existing) {
    const next = Math.max(0, ...[...existing.keys()]) + 1;
    const data = {id: next, admin: sender, backup, name: "ListOne"};
    existing.set(next, data);
    return data;
}
function getList(id, attestationCollection) {
    return attestationCollection.get(id) || null;
}
function getClearlistById(id, clearlistCollection) {
    return clearlistCollection.get(id) || null;
}
function validateType8Whitelist(sender, id, clearlistCollection, attestations) {
    // The actual validator calls ClearList.getList(id), whose implementation
    // reads from 'attestations' (NOT from 'clearlists').
    const info = getList(id, attestations);
    return Boolean(info) &&
        (info.adminAddress === sender || info.backupAddress === sender);
}
function updateProjectedAdmin(id, newAdmin, backup, registry) {
    const value = registry.get(id);
    if (!value) throw new Error("unknown list");
    if (backup) value.backup = newAdmin;
    else value.admin = newAdmin;
}
function validateType9Sender(sender, id, registry) {
    const info = getClearlistById(id, registry);
    return !!info && sender === (info.adminAddress || info.admin);
}
function parseRawType(raw) {
    assert.match(raw, /^tl[789]/);
    return {kind: parseInt(raw[2], 36), fields: raw.slice(3).split(",")};
}

function main() {
    const registry = new Map();
    const attestations = new Map();
    const created = createClearlist("adminA", "backupB", registry);
    assert.equal(created.id, 1);
    assert.equal(created.admin, "adminA");
    assert.equal(created.backup, "backupB");
    assert.equal(validateType9Sender("adminA", 1, registry), true);

    // An ordinary Type7-created list does not cause getList(id) to return
    // the list, and even querying the right collection would return the
    // field names 'admin' / 'backup', not the validator's names.
    assert.equal(validateType8Whitelist("adminA", 1, registry, attestations), false);
    const asIfFromRegistry = registry.get(1);
    assert.equal(asIfFromRegistry.adminAddress, undefined);
    assert.equal(asIfFromRegistry.backupAddress, undefined);

    const type7 = parseRawType("tl7backupB,ListOne,https://example.invalid,note");
    assert.equal(type7.kind, 7);
    assert.equal(type7.fields[0], "backupB");
    const type8 = parseRawType("tl8adminC,1,0,0,1,0");
    assert.equal(type8.kind, 8);
    assert.deepEqual(type8.fields, ["adminC", "1", "0", "0", "1", "0"]);
    const type9 = parseRawType("tl90,1,alice,OK");
    assert.equal(type9.kind, 9);
    assert.deepEqual(type9.fields, ["0", "1", "alice", "OK"]);

    // "If the type8 passed and the update were applied" is conditional
    // analysis. Its source-side acceptance was NOT verified above.
    updateProjectedAdmin(1, type8.fields[0], false, registry);
    assert.equal(validateType9Sender("adminC", 1, registry), true);
    assert.equal(validateType9Sender("adminA", 1, registry), false);
    assert.equal(validateType9Sender("backupB", 1, registry), false);
    console.log("PASS: Type7 allocates ID via DB max and stores admin/backup names");
    console.log("PASS: Type8 validator reads the wrong collection/field family for that list");
    console.log("PASS: Type9 checks current primary admin under a conditional Type8 update");
    console.log("BOUNDARY: source-read replay is not consensus-valid sender, admin, activation or revision");
}
main();
