#!/usr/bin/env node
"use strict";
// Independent, stdlib-only, BigInt analog of pinned JS liquid loss pool
// (clearing.js:2148–2173) + hypothetical insurance cap (insurance.js:190–239).
// These are DIAGNOSTIC vectors, not observed insurance funds or authorizations.
const fs = require("node:fs");
const path = require("node:path");
const CASES = [["zero","legacy","0","0","0","0"],["paid_by_available","legacy","100","200","40","30"],["posted_only_free_untouched","posted","100","200","40","30"],["covered_by_margin","posted","100","0","100","0"],["falls_to_insurance","posted","100","0","30","70"],["insurance_insufficient","posted","100","0","30","40"],["fully_exhausted_legacy","legacy","1000","200","300","400"],["available_then_margin","legacy","90","60","45","100"],["insurance_overfunded","posted","1","9999","0","9999"],["margin_overfunded","posted","1","9999","9999","9999"],["zero_insurance","posted","100","0","10","0"],["only_hypothetical_insurance","posted","120","0","0","60"],["near_u128_limit","legacy","340282366920938463463374607431768211455","340282366920938463463374607431768211453","100","100"],["u128_max_unfunded","posted","340282366920938463463374607431768211455","0","0","0"],["small_atom_residual","legacy","3","1","1","0"],["small_atom_complete","posted","3","1","2","1"]];
const header = "#name\trule\tdemanded_atoms\tfree_available_atoms\tmargin_atoms\tobserved_insurance_atoms\tavailable_debit_atoms\tmargin_debit_atoms\tinsurance_debit_atoms\tunresolved_atoms\tfully_accounted";
function one([name, rule, demanded, available, margin, observedInsurance]) {
  let remainder = BigInt(demanded);
  const free = BigInt(available);
  const posted = BigInt(margin);
  const insurance = BigInt(observedInsurance);
  const useAvailable = rule === "legacy" ? (remainder < free ? remainder : free) : 0n;
  remainder -= useAvailable;
  const useMargin = remainder < posted ? remainder : posted;
  remainder -= useMargin;
  const useInsurance = remainder < insurance ? remainder : insurance;
  remainder -= useInsurance;
  const fullyAccounted = useAvailable + useMargin + useInsurance + remainder === BigInt(demanded);
  return [name, rule, demanded, available, margin, observedInsurance, useAvailable, useMargin, useInsurance, remainder, fullyAccounted ? "1" : "0"].map(String).join("\t");
}
const output = header + "\n" + CASES.map(one).join("\n") + "\n";
const filename = path.join(__dirname, "..", "fixtures", "liquidation_loss_budget_vectors.tsv");
if (process.argv.includes("--check")) {
  if (fs.readFileSync(filename, "utf8") !== output) throw Error("stage25e1 fixture drift");
} else if (process.argv.length === 2) {
  fs.writeFileSync(filename, output);
} else {
  throw Error("usage: node scripts/audit_liquidation_loss_budget.cjs [--check]");
}
console.log("Stage 25E1 independent source-inspired hypothetical budgets:", CASES.length);
