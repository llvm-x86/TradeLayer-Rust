#!/usr/bin/env node
"use strict";
// Optional Stage 24B execution of the ORIGINAL pinned clearing.js implementation.
// Run only with a trusted local checkout in an isolated, networkless environment.
// The harness mocks the side-effectful imports and records only attempted
// Tally.updateBalance requests. It never invokes wallet/RPC or writes a ledger.
const fs = require("node:fs");
const path = require("node:path");
const cp = require("node:child_process");
const Module = require("node:module");

const LEGACY_COMMIT = "dca0888b827c5bc28fa9189bc9e1d507f5c5670c";

async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== "--js") {
    throw Error("usage: node scripts/check_legacy_funding_runtime.cjs --js <trusted-pinned-tradelayer.js>");
  }
  const legacy = fs.realpathSync(args[1]);
  const actual = cp.execFileSync(
    "git", ["-C", legacy, "rev-parse", "HEAD"], {encoding:"utf8", timeout:10_000}
  ).trim();
  if (actual !== LEGACY_COMMIT) {
    throw Error("legacy checkout is not pinned to the exact reference commit");
  }
  const dirty = cp.execFileSync(
    "git", ["-C", legacy, "status", "--porcelain", "--untracked-files=no"],
    {encoding:"utf8", timeout:10_000}
  ).trim();
  if (dirty) throw Error("legacy tracked worktree is dirty; refuse source execution");
  const source = fs.realpathSync(path.join(legacy, "src", "clearing.js"));
  const BigNumber = require(require.resolve("bignumber.js", {paths: [legacy]}));
  const conf = BigNumber.config();
  if (conf.DECIMAL_PLACES !== 20 || conf.ROUNDING_MODE !== 4) {
    throw Error("unexpected BigNumber division or rounding configuration");
  }
  const sourceText = fs.readFileSync(source, "utf8");
  for (const expected of [
    "static async applyFundingToPositions(",
    "static async processFundingPayments(",
    "fundingRate < 0 && pos.contracts < 0",
    "const fundingAmount = contractsBN",
    "-amountOwed.toNumber()",
    "amountReceived.toNumber()"
  ]) {
    if (!sourceText.includes(expected)) throw Error("pinned funding source drift: " + expected);
  }

  const fixture = fs.readFileSync(
    path.join(__dirname, "..", "fixtures", "funding_audit_vectors.tsv"), "utf8"
  );
  const rows = fixture.split(/\r?\n/).filter(r => r && !r.startsWith("#"));
  const calls = [];
  let activePositions = [];
  let activeNotional = 0;
  const mocks = {
    "./tally.js": {
      updateBalance: async (address, propertyId, availableChange, reserved, margin, vesting, reason, block) => {
        if (reserved !== 0 || margin !== 0 || vesting !== 0) throw Error("unexpected non-available funding delta");
        if (propertyId !== 7 || block !== 48) throw Error("unexpected collateral property or block");
        const scaled = new BigNumber(availableChange).times(100000000);
        if (!scaled.isInteger()) throw Error("observed delta is not atomic");
        calls.push({address, atoms:BigInt(scaled.toFixed(0)), reason});
      }
    },
    "./contractRegistry.js": {
      getCollateralId: async () => 7,
      getNotionalValue: async () => ({notionalValue: activeNotional}),
    },
    "./marginMap.js": {
      getInstance: async () => ({getAllPositions: async () => activePositions}),
    },
  };
  const originalLoad = Module._load;
  function hooked(request, parent, isMain) {
    if (parent && parent.filename === source && request.startsWith("./")) {
      return Object.hasOwn(mocks, request) ? mocks[request] : {};
    }
    return originalLoad.call(this, request, parent, isMain);
  }
  const originalLog = console.log;
  const originalWarn = console.warn;
  let passed = 0;
  try {
    Module._load = hooked;
    const Clearing = require(source);
    Clearing.getIndexPrice = async () => 100;
    console.log = () => {};
    console.warn = () => {};
    for (const row of rows) {
      const cols = row.split("\t");
      if (cols.length !== 10) throw Error("malformed funding fixture");
      const [name, _notional, _rate, _positions, _total, legacyDeltas, legacyNet] = cols;
      activePositions = _positions.split(",").map(item => {
        const [address, contracts] = item.split(":");
        return {address, contracts:Number(contracts)};
      });
      activeNotional = Number(_notional) / 100000000;
      calls.length = 0;
      const rate = Number(_rate) / 10000;
      await Clearing.applyFundingToPositions(7, rate, 48);
      const observed = new Map(activePositions.map(p => [p.address, 0n]));
      for (const item of calls) {
        observed.set(item.address, observed.get(item.address) + item.atoms);
      }
      const actualDeltas = activePositions
        .map(p => p.address + ":" + observed.get(p.address).toString()).join(",");
      const actualNet = [...observed.values()].reduce((sum, atom) => sum + atom, 0n).toString();
      if (actualDeltas !== legacyDeltas || actualNet !== legacyNet) {
        throw Error("Legacy runtime divergence in " + name +
          ": expected " + legacyDeltas + " / " + legacyNet +
          ", got " + actualDeltas + " / " + actualNet);
      }
      passed++;
    }
  } finally {
    Module._load = originalLoad;
    console.log = originalLog;
    console.warn = originalWarn;
  }
  console.log("Pinned legacy clearing.js attempted funding deltas matched:", passed, "synthetic cases");
  console.log("This is a mock-balance characterization; no real transfers were executed.");
}

main().catch(error => {
  console.error(error.stack || error.message);
  process.exitCode = 1;
});
