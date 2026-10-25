#!/usr/bin/env node
"use strict";

/*
 * File 7/49: run ACTUAL pinned txEncoder.js and Types.encodePayload,
 * checked byte-for-byte against archived Git blobs. Only mock imports are
 * provided, with exact safe numeric inputs: no wallet, network, chain,
 * native BigNumber package, validator, signing key or live DB.
 *
 * The output strings are source expressions, NOT send-ready transactions.
 */
const assert=require("node:assert/strict");
const fs=require("node:fs");
const crypto=require("node:crypto");
const path=require("node:path");
const vm=require("node:vm");
const ROOT=path.resolve(__dirname,"..");
function source(relative,sha) {
 const raw=fs.readFileSync(path.join(ROOT,relative));
 const header=Buffer.from("blob "+raw.length+"\0");
 const computed=crypto.createHash("sha1").update(Buffer.concat([header,raw])).digest("hex");
 assert.equal(computed,sha);
 return raw.toString("utf8");
}
const encoderSrc=source("fixtures/legacy_types_source_txEncoder.txt",
 "94201b4eaedcd240b9a9f6d54d11b55206615dff");
const decoderSrc=source("fixtures/legacy_types_source_txDecoder.txt",
 "4728f1bdce1190428f91ae5b02f380584073c431");
const typesSrc=source("fixtures/legacy_types_source_types.txt",
 "fb894e8a25d560cc9481f9913aedf003fdde382f");

class NumberStub {
 constructor(v,radix) {this.num=radix===36?parseInt(v,36):Number(v)}
 times(v) {return new NumberStub(this.num*Number(v))}
 div(v) {return new NumberStub(this.num/Number(v))}
 dividedBy(v) {return this.div(v)}
 integerValue() {return new NumberStub(Math.trunc(this.num))}
 decimalPlaces() {return this}
 toString(radix) {return this.num.toString(radix||10)}
 toNumber() {return this.num}
}
NumberStub.ROUND_DOWN=1;
const silence={log(){},warn(){},error(){}};
function load(content, imports) {
 const fakeModule={exports:{}};
 vm.runInNewContext(content,{
  module:fakeModule,exports:fakeModule.exports,console:silence,
  Buffer,BigInt,Number,
  require(name) {
   if (!Object.prototype.hasOwnProperty.call(imports,name))
    throw new Error("disallowed require "+name);
   return imports[name];
  }
 },{timeout:1000});
 return fakeModule.exports;
}
const encoder=load(encoderSrc,{
 "bignumber.js":NumberStub,"./base94.js":{},"./base256.js":{}
});
const decoder=load(decoderSrc,{
 "bignumber.js":NumberStub,"./base94.js":{},"./base256.js":{}
});
const types=load(typesSrc,{
 "./txEncoder":encoder,"./txDecoder":decoder,"./validity":{},
 "./txUtils":{},"./txIndex.js":{},"bignumber.js":NumberStub
});
const long="C".repeat(43);
const zeroHash="0".repeat(62)+"ff";
const baseCommit={
 propertyId:7,amount:1,channelAddress:"channel1",ref:4,
 payEnabled:true,clearLists:[7],isColoredOutput:false,
 commitClearlistId:11
};
const cases=[
 ["activation-256",0,"encodeActivateTradeLayer",{
  txTypeToActivate:[0,9],codeHash:zeroHash
 }],
 ["token-issue-multi",1,"encodeTokenIssue",{
  initialAmount:10,ticker:"TK",whitelists:[7,11],
  managed:true,backupAddress:"backupB",nft:false,coloredCoinHybrid:false
 }],
 ["send-all",2,"encodeSend",{sendAll:true,address:"alice"}],
 ["send-whole",2,"encodeSend",{
  sendAll:false,address:"alice",propertyId:5,amount:2
 }],
 ["send-fraction",2,"encodeSend",{
  sendAll:false,address:"alice",propertyId:5,amount:0.5
 }],
 ["send-multi",2,"encodeSend",{
  sendAll:false,propertyId:[5,7],amount:[1,0.5]
 }],
 ["commit-single",4,"encodeCommit",baseCommit],
 ["commit-multi",4,"encodeCommit",{
  ...baseCommit,payEnabled:false,clearLists:[7,11],isColoredOutput:true,
  commitClearlistId:undefined
 }],
 ["commit-long-ref",4,"encodeCommit",{
  ...baseCommit,channelAddress:long,payEnabled:false,clearLists:11,
  isColoredOutput:true,commitClearlistId:undefined
 }],
 ["whitelist",7,"encodeCreateWhitelist",{
  backupAddress:"backupB",name:"ListOne",url:"site",description:"note"
 }],
 ["admin",8,"encodeUpdateAdmin",{
  newAddress:"adminC",whitelist:true,oracle:false,token:false,
  id:11,updateBackup:true
 }],
 ["attest-numeric",9,"encodeIssueOrRevokeAttestation",{
  revoke:0,id:7,targetAddress:"alice",metaData:"ok"
 }],
 ["attest-boolean-revoke",9,"encodeIssueOrRevokeAttestation",{
  revoke:true,id:7,targetAddress:"alice",metaData:"ok"
 }],
 ["close-ignores-id",15,"encodeCloseOracle",11],
 ["withdraw-boolean",21,"encodeWithdrawal",{
  withdrawAll:true,propertyId:7,amountOffered:1,column:1,
  channelAddress:"channel1",ref:10
 }],
 ["withdraw-long-ref",21,"encodeWithdrawal",{
  withdrawAll:false,propertyId:7,amountOffered:0.5,column:0,
  channelAddress:long,ref:10
 }],
 ["transfer-direct",22,"encodeTransfer",{
  propertyId:7,amount:1,isColumnA:true,destinationAddr:"chanB",
  ref:3,pay:true,payRef:"pay"
 }],
 ["transfer-long-ref",22,"encodeTransfer",{
  propertyId:7,amount:0.5,isColumnA:false,destinationAddr:long,
  ref:3,pay:false,payRef:""
 }],
 ["stake-fraud-relay",30,"encodeStakeFraudProof",{
  action:1,oracleId:11,stakedPropertyId:7,amount:1,
  accusedAddress:"alice",evidenceHash:"evidence",relayType:2,
  stateHash:"state",dlcRef:"dlc",settlementState:"settle",
  relayBlob:"blob",autoRoll:true,nextDlcRef:"next"
 }],
 ["colored-body",33,"encodeColoredCoin",{
  encodeDecodeRecode:2,propertyId:7,SatsRatio:10,homeAddress:"home"
 }],
 ["bridge-body",34,"encodeAbstractionBridge",{
  propertyId:7,amount:11,destinationAddr:"destination"
 }],
 ["bind-body",35,"encodeBindSmartContract",{
  contractHash:"hash",type:2,hostChain:"btc"
 }]
];
const golden=fs.readFileSync(
 path.join(ROOT,"fixtures/legacy_txencoder_source_vectors.tsv"),"utf8")
 .split("\n").filter(s=>s&&!s.startsWith("#")&&!s.startsWith("tag\t"))
 .map(s=>s.split("\t"));
