#!/usr/bin/env node
"use strict";
/*
 * Independent source-expression oracle pinned to
 * tradelayer.js@dca0888b src/channels.js:497-739,802-917,920-940.
 * No original JS dependencies, funds, TxUtils, wallets or network.
 * Every purported handleChannelPubkey result is an explicitly synthetic
 * positive boolean; this cannot prove the historical multisig address.
 */
const assert = require("node:assert/strict");

function col(addr, last = 1) {
    if (!addr || typeof addr !== "string" || addr.length < last) return "A";
    const odd = "ACEGIKMOQSUWY13579";
    return odd.includes(addr[addr.length - last].toUpperCase()) ? "A" : "B";
}
function tie(a, b, requested) {
    const n = Math.min(a.length, b.length);
    for (let i = 1; i <= n; ++i) {
        const ca = col(a, i), cb = col(b, i);
        if (ca === requested && cb !== requested) return [a, b, requested];
        if (cb === requested && ca !== requested) return [b, a, requested];
    }
    return [a, b, requested];
}
function newChannel(channel) {
    return {
        channel,
        participants: {A: "", B: ""},
        A: {}, B: {},
        commits: [],
        clearLists: {A: [], B: []},
        payEnabled: {A: false, B: false},
        lastCommitmentTime: null,
        lastUsedColumn: null,
        deltas: []
    };
}
function change(ch, col, next, height, reason) {
    const old = ch.participants[col] || "";
    if (old !== next) {
        ch.participants[col] = next;
        ch.deltas.push({column: col, old, next, height, reason});
    }
}
function bump(ch, forceAis, forceBis, height) {
    const before = {...ch.participants};
    const prevA = before.A, prevB = before.B;
    const hasA = forceAis !== undefined && forceAis !== null && forceAis !== "";
    const hasB = forceBis !== undefined && forceBis !== null && forceBis !== "";
    let nextA = hasA ? forceAis : (prevA || "");
    let nextB = hasB ? forceBis : (prevB || "");
    if (nextA && nextA === nextB) {
        if (prevA && prevA !== nextA) nextB = prevA;
        else if (prevB && prevB !== nextA) nextB = prevB;
    }
    if (prevA === nextA && prevB === nextB) return ch;
    if (prevA === nextB && prevB === nextA) {
        [ch.participants.A, ch.participants.B] = [ch.participants.B, ch.participants.A];
        [ch.A, ch.B] = [ch.B, ch.A];
    } else if (prevA === nextB || prevB === nextA) {
        [ch.participants.A, ch.participants.B] = [ch.participants.B, ch.participants.A];
        [ch.A, ch.B] = [ch.B, ch.A];
        if (ch.participants.A !== nextA || ch.participants.B !== nextB) {
            ch.participants.A = nextA || ch.participants.A;
            ch.participants.B = nextB || ch.participants.B;
        }
    } else {
        ch.participants.A = nextA;
        ch.participants.B = nextB;
    }
    for (const side of ["A", "B"]) {
        if (before[side] !== ch.participants[side]) {
            ch.deltas.push({column:side, old:before[side], next:ch.participants[side],
                height, reason:"Rotated (bumpColumnAssignment)"});
        }
    }
    return ch;
}
function assign(ch, addr, cp, height) {
    const column = col(addr);
    if (!ch.participants?.A && !ch.participants.B) {
        change(ch, column, addr, height, "Initial assignment");
        return ch;
    }
    if (ch.participants.A === addr || ch.participants.B === addr) return ch;
    if (ch.participants[column] !== cp && !ch.participants[column]) {
        change(ch, column, addr, height, "Assigned via open slot (no conflict)");
        return ch;
    }
    if (ch.participants[column] === cp ||
        (ch.participants[column] && ch.participants[column] !== addr)) {
        const [winner, loser, winnerColumn] = tie(addr, cp, column);
        const desiredA = winnerColumn === "A" ? winner : loser;
        const desiredB = winnerColumn === "B" ? winner : loser;
        const oldA = ch.participants.A || "", oldB = ch.participants.B || "";
        const forceA = desiredA || oldA;
        const forceB = desiredB || ((oldA && oldA !== forceA) ? oldA : oldB);
        return bump(ch, forceA, forceB, height);
    }
}
function resolve(ch, addr) {
    if (!addr) return null;
    if (ch.participants.A === addr) return "A";
    if (ch.participants.B === addr) return "B";
    return null;
}
function prediction(ch, addr, cp) {
    if (!ch || !ch.participants) return col(addr);
    if (!ch.participants.A && !ch.participants.B) return col(addr);
    if (ch.participants.A === addr) return "A";
    if (ch.participants.B === addr) return "B";
    const side = col(addr);
    if (ch.participants[side] !== cp && !ch.participants[side]) return side;
    if (ch.participants[side] === cp ||
        (ch.participants[side] && ch.participants[side] !== addr)) {
        return tie(addr, cp, side)[2];
    }
    return null;
}
function record(ch, sender, amount, opts={}) {
    const height = opts.height || 100;
    const lists = opts.lists || [];
    const pay = opts.pay === true;
    const pid = opts.pid || 7;
    const cp = ch.participants.A && ch.participants.A !== sender
        ? ch.participants.A
        : (ch.participants.B && ch.participants.B !== sender ? ch.participants.B : "");
    let side = prediction(ch, sender, cp);
    // The historical handleChannelPubkey performs FIRST-VIN/pubkey/multisig
    // checks here. These tests only exercise its hypothetical success path.
    assert.equal(opts.allegedPubkeyMismatch, undefined);
    assign(ch, sender, cp, height);
    side = resolve(ch, sender);
    if (!side) return null;
    ch[side][pid] = (ch[side][pid] || 0) + amount;
    if (Array.isArray(lists) && lists.length > 0) ch.clearLists[side] = lists;
    if (pay) ch.payEnabled[side] = true;
    ch.participants[side] = sender;
    ch.commits.push({
        senderAddress: sender, propertyId: pid, tokenAmount: amount,
        block: height, columnAssigned: side, payEnabled: pay,
        commitClearlistId: opts.commitClearlistId ?? null
    });
    ch.lastCommitmentTime = height;
    ch.lastUsedColumn = side;
    return side;
}
function sourceRows(ch, addr) {
    const lower = addr.trim().toLowerCase();
    const a = (ch.participants.A || "").toLowerCase();
    const b = (ch.participants.B || "").toLowerCase();
    const lastMine = [...ch.commits].reverse()
        .find(c => c.senderAddress.toLowerCase() === lower);
    const side = a === lower ? "A" : b === lower ? "B"
        : lastMine?.columnAssigned || null;
    if (!side) return [];
    return Object.entries(ch[side]).map(([id,amount])=>({
        propertyId:Number(id), amount, side,
        senderEverPay: ch.commits.some(c =>
            c.senderAddress.toLowerCase() === lower && c.payEnabled === true)
    })).filter(x=>x.amount > 0);
}

