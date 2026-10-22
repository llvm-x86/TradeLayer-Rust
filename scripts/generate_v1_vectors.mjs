#!/usr/bin/env node
/**
 * Dependency-free reference vectors for the *proposed* version-1 binary wire
 * format. This intentionally has no compatibility fallback to legacy `tl`.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const header = Buffer.from([0x54, 0x4c, 0x52, 1]); // `TLR` + version 1.
function frame(type, body) {
  return Buffer.concat([header, Buffer.from([type]), Buffer.from(body)]);
}
function send(property, atoms, recipient) {
  const addr = Buffer.from(recipient, 'ascii');
  const body = Buffer.alloc(14 + addr.length);
  body[0] = 0;
  body.writeUInt32BE(property, 1);
  body.writeBigUInt64BE(BigInt(atoms), 5);
  body[13] = addr.length;
  addr.copy(body, 14);
  return frame(2, body);
}
function sendAll(recipient) {
  const addr = Buffer.from(recipient, 'ascii');
  return frame(2, Buffer.concat([Buffer.from([1, addr.length]), addr]));
}
assert.equal(send(5, 1, 'alice').toString('hex'), '544c5201020000000005000000000000000105616c696365');
assert.equal(sendAll('bob').toString('hex'), '544c5201020103626f62');

const cases = [
  ['direct-one-atom', send(5, 1, 'alice'), 'direct', '5', '1', 'alice'],
  ['direct-one-token', send(5, 100000000, 'alice'), 'direct', '5', '100000000', 'alice'],
  ['direct-large', send(4294967295, 18446744073709551615n, 'Z'), 'direct', '4294967295', '18446744073709551615', 'Z'],
  ['send-all', sendAll('bob'), 'all', '', '', 'bob'],
  ['unknown-type-36', frame(36, []), 'reject', '', '', ''],
  ['legacy-ascii', Buffer.from('tl20;alice;5;2'), 'reject', '', '', ''],
  ['legacy-double-prefix', Buffer.from('tl2tl20;alice;5;2'), 'reject', '', '', ''],
  ['wrong-version', Buffer.from('544c520202', 'hex'), 'reject', '', '', ''],
  ['zero-amount', send(5, 0, 'alice'), 'reject', '', '', ''],
  ['zero-property', send(0, 1, 'alice'), 'reject', '', '', ''],
  ['short-send', Buffer.from('544c52010200', 'hex'), 'reject', '', '', ''],
  ['trailing-bytes', Buffer.concat([sendAll('bob'), Buffer.from([0])]), 'reject', '', '', ''],
  ['truncated-address', Buffer.from('544c5201020103626f', 'hex'), 'reject', '', '', ''],
  ['unknown-send-variant', Buffer.from('544c5201027f', 'hex'), 'reject', '', '', ''],
  ['blank-address', sendAll(''), 'reject', '', '', ''],
];
const out = '# name\thex\tcategory\tproperty\tatoms\trecipient\n'
  + cases.map(([name, bytes, category, property, atoms, recipient]) =>
    [name, bytes.toString('hex'), category, property, atoms, recipient].join('\t')
  ).join('\n') + '\n';
if (process.argv.includes('--check')) {
  assert.equal(
    out,
    readFileSync(new URL('../fixtures/v1_vectors.tsv', import.meta.url), 'utf8'),
    'the v1 fixture corpus differs from its generator'
  );
  console.log(`Verified ${cases.length} proposed v1 vectors`);
} else {
  process.stdout.write(out);
}
