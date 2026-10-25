#!/usr/bin/env node
"use strict";

/*
 * Historical Channel assignment *source-expression* audit only.
 * Pinned legacy source: tradelayer.js@dca0888b/src/channels.js
 *   742-749 processChannelCommits,
 *   809-821 plain participants object creation,
 *   844-846 direct assignColumnBasedOnAddress/resolveColumn path,
 *   899 A/B assignment,
 *   913-917 areBothPartiesCommitted,
 *   1169-1176 updateChannelWithColumnAssignments.
 *
 * This does NOT import/execute the legacy database, network, or channel
 * dispatcher. It faithfully reconstructs the small recorded expressions
 * and exhibits their distinct behavior under plain objects and Maps.
 * Do not silently "repair" historical consensus or issue authority.
 */
const assert = require("node:assert/strict");

// Exact pinned conditional from channels.js:913-917.
function pinnedAreBothPartiesCommitted(channelsRegistry, channelAddress) {
    const channel = channelsRegistry.get(channelAddress);
    if (!channel) return false;
    return channel.participants.size === 2;
}

function sourceCreatedChannel() {
    // channels.js:809-821 initial persisted representation.
    return {
        participants: {A: "", B: ""},
        channel: "synthetic-channel",
        commits: [],
        A: {},
        B: {},
        clearLists: {A: [], B: []},
        payEnabled: {A: false, B: false},
        lastCommitmentTime: 0,
        lastUsedColumn: null,
        channelPubkeys: {A: "", B: ""}
    };
}

// Exact pinned helper uses this.channels instead of channelsRegistry.
// Isolate its lookup only; the assignment mapper is unreachable whenever
// this.channels is undefined.
function pinnedUpdateLookup() {
    const channel = this.channels.get("synthetic-channel");
    return channel;
}

function main() {
    const registry = new Map();
    const record = sourceCreatedChannel();
    registry.set(record.channel, record);

    assert.equal(pinnedAreBothPartiesCommitted(registry, "missing"), false);
    assert.equal(record.participants.size, undefined);
    assert.equal(pinnedAreBothPartiesCommitted(registry, record.channel), false);
    // The source writes plain-object A/B properties at line 899, not Map entries.
    record.participants.A = "alice";
    record.participants.B = "bob";
    record.commits.push({senderAddress: "alice", columnAssigned: "A"});
    record.commits.push({senderAddress: "bob", columnAssigned: "B"});
    assert.equal(record.commits.length, 2);
    assert.equal(record.participants.A, "alice");
    assert.equal(record.participants.B, "bob");
    assert.equal(pinnedAreBothPartiesCommitted(registry, record.channel), false);

    // A Map is a different representation. It makes .size work but is NOT
    // what the pinned commit persistence routine initializes and updates.
    const mapControl = {participants: new Map([["A", "alice"], ["B", "bob"]])};
    registry.set("map-control", mapControl);
    assert.equal(pinnedAreBothPartiesCommitted(registry, "map-control"), true);

    // Replacing .size with Object.keys(...).length would create another
    // false positive because the plain source object initializes both keys
    // before either party commits. This cannot repair the predicate alone.
    const uncommitted = sourceCreatedChannel();
    assert.equal(Object.keys(uncommitted.participants).length, 2);
    assert.equal(Boolean(uncommitted.participants.A && uncommitted.participants.B), false);
    assert.equal(Boolean(record.participants.A && record.participants.B), true);

    // Pinned processChannelCommits enters assignment branch ONLY if the
    // above guard succeeds. The plain A/B record never triggers it.
    let assignmentCalls = 0;
    if (pinnedAreBothPartiesCommitted(registry, record.channel)) assignmentCalls += 1;
    assert.equal(assignmentCalls, 0);

    // The isolated pinned update helper uses this.channels, whereas all
    // persisted state above belongs to this.channelsRegistry.
    assert.equal(registry.get(record.channel), record);
    assert.throws(
        () => pinnedUpdateLookup.call({channelsRegistry: registry}),
        TypeError
    );

    // Source call-graph review also finds Channels.assignColumns at its
    // invocation line only, with no pinned class definition. This fact
    // requires source-version review before any parity/activation change.
    console.log("PASS: pinned A/B object .size guard denies even two populated parties");
    console.log("PASS: Map control and Object.keys control expose semantic mismatch");
    console.log("PASS: pinned helper's this.channels lookup mismatches channelsRegistry");
    console.log("BOUNDARY: direct recordCommitToChannel path is distinct and unverified");
    console.log("BOUNDARY: no claim of historic deployment, channel authorization or consensus fix");
}
main();
