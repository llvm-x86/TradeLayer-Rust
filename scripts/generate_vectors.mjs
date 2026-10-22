#!/usr/bin/env node
/**
 * Offline, dependency-free reproduction of a limited set of source expressions.
 *
 * Source: patrickdugan/tradelayer.js at the source revisions documented in
 * docs/WIRE_FORMAT.md. These functions are transcribed from txEncoder.js and
 * types.js. This is NOT the result of importing/running the complete JS repo.
 *
 * Generate: node scripts/generate_vectors.mjs > fixtures/wire_vectors.tsv
 * Verify:   node scripts/generate_vectors.mjs --check
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const b36 = value => value.toString(36);

function activation({ txTypeToActivate, codeHash }) {
  const types = Array.isArray(txTypeToActivate)
    ? txTypeToActivate.join(';') : txTypeToActivate;
  return `tl0${types},${BigInt(`0x${codeHash}`).toString(36)}`;
}

function issue({ initialAmount, ticker, whitelists, managed, backupAddress = '', nft = false,
                 coloredCoinHybrid = false, proceduralType }) {
  const fields = [
    b36(initialAmount), ticker, whitelists.map(b36).join(','),
    managed ? '1' : '0', backupAddress, nft ? '1' : '0',
    coloredCoinHybrid ? '1' : '0', proceduralType?.toString(36) ?? '',
  ];
  return `tl1${fields.join(',')}`;
}

function send({ sendAll = false, address, propertyId, amount }) {
  if (sendAll) return `1;${address}`; // Historical missing envelope.
  const scaled = Math.trunc(amount * 1e8); // Exact for these chosen vectors.
  const whole = (amount % 1) === 0;
  const encoded = whole
    ? amount?.toString(36) ?? '0'
    : scaled?.toString(36) ?? '0' + '~'; // Historical operator-precedence bug.
  return `tl20;${address};${b36(propertyId)};${encoded}`;
}

function grant({ propertyId, amount, recipient }) {
  const atomic = Math.round(amount * 1e8);
  return `tlb${[b36(propertyId), b36(atomic), recipient, '', '', '', '', ''].join(',')}`;
}
function redeem({ propertyId, amount }) {
  const atomic = Math.round(amount * 1e8);
  return `tlc${[b36(propertyId), b36(atomic), '', '', ''].join(',')}`;
}

const issuance = issue({ initialAmount: 10, ticker: 'FOO', whitelists: [], managed: true });
const sendTwo = send({ address: 'alice', propertyId: 5, amount: 2 });
const fractional = send({ address: 'alice', propertyId: 5, amount: 0.00000001 });

assert.equal(issuance, 'tl1a,FOO,,1,,0,0,');
assert.equal(sendTwo, 'tl20;alice;5;2');
assert.equal(fractional, 'tl20;alice;5;1');
assert.equal(send({ sendAll: true, address: 'alice' }), '1;alice');
assert.equal(grant({ propertyId: 5, amount: 5, recipient: 'bob' }), 'tlb5,89oqgw,bob,,,,,');
assert.equal(redeem({ propertyId: 5, amount: 1 }), 'tlc5,1njchs,,,');

// Each row: test-id, raw ASCII wire, expected type decimal, body, typed parser disposition.
// 'ok' only describes syntactic parsing. None of these fixtures prove consensus validity.
const cases = [
  ['activation', activation({txTypeToActivate: [0, 2, 11, 12], codeHash: 'ff'}), 0, 'ok'],
  ['issue-empty-whitelist', issuance, 1, 'ok'],
  ['issue-one-whitelist', issue({initialAmount: 10, ticker: 'FOO', whitelists: [1], managed: true}), 1, 'ok'],
  ['issue-multi-whitelist-misaligned', issue({initialAmount: 10, ticker: 'FOO', whitelists: [1, 2], managed: true}), 1, 'reject'],
  ['send-whole', sendTwo, 2, 'ok'],
  ['send-fractional-missing-tilde', fractional, 2, 'ok'],
  ['send-fractional-repaired-example', 'tl20;alice;5;1~', 2, 'ok'],
  ['send-all-wrapped-example', 'tl21;alice', 2, 'ok'],
  ['managed-grant', grant({ propertyId: 5, amount: 5, recipient: 'bob' }), 11, 'ok'],
  ['managed-redeem', redeem({ propertyId: 5, amount: 1 }), 12, 'ok'],
  ['managed-grant-with-hash', 'tlb5,89oqgw,bob,,,,,abc', 11, 'ok'],
  ['double-marker-send', 'tl2' + sendTwo, 2, 'reject'],
  ['double-marker-issue', 'tl1' + issuance, 1, 'reject'],
  ['recognized-unimplemented-type', 'tlzopaque', 35, 'unsupported'],
  ['uppercase-base36-selector', 'tlAopaque', 10, 'unsupported'],
  ['empty-activation', 'tl0', 0, 'reject'],
];

for (const [id, wire, type] of cases) {
  assert(wire.startsWith('tl'));
  assert.equal(Number.parseInt(wire[2], 36), type, id);
}
const header = '# test-id\twire\ttype-decimal\tbody\ttyped-status\n';
const contents = header + cases.map(([id, wire, type, status]) =>
  [id, wire, type, wire.slice(3), status].join('\t')
).join('\n') + '\n';

if (process.argv.includes('--check')) {
  const file = readFileSync(new URL('../fixtures/wire_vectors.tsv', import.meta.url), 'utf8');
  assert.equal(file, contents, 'fixture file differs from source-derived generator');
  console.log(`Verified ${cases.length} source-derived vectors`);
} else {
  process.stdout.write(contents);
}
