#!/usr/bin/env node
"use strict";

/*
 * Independent source-expressions audit pinned to
 * tradelayer.js@dca0888b src/clearlist.js:9-574.
 * No bitcore, database, node RPC, wallet or private keys.
 *
 * Actual JavaScript Map keys: number 1 != string "1".
 * Actual direct attestations: _id=address, independent of listId.
 * Actual Type8 getList: queries attestations, not clearlists.
 * Actual getBanlist: findAsync(...) returns ARRAY, then reads array.data.
 */
const assert = require("node:assert/strict");
const crypto = require("node:crypto");

const sha = text => crypto.createHash("sha256").update(text).digest("hex");

function buildMerkleTree(xpubs) {
    if (!xpubs || xpubs.length === 0) return {root: null, tree: []};
    let leaves = xpubs.map(x => sha(x));
    if (leaves.length % 2 !== 0) leaves.push(leaves[leaves.length - 1]);
    const tree = [leaves.slice()];
    while (leaves.length > 1) {
        const level = [];
        for (let i = 0; i < leaves.length; i += 2) {
            const combined = leaves[i] + (leaves[i + 1] || leaves[i]);
            level.push(sha(combined)); // CRITICAL: ASCII HEX, not digest bytes.
        }
        tree.push(level);
        leaves = level;
    }
    return {root: leaves[0], tree};
}
function generateMerkleProof(xpubs, targetXpub) {
    const {tree} = buildMerkleTree(xpubs);
    if (!tree.length) return null;
    const leafHash = sha(targetXpub);
    let idx = tree[0].indexOf(leafHash);
    if (idx === -1) return null;
    const proof = [];
    for (let level = 0; level < tree.length - 1; level++) {
        const layer = tree[level];
        const isRight = idx % 2 === 1;
        const siblingIdx = isRight ? idx - 1 : idx + 1;
        if (siblingIdx < layer.length) proof.push({
            hash: layer[siblingIdx], position: isRight ? "left" : "right"
        });
        idx = Math.floor(idx / 2);
    }
    return proof;
}
function verifyMerkleProof(xpub, proof, expectedRoot) {
    let hash = sha(xpub);
    for (const sibling of proof) {
        const left = sibling.position === "right" ? hash : sibling.hash;
        const right = sibling.position === "right" ? sibling.hash : hash;
        hash = sha(left + right);
    }
    return hash === expectedRoot;
}

function createClearlistDb() {
    const lists = new Map(), cache = new Map(), att = new Map();
    return {
        lists, cache, att,
        create(admin, backup, id = null) {
            if (!id) {
                this.load();
                id = Math.max(0, ...[...cache.keys()].map(key => parseInt(key, 10))) + 1;
            }
            lists.set(id, {id, admin, backup});
            return id;
        },
        load() { for (const [id, data] of lists) cache.set(id, {...data}); },
        getClearlistById(id) { this.load(); return cache.get(id) || false; },
        verifyAdmin(id, actor) {
            const list = cache.get(id);
            if (!list) throw new Error("Clearlist not found");
            return (list.adminAddress || list.admin) === actor;
        },
        updateAdmin(id, address, backup) {
            const list = cache.get(id);
            if (!list) throw new Error("Clearlist not found");
            if (backup) {
                list.backupAddress = address;
                list.backup = address;
            } else {
                list.adminAddress = address;
                list.admin = address;
            }
            // Important: NeDB _id string in source updateAdmin, but ID
            // numeric in source createClearlist. Map keys don't coerce.
            lists.set(String(id), {...list});
            cache.set(id, list);
        },
        getList(id) { return att.get(id) || null; },
        attest(id, address, meta, timestamp) {
            const existing = att.get(address);
            att.set(address, {listId:id, address, status:"active", data:meta, timestamp});
            return existing;
        },
        revoke(id, address, reason, timestamp) {
            const matches = [...att.values()].filter(x =>
                x.address===address && x.status==="active" && x.listId===id);
            const row = matches[0] || att.get("attestation:"+address) || att.get(address);
            if (!row) throw new Error("Attestation not found");
            row.status = "revoked";
            row.revokeReason=reason;row.timestamp=timestamp;row.id=id;
        },
        isAddressInClearlist(id,address) {
            return [...att.values()].some(x =>
                x.address===address && x.status==="active" && x.listId===id);
        },
        country(address) {
            const row=[...att.values()].find(x=>x.address===address&&x.status==="active"&&x.listId===0);
            return row && typeof row.data==="string" && row.data.trim()
                ? {code:row.data.trim().toUpperCase(),blockHeight:row.timestamp||null} : null;
        },
        storedBanlist:null,
        cachedBanlist:["US","KP","SY","RU","IR","CU"],
        setBanlist(values) { this.storedBanlist=values;this.cachedBanlist=values; },
        getBanlist() {
            // Literal source bug: findAsync returns array, then .data
            const fromDb=this.storedBanlist ? [{_id:"globalBanlist",data:this.storedBanlist}] : [];
            return fromDb ? fromDb.data : null;
        }
    };
}

