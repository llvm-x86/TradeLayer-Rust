#!/usr/bin/env node
"use strict";
// Independently source-derived main.js catch-up vs realtime scheduling oracle.
const fs = require("node:fs");
const path = require("node:path");
const CASES = [{"name":"due_empty","h":240,"tx":[]},{"name":"due_type4_only","h":240,"tx":[[4,0,1]]},{"name":"due_type20_only_k","h":240,"tx":[[20,0,1]]},{"name":"due_type22_only_m","h":240,"tx":[[22,0,1]]},{"name":"due_other_valid","h":240,"tx":[[18,0,1]]},{"name":"due_other_invalid","h":240,"tx":[[18,0,0]]},{"name":"due_other_already_processed","h":240,"tx":[[18,1,1]]},{"name":"due_type4_plus_other_valid","h":240,"tx":[[4,0,1],[18,0,1]]},{"name":"due_type20_plus_invalid","h":240,"tx":[[20,0,1],[18,0,0]]},{"name":"due_type22_plus_invalid","h":240,"tx":[[22,0,1],[18,0,0]]},{"name":"non_due_with_valid_work","h":241,"tx":[[18,0,1]]},{"name":"catchup_force_10000","h":10000,"tx":[]},{"name":"catchup_force_30000_due","h":30000,"tx":[]},{"name":"caught_up_funding_already_processed","h":240,"tx":[[4,1,1]]},{"name":"failed_logic_after_decode_true","h":240,"tx":[[18,0,1]]},{"name":"due_type4_invalid","h":240,"tx":[[4,0,0]]}];
function projection(path,height,tx){
 const typeFunding=ty=>path==="catchup"?(ty===4||ty===20):(ty===4||ty===22);
 const fb=tx.filter(t=>typeFunding(t[0])).length;
 const clearing=path==="catchup"?(height%10000===0||tx.some(t=>t[1]===0)):tx.some(t=>!typeFunding(t[0])&&t[1]===0&&t[2]===1);
 const eligible=height%24===0;
 return [fb,tx.length-fb,clearing?1:0,eligible?1:0,clearing&&eligible?1:0,(path==="realtime"||clearing)?1:0];
}
const header = "#scenario\tpath\theight\ttxs_type_processed_valid\tfunding_bucket\tother_bucket\tclearing_called\tfunding_height_eligible\tfunding_attempted\twithdrawals_attempted";
const rows = CASES.flatMap(c=>["catchup","realtime"].map(mode=>[c.name,mode,String(c.h),c.tx.map(t=>t.join(":")).join(","),...projection(mode,c.h,c.tx).map(String)]));
const expected = header+"\n"+rows.map(r=>r.join("\t")).join("\n")+"\n";
const target = path.join(__dirname,"..","fixtures","clearing_cadence_vectors.tsv");
if(process.argv.includes("--check")) { if(fs.readFileSync(target,"utf8")!==expected) throw Error("Cadence corpus drift"); }
else if(process.argv.length===2) fs.writeFileSync(target,expected); else throw Error("usage: node scripts/audit_clearing_cadence.cjs [--check]");
console.log("Stage 24C clearing scheduler vectors:",rows.length);
