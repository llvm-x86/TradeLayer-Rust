#!/usr/bin/env node
"use strict";
// Standalone BigInt oracle independently implementing the formulas in
// tradelayer.js@dca0888b src/clearing.js:50-64, 661-668, 2189-2208.
// Synthetic integer-unit subset only. No consensus validity or signers.
const fs = require("node:fs");
const path = require("node:path");
const CASES = [["linear_long","2","10000000000","12000000000","100000000","0","10","100000000","1","0","0"],["linear_short","-3","12000000000","10000000000","200000000","0","-9","300","3","-4","2"],["inverse_long","2","10000000000","12000000000","10000000000","1","5","0","1","0","0"],["inverse_short","-2","10000000000","12000000000","10000000000","1","-5","0","1","0","0"],["inverse_fraction","7","303000000","315000000","123456789","1","4","-100","27","3","2"],["inverse_negative","-7","303000000","315000000","123456789","1","-4","-100","27","-3","2"],["half_atom_round","1","100000000","100000001","50000000","0","6","0","1","0","0"],["half_atom_negative","-1","100000000","100000001","50000000","0","-6","0","1","0","0"],["zero_from","5","0","12000000000","100000000","1","0","10","1","-20","0"],["zero_to","5","11000000000","0","100000000","0","12","0","0","0","1"],["zero_notional_defaults_to_one","3","10000000000","12000000000","0","0","-12","5","2","0","0"],["flat","-100","10000000000","10000000000","500000000","1","1","0","1","0","0"],["20dp_inverse_divergence","100000","93965940005094","143291410577395","1000000000000","1","5","100","0","-10","500"],["micro_inverse","1","110000000","110000001","100000000","1","-1","1","3","0","0"],["large_signed_linear","-8500","77700000000","77800000000","12000000000","0","6","-2000","25","-400","1500"]];
const SCALE = 100000000n;
const DECIMAL_20 = 10n ** 20n;
const COLUMNS = [
  "#name", "contracts", "from_price_e8", "to_price_e8", "notional_e8",
  "inverse", "legacy_pnl_atoms", "exact_pnl_atoms", "funding_bps",
  "funding_clamped_bps", "available_atoms", "margin_atoms",
  "premium_mtm_atoms", "naked_maint_atoms", "coverage_twice_atoms"
];
function halfUp(signedNumerator, positiveDenominator) {
  if (positiveDenominator <= 0n) throw Error("invalid denominator");
  const negative = signedNumerator < 0n;
  const abs = negative ? -signedNumerator : signedNumerator;
  const quotient = abs / positiveDenominator;
  const remainder = abs % positiveDenominator;
  const magnitude = quotient + (remainder >= positiveDenominator - remainder ? 1n : 0n);
  return negative ? -magnitude : magnitude;
}
function pnl(contracts, from, to, notional, inverse, legacy20dp) {
  if (contracts === 0n || from === 0n || to === 0n) return 0n;
  const n = notional === 0n ? SCALE : notional;
  if (!inverse) return halfUp((to - from) * contracts * n, SCALE);
  if (!legacy20dp) {
    return halfUp((to - from) * contracts * n * SCALE, from * to);
  }
  // Pinned JS BigNumber default uses 20 decimal places for each 1/price,
  // before subtraction and final eight-decimal rounding.
  const reciprocalFrom = halfUp(SCALE * DECIMAL_20, from);
  const reciprocalTo = halfUp(SCALE * DECIMAL_20, to);
  return halfUp((reciprocalFrom - reciprocalTo) * contracts * n, DECIMAL_20);
}
function clampBps(rate) {
  if (rate > 5n) return rate - 5n;
  if (rate < -5n) return rate + 5n;
  return 0n;
}
const rows = [COLUMNS.join("\t")];
let precisionDifferences = 0;
for (const data of CASES) {
  const [name, cRaw, fRaw, tRaw, nRaw, iRaw, rateRaw, availRaw, marginRaw, mtmRaw, nakedRaw] = data;
  const [c, f, t, n, i, rate, available, margin, mtm, naked] =
    data.slice(1).map(BigInt);
  const legacy = pnl(c, f, t, n, i !== 0n, true);
  const exact = pnl(c, f, t, n, i !== 0n, false);
  if (legacy !== exact) precisionDifferences++;
  const twiceCoverage = 2n * available + margin + 2n * mtm - 2n * naked;
  rows.push([
    name, cRaw, fRaw, tRaw, nRaw, iRaw, legacy, exact, rateRaw,
    clampBps(rate), availRaw, marginRaw, mtmRaw, nakedRaw, twiceCoverage
  ].map(String).join("\t"));
}
if (precisionDifferences !== 1) throw Error("unexpected reciprocal-rounding discrepancy");
const output = rows.join("\n") + "\n";
const target = path.join(__dirname, "..", "fixtures", "clearing_math_vectors.tsv");
if (process.argv.includes("--check")) {
  if (fs.readFileSync(target, "utf8") !== output) throw Error("Stage 24 clearing corpus drift");
} else if (process.argv.length === 2) {
  fs.writeFileSync(target, output);
} else {
  throw Error("usage: node scripts/audit_clearing_math.cjs [--check]");
}
console.log("Stage 24 independent BigInt clearing vectors: " + CASES.length +
  ", reciprocal-rounding differences: " + precisionDifferences);