assert.equal(cases.length,22);assert.equal(golden.length,22);
for(let i=0;i<cases.length;i++) {
 const [tag,type,method,params]=cases[i];
 assert.equal(golden[i][0],tag,"fixture ordering");
 const actual=encoder[method](params);
 assert.equal(actual,golden[i][1],tag+" source inner mismatch");
 const generic=golden[i][2];
 if(type===4) {
  assert.equal(generic,"!missing");
  assert.equal(typeof encoder.encodeCommitToken,"undefined");
  assert.throws(()=>types.encodePayload(4,params),{name:"TypeError"});
 }else{
  assert.equal(types.encodePayload(type,params),generic,tag+" generic mismatch");
 }
}
// Fractional encodeAmount is missing the "~" due to JS ?? precedence,
// so its standalone decodeAmount interprets that raw base36 as units.
assert.equal(encoder.encodeAmount(0.5),"tro8w");
assert.equal(decoder.decodeAmount("tro8w"),50000000);
assert.equal(decoder.decodeAmount("tro8w~"),0.5);
const whole=decoder.decodeSend("0;alice;5;2");
const fraction=decoder.decodeSend("0;alice;5;tro8w");
assert.equal(whole.amounts,2);
assert.equal(fraction.amounts,50000000);
const multi=decoder.decodeSend("0;;5,7;1,tro8w");
assert.equal(multi.propertyIds,5);
assert.equal(multi.amounts,1);

// Source decimal-vs-base36 withdrawal amount is a concrete economic hazard.
const withdrawal=encoder.encodeWithdrawal({
 withdrawAll:true,propertyId:7,amountOffered:1,column:1,
 channelAddress:"channel1"
});
const parsed=decoder.decodeWithdrawal(withdrawal.slice(3));
assert.equal(parsed.withdrawAll,false); // JS true becomes "true", != '1'
assert.equal(parsed.amount,parseInt("100000000",36)/1e8);
assert.notEqual(parsed.amount,1);
const odd=decoder.decodeIssueOrRevokeAttestation(
 encoder.encodeIssueOrRevokeAttestation({
  revoke:true,id:7,targetAddress:"alice",metaData:"ok"
 }).slice(3)
);
assert.equal(odd.revoke,false); // "true" !== "1"
const numeric=decoder.decodeIssueOrRevokeAttestation("1,7,alice,ok");
assert.equal(numeric.revoke,true);

// Type1 whitelist encoding uses commas; decoder reads only parts[2] and
// assumes subsequent fields are managed, backupAddress, nft etc.
const issued=encoder.encodeTokenIssue({
 initialAmount:10,ticker:"TK",whitelists:[7,11],managed:true,
 backupAddress:"backupB",nft:false,coloredCoinHybrid:false
});
const interpreted=decoder.decodeTokenIssue(issued.slice(3));
assert.deepEqual(Array.from(interpreted.whitelists),[7]);
assert.equal(interpreted.managed,false);
assert.equal(interpreted.backupAddress,"1");

// Original Type4 brackets explode into invalid metadata when list >1.
const committed=encoder.encodeCommit({
 ...baseCommit,payEnabled:false,clearLists:[7,11],isColoredOutput:true,
 commitClearlistId:undefined
});
const bad=decoder.decodeCommitToken(committed.slice(3));
assert.equal(Number.isNaN(bad.clearLists[0]),true);

// Type15 oracle identifier ignored even when directly supplied.
assert.equal(encoder.encodeCloseOracle(7),encoder.encodeCloseOracle(100));

// Two identifiers reused as "property amount" without 1e8 in bridge.
assert.equal(encoder.encodeAbstractionBridge({
 propertyId:7,amount:11,destinationAddr:"destination"
 }),"7,b,destination");

console.log("PASS: Git-blob-pinned txEncoder and Types code; 22 direct/generic golden strings");
console.log("PASS: 256-bit activation code hash, whole/fractional and multi-send encodings");
console.log("PASS: Type4 missing generic export, list-comma and Type9 boolean revoke mismatch");
console.log("PASS: Type21 decimal-vs-base36 scaled amount and Boolean flag incompatibility");
console.log("PASS: Type1 whitelist shift, Type15 ignored oracle ID, body-only 33/34/35");
console.log("BOUNDARY: source strings only; no wallet, chain, signatures or settlement permissions");
