#!/usr/bin/env node
// Independent bigint oracle for bounded Stage 19 rational matching.
// Legacy JS diagnostic: tradelayer.js dca0888b827c5bc28fa9189bc9e1d507f5c5670c
// orderbook.js calculatePrice divides offered/expected on both sides.
// This fixture records the mismatch; it is not consensus equivalence.
const fs = require("node:fs");
const path = require("node:path");
const ROOT = path.resolve(__dirname, "..");
const cases = [
  ["maker_sell",100,200,220,100],
  ["uncrossed",100,201,200,100],
  ["exact",100,200,200,100],
  ["fractional",3,1,1,2],
  ["partial_buy",80,120,200,100],
  ["wide",50,125,180,60],
  ["high_precision",7,10,15,7],
  ["tight_quoted",90,100,100,90]
];
function ceil(n, d) { return (n + d - 1n) / d; }
function round8(n, d) { return (n * 100000000n * 2n + d) / (2n * d); }
function quote(sBase, sWant, bOffer, bWant, maker) {
  if (bOffer * sBase < sWant * bWant) return "none";
  const base = sBase < bWant ? sBase : bWant;
  const paid = maker === "sell" ? ceil(base * sWant, sBase) : ceil(base * bOffer, bWant);
  const valid = paid > 0n && paid <= bOffer &&
    paid * sBase >= base * sWant &&
    paid * bWant <= base * bOffer;
  return valid ? base.toString() + ":" + paid.toString() : "reject";
}
const lines = ["#name\tsell_base\tsell_want_quote\tbuy_offer_quote\tbuy_want_base\tsell_maker\tbuy_maker\tlegacy_sell_offered_over_expected_e8"];
for (const [label, s, sw, b, bw] of cases) {
  const [a, c, d, e] = [s, sw, b, bw].map(BigInt);
  lines.push([
    label, s, sw, b, bw, quote(a,c,d,e,"sell"),
    quote(a,c,d,e,"buy"), round8(a,c).toString()
  ].join("\t"));
}
const expected = lines.join("\n") + "\n";
const target = path.join(ROOT, "fixtures", "spot_rational_vectors.tsv");
if (process.argv.includes("--check")) {
  if (fs.readFileSync(target, "utf8") !== expected) {
    throw Error("Stage 19 independent spot rational corpus drift");
  }
} else {
  fs.writeFileSync(target, expected);
}
console.log("Stage 19 independent bigint spot vectors: " + cases.length + " checked");
