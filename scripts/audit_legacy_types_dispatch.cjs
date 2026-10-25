#!/usr/bin/env node
"use strict";
// File 5/49: run PINNED source types.js with NO real external dependencies.
// Byte-identical archives and mock-only VM imports: no chain, RPC or keys.
const assert=require("node:assert/strict");
const crypto=require("node:crypto");
const fs=require("node:fs");
const path=require("node:path");
const vm=require("node:vm");
const root=path.resolve(__dirname,"..");
const archives=[
 ["fixtures/legacy_types_source_types.txt","fb894e8a25d560cc9481f9913aedf003fdde382f"],
 ["fixtures/legacy_types_source_txEncoder.txt","94201b4eaedcd240b9a9f6d54d11b55206615dff"],
 ["fixtures/legacy_types_source_txDecoder.txt","4728f1bdce1190428f91ae5b02f380584073c431"]
];
const sources=archives.map(([name,sha])=>{
 const buffer=fs.readFileSync(path.join(root,name));
 const blob=Buffer.concat([Buffer.from("blob "+buffer.length+"\0"),buffer]);
 assert.equal(crypto.createHash("sha1").update(blob).digest("hex"),sha);
 return buffer.toString("utf8");
});
class NumberStub{
 constructor(value,base){this.n=base===36?parseInt(value,36):Number(value)}
 times(v){return new NumberStub(this.n*Number(v))}
 integerValue(){return this}
 decimalPlaces(){return this}
 toString(radix){return this.n.toString(radix||10)}
 toNumber(){return this.n}
 div(v){return new NumberStub(this.n/Number(v))}
}
const silent={log(){},warn(){},error(){}};
function load(file, imports){
 const module={exports:{}};
 const sandbox={
  console:silent,Buffer,BigInt,module,exports:module.exports,
  require(name){
   if(!Object.prototype.hasOwnProperty.call(imports,name))throw Error("forbidden require "+name);
   return imports[name];
  }
 };
 vm.runInNewContext(file,sandbox,{timeout:1000});
 return module.exports;
}
const encoder=load(sources[1],{"bignumber.js":NumberStub,"./base94.js":{},"./base256.js":{}});
const decoder=load(sources[2],{"bignumber.js":NumberStub,"./base94.js":{},"./base256.js":{}});
const calls=[];
const validity=new Proxy({},{
 get(_o,key){
  return async(sender,params)=>{
   calls.push({method:key,sender,type:typeof params,block:params?.block});
   return params && typeof params==="object"?{...params,valid:true}:params;
  };
 }
});
const routedDecoder={...decoder,decodeTradeTokenForUTXO:()=>({
 payToAddress:0,tokenOutput:1,tagWithdraw:null
})};
const types=load(sources[0],{
 "./txEncoder":encoder,"./txDecoder":routedDecoder,"./validity":validity,
 "./txUtils":{},"./txIndex.js":{},"bignumber.js":NumberStub
});
const tsv=fs.readFileSync(path.join(root,"fixtures/legacy_types_routes.tsv"),"utf8");
const actual=tsv.split("\n").filter(l=>l&&!l.startsWith("#")&&!l.startsWith("id\t"))
.map(line=>{
 const [id,encode,ePresent,decode,dPresent,validator,called]=line.split("\t");
 return {id:Number(id),encode,ePresent:ePresent==="1",decode,
  dPresent:dPresent==="1",validator,called:called==="1"};
});
const method=(source,name)=>new RegExp("(?:^|\\n)\\s*"+name+
 "\\s*(?:\\([^)]*\\)\\s*\\{|:\\s*\\([^)]*\\)\\s*=>)","m").test(source);
