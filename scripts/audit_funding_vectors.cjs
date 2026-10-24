#!/usr/bin/env node
"use strict";
// Stage 24B, pinned clearing.js@dca0888b:670-768. Independent BigInt projection.
// Synthetic integer-position inputs only; not a historical runtime oracle.
const fs = require("node:fs");
const path = require("node:path");
const CASES = [{"name":"positive_one_to_one","n":"100000000","r":"100000","p":[["alice","1","100000000"],["bob","-1","100000000"]]},{"name":"negative_one_to_one","n":"100000000","r":"-100000","p":[["alice","1","100000000"],["bob","-1","100000000"]]},{"name":"positive_weight_rounding","n":"510","r":"100000","p":[["longA","1","100"],["longB","1","100"],["longC","2","100"],["shortX","-4","100"]]},{"name":"negative_weight_rounding","n":"510","r":"-100000","p":[["shortA","-1","100"],["shortB","-1","100"],["shortC","-2","100"],["longX","4","100"]]},{"name":"balanced_oi_rounding","n":"510","r":"100000","p":[["long","2","100"],["shortA","-1","100"],["shortB","-1","100"]]},{"name":"positive_no_receivers","n":"100000000","r":"100000","p":[["long","2","100000000"]]},{"name":"negative_no_receivers","n":"100000000","r":"-100000","p":[["short","-2","100000000"]]},{"name":"no_positive_payers","n":"100000000","r":"100000","p":[["short","-4","100000000"]]},{"name":"zero_rate","n":"100000000","r":"0","p":[["alice","1","100000000"],["bob","-1","100000000"]]},{"name":"rounds_to_zero","n":"1","r":"100000","p":[["alice","2","100000000"],["bob","-2","100000000"]]},{"name":"insufficient_long_balance","n":"100000000","r":"100000","p":[["long","1","1"],["short","-1","100000000"]]},{"name":"positive_skew","n":"510","r":"100000","p":[["longA","1","100"],["longB","3","100"],["shortX","-4","100"]]},{"name":"negative_skew","n":"510","r":"-100000","p":[["shortA","-1","100"],["shortB","-3","100"],["longX","4","100"]]},{"name":"maximum_hourly_rate","n":"100000000","r":"125000","p":[["largeLong","5","5000000"],["largeShort","-5","5000000"]]}];
const S = 100000000n;
function amount(x){return x<0n?-x:x}
function roundHalfUp(n,d){if(d<=0n)throw Error("bad divisor"); const q=n/d,r=n%d;return q+(r>=d-r?1n:0n);}
function allocation(total,side,p,largestRemainder){
 if(!side.length)return [];
 const denom=side.reduce((s,i)=>s+amount(BigInt(p[i][1])),0n);
 let rows=side.map(i=>{const numerator=total*amount(BigInt(p[i][1]));return {i,value:largestRemainder?numerator/denom:roundHalfUp(numerator,denom),rem:numerator%denom}});
 if(largestRemainder){
   const base=rows.reduce((s,x)=>s+x.value,0n);
   const extras=Number(total-base);
   const chosen=[...rows].sort((a,b)=>a.rem!==b.rem?(a.rem>b.rem?-1:1):p[a.i][0]<p[b.i][0]?-1:1);
   for(const z of chosen.slice(0,extras))z.value+=1n;
 }
 return rows.map(x=>[x.i,x.value]);
}
function evaluate(c){
 const p=c.p,n=BigInt(c.n),rate=BigInt(c.r),positive=rate>0n;
 const pay=[],receive=[];
 for(let i=0;i<p.length;i++){const contracts=BigInt(p[i][1]);if(contracts===0n)continue;if((contracts>0n)===positive)pay.push(i);else receive.push(i);}
 const individual=pay.map(i=>[i,roundHalfUp(amount(BigInt(p[i][1]))*n*amount(rate),S)]);
 const total=individual.reduce((a,z)=>a+z[1],0n);
 const legacy=Array(p.length).fill(0n),cons=Array(p.length).fill(0n);
 let proposedError=null;
 if(total>0n){
  for(const [i,a] of allocation(total,pay,p,false))legacy[i]+=positive?-a:a;
  for(const [i,a] of allocation(total,receive,p,false))legacy[i]+=positive?a:-a;
  if(!receive.length)proposedError="MissingCounterparty";
  else{
   for(const [i,a] of individual)cons[i]-=a;
   for(const [i,a] of allocation(total,receive,p,true))cons[i]+=a;
   for(let i=0;i<p.length;i++){if(cons[i]<0n && amount(cons[i])>BigInt(p[i][2])){proposedError="InsufficientAvailable";break;}}
  }
 }
 const encode=(ds)=>p.map((position,i)=>`${position[0]}:${ds[i]}`).join(",");
 const after=proposedError?"ERR:"+proposedError:p.map((z,i)=>`${z[0]}:${BigInt(z[2])+cons[i]}`).join(",");
 return [c.name,c.n,c.r,p.map(z=>z.join(":")).join(","),total.toString(),encode(legacy),legacy.reduce((s,v)=>s+v,0n).toString(),proposedError?"ERR:"+proposedError:encode(cons),proposedError?"-":cons.reduce((s,v)=>s+v,0n).toString(),after];
}
const header = "# name\tnotional_atoms\trate_bps_e4\tpositions_account_contracts_available\tnominal_atoms\tlegacy_attempted_deltas\tlegacy_net_atoms\tconserving_deltas_or_error\tconserving_net_atoms\tconserving_preview_balances";
const fixture = header+"\n"+CASES.map(c=>evaluate(c).join("\t")).join("\n")+"\n";
const destination = path.join(__dirname,"..","fixtures","funding_audit_vectors.tsv");
if(process.argv.includes("--check")) { if(fs.readFileSync(destination,"utf8")!==fixture) throw Error("Funding audit fixture drift"); }
else if(process.argv.length===2) fs.writeFileSync(destination,fixture); else throw Error("usage: node scripts/audit_funding_vectors.cjs [--check]");
console.log("Stage 24B independent BigInt funding vectors:",CASES.length);