function main() {
    assert.equal(col("Alice1"), "A");
    assert.equal(col("Bob2"), "B");
    assert.equal(col(""), "A");
    assert.deepEqual(tie("Mike3", "Alice1", "A"), ["Mike3","Alice1","A"]);
    assert.deepEqual(tie("Eve3", "Alice1", "A"), ["Alice1","Eve3","A"]);

    const ch = newChannel("channel");
    assert.equal(record(ch,"Alice1",2,{height:100,lists:[7],pay:true}),"A");
    assert.equal(record(ch,"Bob2",3,{height:101,lists:[11]}),"B");
    assert.equal(ch.participants.size,undefined);
    assert.equal(ch.participants.size === 2,false); // source guard is dead
    assert.equal(ch.participants.A,"Alice1");
    assert.equal(ch.participants.B,"Bob2");
    assert.equal(ch.A[7],2);
    assert.equal(ch.B[7],3);
    assert.equal(ch.deltas.length,2);
    assert.equal(record(ch,"Alice1",2.5,{height:102,lists:[],pay:false}),"A");
    assert.equal(ch.A[7],4.5);
    assert.deepEqual(ch.clearLists.A,[7]);
    assert.equal(ch.payEnabled.A,true);
    assert.equal(ch.commits.length,3);
    assert.equal(ch.deltas.length,2);

    const overwritten = newChannel("crowded");
    record(overwritten,"Alice1",2,{lists:[7],pay:true});
    record(overwritten,"Bob2",3,{lists:[11]});
    assert.equal(record(overwritten,"Eve3",0.5),"B");
    assert.equal(overwritten.participants.B,"Eve3");
    assert.equal(overwritten.B[7],3.5);
    assert.deepEqual(overwritten.clearLists.B,[11]);
    assert.equal(sourceRows(overwritten,"Bob2")[0].amount,3.5);
    assert.equal(sourceRows(overwritten,"Eve3")[0].amount,3.5);

    const swapped = newChannel("swapped");
    record(swapped,"Alice1",1,{lists:[7],pay:true});
    record(swapped,"Bob2",5,{lists:[11]});
    assert.equal(record(swapped,"Mike3",2),"A");
    assert.equal(swapped.participants.A,"Mike3");
    assert.equal(swapped.participants.B,"Alice1");
    assert.equal(swapped.A[7],7);
    assert.equal(swapped.B[7],1);
    assert.deepEqual(swapped.clearLists.A,[7]);
    assert.deepEqual(swapped.clearLists.B,[11]);
    assert.equal(swapped.payEnabled.A,true);
    assert.equal(swapped.payEnabled.B,false);

    // Pinned savePendingWithdrawalToDB key includes height+sender but
    // omits channel and property; it permits last-write-wins collisions.
    const pending = new Map();
    const k = (h,s)=>"withdrawal-"+h+"-"+s;
    pending.set(k(400,"Alice1"),{channel:"channel-A",propertyId:7});
    pending.set(k(400,"Alice1"),{channel:"channel-B",propertyId:8});
    assert.equal(pending.size,1);
    assert.equal(pending.get(k(400,"Alice1")).channel,"channel-B");
    assert.equal(406 >= 400+7,false);
    assert.equal(407 >= 400+7,true);

    // Pinned processTransfer B->A and B->B branches update objects
    // inconsistently, not typed property subkeys. Never activate them.
    const sourceBtoA = "destinationChannel.A +=amount";
    const sourceBtoB = "destinationChannel.B +=amount";
    assert(sourceBtoA.includes("Channel.A +="));
    assert(sourceBtoB.includes("Channel.B +="));

    console.log("PASS: direct A/B assignment, duplicate commits, sticky policy");
    console.log("PASS: crowded overwritten participant and swap with balances but not policy");
    console.log("PASS: plain-object size guard false and withdrawal persistence key collision");
    console.log("BOUNDARY: no first-vin/2-of-2 signature or state revision/settlement authority");
}
main();
