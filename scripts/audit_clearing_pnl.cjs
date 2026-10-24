#!/usr/bin/env node
"use strict";
// Independent BigInt projection of the two Stage 24A 8-decimal mark policies.
// Synthetic signed position books, no network or legacy JS runtime.
const fs = require("node:fs");
const path = require("node:path");
const CASES = [["linear_pair","10000000000","10100000000","100000000","0","exact",[["alice",1],["bob",-1]]],["linear_pair_legacy","10000000000","10100000000","100000000","0","legacy",[["alice",1],["bob",-1]]],["inverse_pair","10000000000","12000000000","10000000000","1","legacy",[["alice",1],["bob",-1]]],["inverse_pair_exact","10000000000","12000000000","10000000000","1","exact",[["alice",1],["bob",-1]]],["linear_round_residual","10000000000","10000000001","50000000","0","exact",[["alice",1],["bob",-2],["carol",1]]],["linear_round_balanced","10000000000","10000000002","50000000","0","exact",[["alice",1],["bob",-2],["carol",1]]],["no_mark_change","10000000000","10000000000","100000000","0","exact",[["alice",2],["bob",-2]]],["down_price","10100000000","9900000000","100000000","0","exact",[["alice",1],["bob",-1]]],["precision_old","93965940005094","143291410577395","1000000000000","1","legacy",[["alice",100000],["bob",-100000]]],["precision_exact","93965940005094","143291410577395","1000000000000","1","exact",[["alice",100000],["bob",-100000]]],["empty_active","10000000000","10100000000","100000000","0","exact",[]]];
const ONE = 100000000n;
const DEC = 10n ** 20n;
function roundHalfUp(numerator, denominator) {
  if (denominator <= 0n) throw Error("invalid divisor");
  const negative = numerator < 0n;
  const absolute = negative ? -numerator : numerator;
  const quotient = absolute / denominator;
  const remainder = absolute % denominator;
  const magnitude = quotient + (remainder >= denominator - remainder ? 1n : 0n);
  return negative ? -magnitude : magnitude;
}
function mark(c, from, to, notional, inverse, mode) {
  if (!inverse) return roundHalfUp((to - from) * c * notional, ONE);
  if (mode === "legacy") {
    const reciprocalFrom = roundHalfUp(ONE * DEC, from);
    const reciprocalTo = roundHalfUp(ONE * DEC, to);
    return roundHalfUp((reciprocalFrom - reciprocalTo) * c * notional, DEC);
  }
  return roundHalfUp((to - from) * c * notional * ONE, from * to);
}
const columns = ["#name","from_price_e8","to_price_e8","notional_e8","inverse","rounding","positions","account_deltas","net_atoms","payer_atoms","receiver_atoms"];
const rows = [columns.join("\t")];
for (const [name, fromText, toText, notionalText, inverse, mode, positions] of CASES) {
  const from = BigInt(fromText), to = BigInt(toText), notional = BigInt(notionalText);
  const projected = positions.map(([owner, contracts]) =>
    [owner, mark(BigInt(contracts), from, to, notional, inverse === "1", mode)]
  ).sort((a, b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0);
  const net = projected.reduce((s, p) => s + p[1], 0n);
  const payer = projected.reduce((s, p) => s + (p[1] < 0n ? -p[1] : 0n), 0n);
  const receiver = projected.reduce((s, p) => s + (p[1] > 0n ? p[1] : 0n), 0n);
  rows.push([
    name, fromText, toText, notionalText, inverse, mode,
    positions.map(p => p.join(":")).join(","),
    projected.map(p => p.join(":")).join(","),
    net, payer, receiver
  ].join("\t"));
}
const output = rows.join("\n") + "\n";
const file = path.join(__dirname, "..", "fixtures", "clearing_pnl_vectors.tsv");
if (process.argv.includes("--check")) {
  if (fs.readFileSync(file, "utf8") !== output) throw Error("PnL fixture drift");
} else if (process.argv.length === 2) {
  fs.writeFileSync(file, output);
} else {
  throw Error("usage: node scripts/audit_clearing_pnl.cjs [--check]");
}
console.log("Stage 25C independent BigInt mark PnL projections:", CASES.length);
