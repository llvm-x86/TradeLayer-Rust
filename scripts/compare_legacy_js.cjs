#!/usr/bin/env node
'use strict';
// Optional side-by-side comparison using the actual historical JS decoder.
// Loading an arbitrary JavaScript file executes its top-level code. Run only
// with a trusted, pinned checkout in an isolated, networkless environment.
// The legacy dependencies must already exist in that checkout.
const fs = require('node:fs');
const path = require('node:path');

function normalize(decoded, type) {
  const dash = '-';
  if (type === 0) {
    const types = decoded.txTypesToActivate;
    if (!Array.isArray(types)) throw Error('missing activation types');
    return ['activate', types.join(';'), String(decoded.codeHash), dash, dash];
  }
  if (type === 1) {
    return ['issue', String(decoded.initialAmount), String(decoded.ticker),
      // The Rust parser deliberately preserves the raw whitelist field.
      // JS decoded whitelist values cannot always reconstruct raw commas.
      null, decoded.managed ? '1' : '0'];
  }
  if (type === 2) {
    if (decoded.sendAll) return ['all', String(decoded.address), dash, dash, dash];
    const amount = Number(decoded.amounts);
    const property = decoded.propertyIds;
    if (!Number.isFinite(amount) || !Number.isInteger(property)) throw Error('invalid JS send');
    const atomNumber = amount * 1e8;
    // JS Number loses integer precision beyond this bound. Mark inconclusive.
    if (!Number.isSafeInteger(atomNumber)) throw Error('legacy Number cannot be compared exactly');
    return ['send', String(property), String(atomNumber), String(decoded.address), dash];
  }
  if (type === 11 || type === 12) {
    const amount = Number(type === 11 ? decoded.amountGranted : decoded.amountDestroyed);
    if (!Number.isSafeInteger(amount * 1e8)) throw Error('legacy Number cannot be compared exactly');
    if (type === 11) return ['grant', String(decoded.propertyId), String(amount * 1e8), String(decoded.addressToGrantTo), dash];
    return ['redeem', String(decoded.propertyId), String(amount * 1e8), dash, dash];
  }
  return null;
}

function decode(legacy, type, body) {
  const methods = {0: 'decodeActivateTradeLayer', 1: 'decodeTokenIssue', 2: 'decodeSend',
    11: 'decodeGrantManagedToken', 12: 'decodeRedeemManagedToken'};
  const name = methods[type];
  if (!name || typeof legacy[name] !== 'function') throw Error('missing legacy decoder function');
  return legacy[name](body);
}

function compare(input, decoder) {
  // Legacy decoders print verbose debug messages. Keep reports machine-readable.
  const previousLog = console.log;
  let legacy;
  try {
    console.log = () => {};
    legacy = require(path.resolve(decoder)); // explicitly trusted local source
  } finally {
    console.log = previousLog;
  }
  let matched = 0, disagreed = 0, inconclusive = 0;
  const records = [];
  for (const [index, line] of fs.readFileSync(input, 'utf8').split(/\r?\n/).entries()) {
    if (!line || line.startsWith('#')) continue;
    const fields = line.split('\t');
    if (fields.length !== 9) throw Error(`bad Rust diagnostic TSV line ${index + 1}`);
    const [txid, vout, typeRaw, bodyHex, ...rust] = fields;
    const type = Number(typeRaw);
    if (![0, 1, 2, 11, 12].includes(type) || rust[0] === 'invalid') {
      inconclusive++;
      records.push({txid, vout, type, verdict: 'inconclusive', reason: 'unsupported or malformed'});
      continue;
    }
    const body = Buffer.from(bodyHex, 'hex').toString('ascii');
    try {
      let decoded;
      try {
        console.log = () => {};
        decoded = decode(legacy, type, body);
      } finally {
        console.log = previousLog;
      }
      const js = normalize(decoded, type);
      const differing = rust.map((value, i) => js[i] === null ? false : String(value) !== String(js[i]));
      const verdict = differing.some(Boolean) ? 'disagree' : 'match';
      if (verdict === 'match') matched++; else disagreed++;
      records.push({txid, vout, type, verdict, rust, js});
    } catch (err) {
      inconclusive++;
      records.push({txid, vout, type, verdict: 'inconclusive', reason: String(err.message)});
    }
  }
  return {source: path.resolve(decoder), matched, disagreed, inconclusive, records};
}

if (require.main === module) {
  const args = process.argv.slice(2);
  if (args.length !== 2) {
    console.error('usage: node scripts/compare_legacy_js.cjs <rust-fields.tsv> <trusted-tradelayer.js/src/txDecoder.js>');
    process.exit(2);
  }
  const result = compare(args[0], args[1]);
  console.log(JSON.stringify(result, null, 2));
  if (result.disagreed) process.exitCode = 1;
}
module.exports = {normalize, compare};