function checkRoutes(){
 const encSection=sources[0].slice(0,sources[0].indexOf("decodePayload:"));
 const encode=[...encSection.matchAll(/case (\d+):\s*payload \+= Encode\.(encode\w+)\(/g)];
 const decode=[...sources[0].slice(sources[0].indexOf("decodePayload:"))
  .matchAll(/case (\d+):([\s\S]*?)(?=\n\s*case \d+:|\n\s*default:)/g)];
 assert.equal(encode.length,36);
 assert.equal(decode.length,36);
 const observed=decode.map((entry,i)=>{
  const id=Number(entry[1]);
  assert.equal(id,i);
  assert.equal(Number(encode[i][1]),id);
  const enc=encode[i][2];
  const dec=entry[2].match(/Decode\.(decode\w+)\(/)?.[1]||"?";
  const valid=entry[2].match(/^\s*params\s*=\s*await Validity\.(validate\w+)\(/m)?.[1]||"-";
  return {id,encode:enc,ePresent:method(sources[1],enc),
   decode:dec,dPresent:method(sources[2],dec),validator:valid,called:valid!=="-"};
 });
 assert.deepEqual(observed,actual);
}
async function test(){
 checkRoutes();
 assert.equal(typeof encoder.encodeCommitToken,"undefined");
 assert.equal(typeof encoder.encodeCommit,"function");
 for(const name of ["decodeTradeBaiUrbun","decodeTradeMurabaha",
    "decodeCrossLayerBridge","decodeSmartContractBind"])
    assert.equal(typeof decoder[name],"undefined",name);
 assert.equal(typeof decoder.decodeAbstractionBridge,"function");
 assert.throws(()=>types.encodePayload(4,{}),{name:"TypeError"});
 const att={revoke:0,id:7,targetAddress:"alice",metaData:"OK"};
 assert.equal(encoder.encodeIssueOrRevokeAttestation(att),"tl90,7,alice,OK");
 assert.equal(types.encodePayload(9,att),"tl9tl90,7,alice,OK");
 assert.equal(types.encodePayload(2,{sendAll:true,address:"alice"}),"tl21;alice");
 assert.equal(types.encodePayload(2,{sendAll:false,address:"alice",
   propertyId:5,amount:2}),"tl2tl20;alice;5;2");
 assert.equal(encoder.encodeCloseOracle(),"tlf");
 assert.equal(types.encodePayload(15,{id:11}),"tlftlf");
 assert.throws(()=>types.encodePayload(36,{}),{message:"Unknown transaction type"});

 const args=(id,marker,body,ref=[])=>[
   "fake-txid",id,marker,body,"adminA",ref,0,0,101
 ];
 assert.equal((await types.decodePayload(...args(7,"BAD","x"))).message,"Invalid payload");
 // txIndex.processTransaction supplies ONLY (txId,marker,payload).
 assert.equal((await types.decodePayload("fake","tl","90,7,alice,OK")).message,"Invalid payload");
 assert.equal((await types.decodePayload("fake","tl","tl")).message,"Unknown transaction type");
 const created=await types.decodePayload(...args(7,"tl","backupB,ListOne,site,desc"));
 assert.equal(created.valid,true);
 assert.equal(created.backupAddress,"backupB");
 assert.equal(calls.at(-1).method,"validateCreateWhitelist");
 const issue=await types.decodePayload(...args(9,"tl","0,7,alice,ok"));
 assert.equal(issue.valid,true);assert.equal(issue.id,7);
 assert.equal(calls.at(-1).method,"validateIssueOrRevokeAttestation");
 // Raw post-marker bytes still contain type selector; passing them as
 // decode body makes "91" non-equal to revoke flag "1".
 assert.equal((await types.decodePayload(...args(9,"tl","91,7,alice,ok"))).revoke,false);
 assert.equal((await types.decodePayload(...args(9,"tl","1,7,alice,ok"))).revoke,true);

 const before=calls.length;
 const noRefs=await types.decodePayload(...args(3,"tl","ignored",[]));
 assert.equal(calls.length,before);
 assert.equal(noRefs.valid,undefined);
 await assert.rejects(()=>types.decodePayload("fake",3,"tl","ignored","adminA"),
   {name:"TypeError"});
 const prior=calls.length;
 const closed=await types.decodePayload(...args(15,"tl","f"));
 assert.equal(closed,15);
 assert.equal(calls.length,prior+1);
 assert.equal(calls.at(-1).method,"validatePublishOracleData");
 assert.equal(calls.at(-1).type,"number");
 assert.equal(calls.at(-1).block,undefined);
 for(const id of [28,29,34,35]){
   await assert.rejects(()=>types.decodePayload(...args(id,"tl","0")),{name:"TypeError"});
 }
 for(const [id,body] of [[32,"synthetic-json"],[33,"0,1,1,home"]]){
  const n=calls.length;
  const result=await types.decodePayload(...args(id,"tl",body));
  assert.equal(result.valid,undefined);
  assert.equal(calls.length,n);
  assert.equal(result.valid == false,false); // legacy logic.typeSwitch guard
 }
 assert.equal(types.resolveReferenceAddress("literal"),"literal");
 assert.equal(types.resolveReferenceAddress([
    {},{address:""},{address:"first"},{address:"second"}]),"first");
 assert.equal(types.resolveReferenceAddress({senderAddress:"backup"}),"backup");
 assert.equal(types.resolveReferenceAddress({address:"main",senderAddress:"backup"}),"main");
 assert.equal(types.resolveReferenceAddress(null),"");
 console.log("PASS: exact 3 Git-blob-verified JS sources, 36 encoder/decoder/validator routes");
 console.log("PASS: Type4 undefined encoder; generic Type2/9/15 prefix and oracle ID bugs");
 console.log("PASS: txIndex 3-to-9 call, marker Error OBJECT, Type3 conditional skipped validation");
 console.log("PASS: Type15 wrong validator/primitive, Type28/29/34/35 absent decoder");
 console.log("PASS: Type32/33 decode without validator; Type11 reference precedence");
 console.log("BOUNDARY: all Validity calls mocked; NO historical authority or settlement");
}
test().catch(e=>{console.error("FAIL:",e?.stack||String(e));process.exitCode=1});