function main() {
    const store = createClearlistDb();
    assert.equal(store.create("adminA","backupB"),1);
    assert.equal(store.create("adminB","backupC"),2);
    assert.equal(store.getClearlistById(1).admin,"adminA");
    store.updateAdmin(1,"adminC",false);
    assert.equal(store.verifyAdmin(1,"adminC"),true);
    assert.equal(store.lists.get("1").admin,"adminC");
    assert.equal(store.lists.get(1).admin,"adminA");
    assert.equal(store.getClearlistById(1).admin,"adminA");
    assert.equal(store.verifyAdmin(1,"adminC"),false);
    assert.equal(store.getList(1),null);
    assert.equal(store.lists.size,3); // two numeric IDs plus string shadow

    store.attest(1,"alice","OK",10);
    assert.equal(store.isAddressInClearlist(1,"alice"),true);
    store.attest(2,"alice","OTHER",11);
    assert.equal(store.isAddressInClearlist(1,"alice"),false);
    assert.equal(store.isAddressInClearlist(2,"alice"),true);
    // No active row for requested list 1, so source fallback revokes
    // address alice, even though the persisted row now belongs to list 2.
    store.revoke(1,"alice","wrong list",12);
    assert.equal(store.att.get("alice").listId,2);
    assert.equal(store.att.get("alice").id,1);
    assert.equal(store.isAddressInClearlist(2,"alice"),false);
    store.attest(0,"alice"," us ",0);
    assert.deepEqual(store.country("alice"),{code:"US",blockHeight:null});
    store.attest(2,"alice","OK",1);
    assert.equal(store.country("alice"),null);

    assert.equal(store.getBanlist(),undefined);
    store.setBanlist(["GB","KP"]);
    assert.deepEqual(store.cachedBanlist,["GB","KP"]);
    assert.equal(store.getBanlist(),undefined);

    const single=buildMerkleTree(["xpub-synthetic-0"]);
    assert.equal(single.root,
        "82f8fcd70bcdf26565adb613a0dfe04955ee27054ab30f03e23160219aa73de9");
    assert(verifyMerkleProof("xpub-synthetic-0",
        generateMerkleProof(["xpub-synthetic-0"],"xpub-synthetic-0"),single.root));
    const five=Array.from({length:5},(_,i)=>"xpub-synthetic-"+i);
    const tree5=buildMerkleTree(five);
    assert.equal(tree5.root,
        "a9a5833ec88f79b211ba29e8d3bc936c02dba0ffaeaf2a48149304b679b63f3a");
    assert(verifyMerkleProof(five[0],generateMerkleProof(five,five[0]),tree5.root));
    const missing=generateMerkleProof(five,five[4]);
    assert.equal(missing.length,2);
    assert.equal(verifyMerkleProof(five[4],missing,tree5.root),false);

    console.log("PASS: numeric-vs-string admin upsert, stale load, wrong attestations getList");
    console.log("PASS: address-key overwrite and wrong-list revoke fallback");
    console.log("PASS: banlist array .data getter, list0 self-cert overwrite");
    console.log("PASS: source ASCII-hex Merkle and odd-parent skipped-sibling bug");
    console.log("BOUNDARY: no live source DB, xpub BIP32, historic signer or authority");
}
main();
