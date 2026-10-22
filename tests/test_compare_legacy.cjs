'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {compare} = require('../scripts/compare_legacy_js.cjs');

const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'tradelayer-legacy-diff-'));
const decoder = path.join(__dirname, 'fixtures', 'minimal_decoder.cjs');
const txid = 'a'.repeat(64);
try {
  const rows = [
    ['# txid', 'vout', 'type', 'body_hex', 'rust_kind', 'field1', 'field2', 'field3', 'field4'].join('\t'),
    [txid, '0', '2', Buffer.from('0;alice;5;1').toString('hex'), 'send', '5', '100000000', 'alice', '-'].join('\t'),
    [txid, '1', '2', Buffer.from('0;alice;5;1').toString('hex'), 'send', '5', '1', 'alice', '-'].join('\t'),
    [txid, '2', '2', Buffer.from('0;alice;5;1~').toString('hex'), 'send', '5', '1', 'alice', '-'].join('\t'),
    [txid, '3', '1', Buffer.from('a,FOO,,1,,0,0,').toString('hex'), 'issue', '10', 'FOO', '', '1'].join('\t'),
    [txid, '4', '5', Buffer.from('opaque').toString('hex'), 'unsupported', '-', '-', '-', '-'].join('\t'),
  ];
  const file = path.join(temp, 'fields.tsv');
  fs.writeFileSync(file, rows.join('\n'));
  const result = compare(file, decoder);
  assert.equal(result.matched, 3);
  assert.equal(result.disagreed, 1);
  assert.equal(result.inconclusive, 1);
  assert.equal(result.records[1].verdict, 'disagree');
  console.log('legacy comparison harness: 3 match, 1 disagreement, 1 inconclusive (fake decoder)');
} finally {
  fs.rmSync(temp, {recursive: true, force: true});
}
