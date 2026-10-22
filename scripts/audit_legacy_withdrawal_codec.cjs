#!/usr/bin/env node
'use strict';
// Independent, dependency-free reproduction of the base mismatch in
// tradelayer.js src/txEncoder.js encodeWithdrawal / src/txDecoder.js
// decodeWithdrawal. It intentionally does NOT execute legacy npm modules.
// Source commit: dca0888b827c5bc28fa9189bc9e1d507f5c5670c.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
function parseAsBase36(decimalText) {
  let result = 0n;
  for (const char of decimalText) {
    if (char < '0' || char > '9') throw new Error('not decimal ASCII');
    result = result * 36n + BigInt(char.charCodeAt(0) - 48);
  }
  return result;
}
const fixture = fs.readFileSync(path.join(__dirname, '../fixtures/withdrawal_codec_vectors.tsv'), 'utf8');
let count = 0;
for (const line of fixture.split(/\r?\n/)) {
  if (!line || line.startsWith('#')) continue;
  const columns = line.split('\t');
  assert.equal(columns.length, 3, 'fixture needs three columns');
  const atoms = BigInt(columns[0]);
  assert.ok(atoms >= 0n && atoms <= MAX_SAFE);
  // The historical encoder serializes Number(amount * 1e8).toString() in
  // decimal, while the decoder treats the same text as base 36.
  const wire = Number(atoms).toString(10);
  assert.equal(wire, columns[1]);
  const decodedAtoms = parseAsBase36(wire);
  assert.equal(decodedAtoms, BigInt(columns[2]));
  count++;
}
assert.ok(count >= 6, 'fixture unexpectedly short');
console.log('Legacy type-21 decimal/base36 mismatch: ' + count + ' exact-integer vectors verified.');
