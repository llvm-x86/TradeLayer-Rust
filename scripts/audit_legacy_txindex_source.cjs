#!/usr/bin/env node
"use strict";
/*
 * Run ORIGINAL source txIndex.js@dca0888b (Git-blob pinned) in an isolated
 * mock-only Node VM to test why sequential JS per-TX ingestion cannot
 * drive the Rust consensus pipeline. Absolutely no real node, wallet,
 * private keys or database access.
 */
const assert=require("node:assert/strict");
const fs=require("node:fs");
const path=require("node:path");
const crypto=require("node:crypto");
const vm=require("node:vm");
const source=fs.readFileSync(path.join(__dirname,
 "../fixtures/legacy_txindex_source_txIndex.txt"));
const sha=crypto.createHash("sha1").update(Buffer.concat([
 Buffer.from("blob "+source.length+"\0"),source
])).digest("hex");
assert.equal(sha,"3d7bdafbdffe5af7d3250c4dcc80b8358c485597");

let calls=[];
const utils={
 async getSender(txid){return {senderAddress:"alice",amount:1,txid}},
 async getTransactionOutputs(){return [
  {vout:2,address:"change",satoshis:1000},
  {vout:1,address:"other",satoshis:600},
  {vout:0,address:"first",satoshis:400}
 ]}
};
const types={
 decodePayload(...args){
  calls.push(args);
  return {invocationArity:args.length,markerInTypeArg:args[1],payloadInMarkerArg:args[2]};
 }
};
const persist=[];
const db={async getDatabase(){
 return {async updateAsync(q,data) {persist.push({key:q._id,value:data.value});}}
}};
const fakeModule={exports:{}};
vm.runInNewContext(source.toString("utf8"),{
 module:fakeModule,exports:fakeModule.exports,
 console:{log(){},warn(){},error(){}},Buffer,process:{env:{}},
 require(name){
  if(name==="big-json")return {};
  if(name==="util")return {};
  if(name==="./txUtils")return utils;
  if(name==="./db.js")return db;
  if(name==="./client.js")return {};
  if(name==="./types.js")return types;
  throw Error("unexpected import: "+name);
 }
},{timeout:1000,filename:"pinned-txIndex.js"});
const Index=fakeModule.exports;
const hex=data=>Buffer.from(data).toString("hex");
const script=body=>"6a"+(2+body.length).toString(16).padStart(2,"0")+"746c"+hex(body);
const out=(index,hexCode,nulldata=true)=>({
 n:index,scriptPubKey:{hex:hexCode,type:nulldata?"nulldata":"witness_v0_keyhash"}
});
async function check(){
 const good=script("20;alice;5;2");
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"",false),out(1,good)]};
 }};
 const result=await Index.DecodeRawTransaction("fake");
 assert.equal(result.marker,"tl");
 assert.equal(result.payload,"20;alice;5;2");
 assert.equal(result.decodedTx.vout.length,2);
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"6a0568656c6c6f"),out(1,good)]};
 }};
 assert.equal(await Index.DecodeRawTransaction("fake"),null);
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"6a746c32303b616c696365")]};
 }};
 const strange=await Index.DecodeRawTransaction("fake");
 assert.equal(strange.payload,"20;alice");
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"6a02746c3230f")]};
 }};
 assert.equal((await Index.DecodeRawTransaction("fake")).payload,"20");
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"6a02746c00")]};
 }};
 assert.equal(await Index.DecodeRawTransaction("fake"),null);
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"6a02746c")]};
 }};
 assert.equal((await Index.DecodeRawTransaction("fake")).payload,"");
 Index.client={async decoderawtransaction(){
  return {vout:[out(0,"746C20")]};
 }};
 assert.equal(await Index.DecodeRawTransaction("fake"),null);

 const one=await Index.processTransaction("90,7,alice,OK","fakeTx","tl");
 assert.equal(one.sender.senderAddress,"alice");
 assert.deepEqual(Array.from(one.reference, r=>[r.address,r.satoshis]),
   [["other",600],["first",400]]);
 assert.equal(Object.hasOwn(one.reference[0],"vout"),false);
 assert.equal(calls.length,1);
 assert.equal(calls[0].length,3);
 assert.deepEqual(calls[0],["fakeTx","tl","90,7,alice,OK"]);
 assert.equal(one.decodedParams.markerInTypeArg,"tl");

 // The source's block loop fetches and processes each TX sequentially.
 // Even slow "RPC" futures never have more than one in flight here.
 let active=0,peak=0;
 const origFetch=Index.fetchTransactionData;
 const origDecode=Index.DecodeRawTransaction;
 const origProcess=Index.processTransaction;
 const origSave=Index.saveMaxHeight;
 Index.fetchTransactionData=async txid=>{
  active++;peak=Math.max(peak,active);
  await Promise.resolve();
  active--;return txid;
 };
 Index.DecodeRawTransaction=async raw=>({marker:"tl",payload:raw});
 Index.processTransaction=async payload=>({payload});
 Index.saveMaxHeight=async()=>{};
 const rows=await Index.processBlockData({
  hash:"claimed-hash",tx:["A","B","C","D"]
 },101);
 assert.deepEqual(Array.from(rows,r=>r.payload),["A","B","C","D"]);
 assert.equal(peak,1);
 assert.equal(persist.length,4);
 assert.deepEqual(persist.map(p=>p.key),
  ["tx-101-A","tx-101-B","tx-101-C","tx-101-D"]);
 Index.fetchTransactionData=origFetch;
 Index.DecodeRawTransaction=origDecode;
 Index.processTransaction=origProcess;
 Index.saveMaxHeight=origSave;
 console.log("PASS: byte-pinned 506-line JS source, marker offset and Buffer.from odd-hex behavior");
 console.log("PASS: first nulldata takes precedence, ASCII guard, three-vs-nine arg decode call");
 console.log("PASS: vout 0/1 mapping loses vout IDs, source per-block processing peak in-flight=1");
 console.log("BOUNDARY: mock-only source callbacks, no real network or canonical admission");
}
check().catch(e=>{console.error("FAIL:",e.stack||String(e));process.exitCode=1});
