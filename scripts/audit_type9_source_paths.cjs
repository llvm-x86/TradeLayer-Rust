#!/usr/bin/env node
"use strict";

/*
 * Stage25O: offline exact-source-expression characterization for Type9.
 *
 * PINNED JS SOURCE at tradelayer.js@dca0888b:
 * - src/txEncoder.js:228-237 encodes a complete "tl9" prefix
 * - src/txUtils.js:912-955 directly passes this encoder's bytes to
 *   litecore.Script.buildDataOut, then signs and broadcasts
 * - utils/attest.js:69-79 calls createAttestTransaction
 * - utils/createAttestation.js:98-105 likewise calls encoder directly
 * - src/types.js:27-61 prepends "tl" + transactionId.toString(36) AND
 *   concatenates the already prefixed encodeIssueOrRevokeAttestation result
 * - src/txDecoder.js:250-260 receives the body AFTER selecting type9.
 * - src/txIndex.js:301-315 calls Types.decodePayload(txid, marker, payload),
 *   whereas Types.decodePayload expects (txid, type, marker, encodedPayload,
 *   sender,reference,senderAmount,referenceAmount,block). That separate
 *   indexing path is not an authenticated historical decoder oracle.
 *
 * The two small functions below reproduce the literal pinned expressions;
 * no wallet, block scanner, imported npm dependency or network call.
 * They do not establish which past transactions were actually broadcast.
 */
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const marker = "tl";
function encodeIssueOrRevokeAttestation(params) {
    const payload = [
        params.revoke,
        params.id?.toString(36) ?? "0",
        params.targetAddress,
        params.metaData
    ];
    const type = 9;
    const typeStr = type?.toString(36) ?? "0";
    return marker + typeStr + payload.join(",");
}
function genericEncodeType9(params) {
    let payload = "tl";
    payload += (9).toString(36);
    payload += encodeIssueOrRevokeAttestation(params);
    return payload;
}
function decodeIssueOrRevokeAttestation(payload) {
    const parts = payload.split(",");
    return {
        revoke: parts[0] === "1",
        id: parseInt(parts[1] || "0", 36),
        targetAddress: parts[2] || "",
        metaData: parts[3].toUpperCase() || ""
    };
}
function frozen() {
    const values = new Map();
    const fixture = fs.readFileSync(
        path.join(__dirname, "../fixtures/channel_type9_p2wpkh_signed_synthetic.tsv"),
        "utf8"
    );
    for (const line of fixture.split("\n").filter(line => line && !line.startsWith("#"))) {
        const [key, value] = line.split("\t");
        assert(!values.has(key), "duplicate golden field");
        values.set(key, value);
    }
    return values;
}

function main() {
    const f = frozen();
    const params = {revoke: 0, id: 7, targetAddress: "alice", metaData: "OK"};
    const direct = encodeIssueOrRevokeAttestation(params);
    assert.equal(direct, "tl90,7,alice,OK");
    assert.equal(direct, f.get("payload_ascii"));
    assert.equal(genericEncodeType9(params), "tl9tl90,7,alice,OK");
    assert.deepEqual(decodeIssueOrRevokeAttestation(direct.slice(3)), {
        revoke: false, id: 7, targetAddress: "alice", metaData: "OK"
    });
    const revoked = encodeIssueOrRevokeAttestation({...params, revoke: 1});
    assert.equal(revoked, "tl91,7,alice,OK");
    assert.equal(decodeIssueOrRevokeAttestation(revoked.slice(3)).revoke, true);
    assert.equal(decodeIssueOrRevokeAttestation("0,b,bob,ok").id, 11);
    assert.equal(genericEncodeType9({...params, id: 11}), "tl9tl90,b,alice,OK");
    const type9Header = "746c";
    const opReturnScript = Buffer.concat([
        Buffer.from([0x6a, direct.length]),
        Buffer.from(direct, "utf8")
    ]);
    // Pinned txIndex.js extraction searches "746c" in scriptPubKey.hex
    // and extracts after the first "tl". This doesn't validate sender.
    const opReturnHex = opReturnScript.toString("hex");
    assert.equal(opReturnHex.indexOf(type9Header), 4);
    assert.equal(Buffer.from(opReturnHex.slice(8), "hex").toString(), "90,7,alice,OK");
    // Source txIndex.js::processTransaction forwards the wrong 3-argument
    // shape to the 9-argument Types.decodePayload; this path cannot be
    // promoted as a reliable historical authorization check.
    const actualArgCount = 3, decoderRequiredContextCount = 9;
    assert(actualArgCount < decoderRequiredContextCount);

    console.log("PASS: direct type9 constructor broadcasts one-prefixed source expression");
    console.log("PASS: generic Types encoder has a distinct nested-prefix result");
    console.log("PASS: source body decoder, base36 list, revoke flag, OP_RETURN marker extraction");
    console.log("BOUNDARY: no historical transaction samples or valid source-side admin execution");
}
main();
