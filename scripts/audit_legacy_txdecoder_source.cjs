#!/usr/bin/env node
"use strict";
/*
 * File 6/49: execute BYTE-EXACT pinned txDecoder.js in a Node VM.
 * All imports are mocks: no wallets, chain, bitcore, actual BIP32, DB,
 * BigNumber package, Base94 runtime or external network. Source VM is
 * observational only; a decoded object is NEVER an accepted transaction.
 */
const assert=require("node:assert/strict");
const fs=require("node:fs");
const path=require("node:path");
const crypto=require("node:crypto");
const vm=require("node:vm");
const ROOT=path.resolve(__dirname,"..");
const filename=path.join(ROOT,"fixtures/legacy_types_source_txDecoder.txt");
const bytes=fs.readFileSync(filename);
const sourceSha=crypto.createHash("sha1").update(Buffer.concat([
 Buffer.from("blob "+bytes.length+"\0"),bytes
])).digest("hex");
assert.equal(sourceSha,"4728f1bdce1190428f91ae5b02f380584073c431");

class DummyBN {
 constructor(value,base) {
  this.value=base===36?parseInt(value,36):Number(value);
 }
 div(v){return new DummyBN(this.value/Number(v))}
 dividedBy(v){return this.div(v)}
 decimalPlaces(){return this}
 toNumber(){return this.value}
}
const mockedBase94={fromBase94(s){return String(s)}};
const forbiddenBase256={base256ToHex(){throw Error("SHADOWED FIRST SETTLEMENT DECODER CALLED")}};
const module={exports:{}};
vm.runInNewContext(bytes.toString("utf8"),{
 module,exports:module.exports,console:{log(){},error(){}},
 require(key){
  if(key==="bignumber.js")return DummyBN;
  if(key==="./base94.js")return mockedBase94;
  if(key==="./base256.js")return forbiddenBase256;
  throw Error("unapproved module import "+key);
 }
},{timeout:1000,filename:"pinned-txDecoder.js"});
const decoder=module.exports;
const methods={
 4:"decodeCommitToken",
 7:"decodeCreateWhitelist",
 8:"decodeUpdateAdmin",
 9:"decodeIssueOrRevokeAttestation",
 15:"decodeCloseOracle",
 21:"decodeWithdrawal",
 22:"decodeTransfer",
 30:"decodeStakeFraudProof",
 32:"decodeBatchMoveZkRollup",
 33:"decodeColoredCoin"
};
const lines=fs.readFileSync(path.join(ROOT,"fixtures/legacy_txdecoder_body_vectors.tsv"),"utf8")
 .split("\n").filter(s=>s&&!s.startsWith("#")).map(s=>{
  const [rawId,body,tag]=s.split("\t");
  assert(tag&&body!==undefined);
  return {id:Number(rawId),body,tag};
});
assert.equal(lines.length,17);
function norm(value){return JSON.parse(JSON.stringify(value))}
let accepted=0,malformed=0;
for(const row of lines){
 const fn=decoder[methods[row.id]];
 assert.equal(typeof fn,"function");
 if(row.tag.endsWith("-throws")){
  assert.throws(()=>fn(row.body),{name:"TypeError"},row.tag);
  malformed++;
  continue;
 }
 const p=fn(row.body);
 switch(row.tag){
  case "commit-singleton-list":
   assert.equal(p.propertyId,7);assert.equal(p.amount,10/1e8);
   assert.equal(p.channelAddress,"channel1");assert.equal(p.ref,false);
   assert.equal(p.payEnabled,true);
   assert.deepEqual(Array.from(p.clearLists),[7]);
   assert.equal(p.isColoredOutput,false);assert.equal(p.commitClearlistId,11);
   break;
  case "commit-synthetic-raw-ref":
   assert.equal(p.propertyId,"s-1-10");
   assert.equal(p.amount,2/1e8);assert.equal(p.ref,"04");
   assert.equal(p.channelAddress,"");assert.deepEqual(Array.from(p.clearLists),[11]);
   assert.equal(p.isColoredOutput,true);assert.equal(p.commitClearlistId,2);
   break;
  case "commit-comma-bracket-misparse":
   assert.equal(Number.isNaN(p.clearLists[0]),true);
   assert.equal(p.commitClearlistId,0);
   assert.equal(p.isColoredOutput,false);
   malformed++;break;
  case "whitelist-four-fields":
   assert.deepEqual(norm(p),{
    backupAddress:"backupB",name:"ListOne",url:"url",description:"desc"
   });break;
  case "admin-base36-and-backup":
   assert.deepEqual(norm(p),{
    newAddress:"adminC",whitelist:true,oracle:false,
    token:false,id:11,updateBackup:true
   });break;
  case "attestation-revoke-uppercase":
   assert.deepEqual(norm(p),{
    revoke:true,id:11,targetAddress:"alice",metaData:"OK"
   });break;
  case "attestation-post-marker-selector-is-not-revoke":
   assert.equal(p.revoke,false);assert.equal(p.id,7);
   assert.equal(p.metaData,"OK");break;
  case "close-oracle-primitive":
   assert.equal(p,15);assert.equal(typeof p,"number");break;
  case "withdrawal-decimal-ref":
   assert.equal(p.withdrawAll,true);assert.equal(p.propertyId,7);
   assert.equal(p.amount,10/1e8);assert.equal(p.column,true);
   assert.equal(p.ref,10);assert.equal(p.channelAddress,"");break;
  case "withdrawal-bad-ref-false":
   assert.equal(p.withdrawAll,false);assert.equal(p.propertyId,"s-1-10");
   assert.equal(p.amount,2/1e8);assert.equal(p.ref,false);break;
  case "transfer-text-ref":
   assert.equal(p.propertyId,7);assert.equal(p.amount,10/1e8);
   assert.equal(p.isColumnA,true);assert.equal(p.toChannelAddress,"");
   assert.equal(p.ref,"0");assert.equal(p.pay,true);assert.equal(p.payRef,"payRef");
   break;
  case "stake-fraud-all-fields":
   assert.equal(p.action,1);assert.equal(p.oracleId,11);
   assert.equal(p.stakedPropertyId,7);assert.equal(p.amount,10/1e8);
   assert.equal(p.autoRoll,true);assert.equal(p.relayType,2);
   assert.equal(p.dlcRef,"ref");assert.equal(p.nextDlcRef,"next");
   break;
  case "zk-batch-opaque-not-validated":
   assert.equal(p.ordinalRevealJSON,row.body);assert.equal(p.valid,undefined);
   break;
  case "colored-decimal-action-base36-ratio":
   assert.equal(p.encodeDecodeRecode,2);assert.equal(p.propertyId,10);
   assert.equal(p.satsRatio,36);assert.equal(p.homeAddress,"home");
   assert.equal(p.valid,undefined);break;
  default:throw Error("unhandled vector "+row.tag);
 }
 accepted++;
}
assert.equal(accepted,14);
assert.equal(malformed,3);

// Exactly two declarations of the same object property; JS uses the LAST.
// base256 mock throws if the original first method were invoked.
const declarations=[...bytes.toString("utf8").matchAll(/^\s*decodeSettleChannelPNL\s*:/gm)];
assert.equal(declarations.length,2);
const settle=decoder.decodeSettleChannelPNL("one,two,12.5,2,1,0,99.25,10");
assert.equal(settle.markPrice,12.5);
assert.equal(settle.settleType,2);
assert.equal(settle.netAmount,99.25);
assert.equal(settle.expiryBlock,10);
assert.equal(settle.close,undefined);

// The exported bridge helper is a different name from Types Type34 route.
assert.equal(typeof decoder.decodeAbstractionBridge,"function");
assert.equal(typeof decoder.decodeCrossLayerBridge,"undefined");
const helper=decoder.decodeAbstractionBridge("a,b,ignored-destination");
assert.equal(helper.propertyId,10);assert.equal(helper.amount,11);
assert.equal(Object.hasOwn(helper,"destinationAddr"),false);

// The send scalar branch intercepts four semicolon elements FIRST, even if
// elements themselves contain comma-delimited multi-property fields.
const malformedMulti=decoder.decodeSend("0;;5,7;a,b");
assert.equal(malformedMulti.propertyIds,5);
assert.equal(malformedMulti.amounts,10);
assert.equal(malformedMulti.isColoredOutput,false);
assert.equal(decoder.decodeAmount("a~"),10/1e8);
assert.equal(decoder.decodeAmount("a"),10);
assert.equal(decoder.decodePropertyId("s-1-a"),"s-1-10");
assert.equal(decoder.decodePropertyId("s1-a"),"s-1-10");

// No imported source method constitutes historical signer/UTXO permission.
console.log("PASS: pinned txDecoder blob and 17 shared literal body vectors");
console.log("PASS: comma-bracket NaN, missing fields throwing, Type4/21/22 distinct refs");
console.log("PASS: Type23 later base94 decoder wins, source send multi-field shadow");
console.log("PASS: Type34 standalone bridge helper exists, but dispatch name is absent");
console.log("BOUNDARY: mock BigNumber/Base94 and no chain, economic or signer authority");
