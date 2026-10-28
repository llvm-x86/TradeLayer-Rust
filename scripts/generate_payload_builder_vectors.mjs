#!/usr/bin/env node
/**
 * Payload-builder parity vectors for the Rust direct legacy encoders.
 *
 * Runs the frozen src/txEncoder.js and src/txDecoder.js from
 * patrickdugan/tradelayer.js@dca0888 (git blob hashes checked) inside Node's vm
 * module, using the vendored bignumber.js 9.1.2 that the JS package-lock pins
 * (sha256 checked). Nothing is imported from the wider JS repo and nothing
 * touches the network.
 *
 * Each row records:
 *   - the JS encoder output for one fixed input (js_payload),
 *   - the JS decoder reading of that output (js_decoded),
 *   - the outcome the Rust builders must produce (rust_outcome).
 * parity is "match" only when rust_outcome is byte-identical to js_payload.
 * Every other row is a deliberate divergence and must carry a note; see
 * docs/PAYLOAD_BUILDERS.md. tests/payload_builders.rs checks rust_outcome
 * against the Rust implementation.
 *
 * Generate: node scripts/generate_payload_builder_vectors.mjs > fixtures/payload_builder_js_vectors.tsv
 * Verify:   node scripts/generate_payload_builder_vectors.mjs --check
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const FIXTURE = 'fixtures/payload_builder_js_vectors.tsv';

function readBytes(relative) {
  return readFileSync(join(ROOT, relative));
}

function gitBlobSha1(raw) {
  const header = Buffer.from(`blob ${raw.length}\0`);
  return createHash('sha1').update(Buffer.concat([header, raw])).digest('hex');
}

function pinnedText(relative, blobSha1) {
  const raw = readBytes(relative);
  assert.equal(gitBlobSha1(raw), blobSha1, `${relative} is not the pinned git blob`);
  return raw.toString('utf8');
}

const encoderSource = pinnedText('fixtures/legacy_types_source_txEncoder.txt',
  '94201b4eaedcd240b9a9f6d54d11b55206615dff');
const decoderSource = pinnedText('fixtures/legacy_types_source_txDecoder.txt',
  '4728f1bdce1190428f91ae5b02f380584073c431');
const bignumberRaw = readBytes('fixtures/js_vendor/bignumber-9.1.2.js');
assert.equal(createHash('sha256').update(bignumberRaw).digest('hex'),
  '20db2d8d32fd388a927588beb9c1cdbb2c91ddf0273a180161334b28cb00902e',
  'vendored bignumber.js is not version 9.1.2');

const silent = { log() {}, warn() {}, error() {} };

function evaluate(source, imports) {
  const module = { exports: {} };
  vm.runInNewContext(source, {
    module,
    exports: module.exports,
    console: silent,
    require(name) {
      if (!Object.prototype.hasOwnProperty.call(imports, name)) {
        throw new Error(`disallowed require: ${name}`);
      }
      return imports[name];
    },
  }, { timeout: 2000 });
  return module.exports;
}

const BigNumber = evaluate(bignumberRaw.toString('utf8'), {});
const sourceImports = { 'bignumber.js': BigNumber, './base94.js': {}, './base256.js': {} };
const Encode = evaluate(encoderSource, sourceImports);
const Decode = evaluate(decoderSource, sourceImports);

const DECODERS = {
  send: (body) => Decode.decodeSend(body),
  commit: (body) => Decode.decodeCommitToken(body),
  attest: (body) => Decode.decodeIssueOrRevokeAttestation(body),
  withdraw: (body) => Decode.decodeWithdrawal(body),
};

const b36 = (n) => BigInt(n).toString(36);
const LONG = 'C'.repeat(43);
const MAX_SAFE = 9007199254740991n; // 2^53 - 1, JS Number safe integer

// Each row: family, id, JS encoder method, JS input, Rust outcome, note.
// Rust outcome grammar: payload:<text> | error:<PayloadError variant>
// | unrepresentable:<reason> | not-implemented
const ROWS = [
  // ---- simple send: legacy type 2, direct (non-send-all) layout ----
  row('send', 'send-whole-two', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 5, amount: 2 },
    `payload:tl20;alice;5;${b36(2)}`,
    'whole-token amount; Rust atoms 200000000 give the same base36 token count'),
  row('send', 'send-property-min', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: 1 },
    `payload:tl20;alice;1;${b36(1)}`,
    'property id 1 boundary'),
  row('send', 'send-property-max', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 4294967295, amount: 1 },
    `payload:tl20;alice;${b36(4294967295n)};${b36(1)}`,
    'property id 2^32-1 boundary'),
  row('send', 'send-zero-amount', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 5, amount: 0 },
    'payload:tl20;alice;5;0',
    'zero amount is wire-valid; economic validity is out of scope'),
  row('send', 'send-recipient-90', 'encodeSend',
    { sendAll: false, address: 'a'.repeat(90), propertyId: 5, amount: 2 },
    `payload:tl20;${'a'.repeat(90)};5;2`,
    'recipient at the 90-byte cap'),
  row('send', 'send-recipient-91', 'encodeSend',
    { sendAll: false, address: 'a'.repeat(91), propertyId: 5, amount: 2 },
    'error:FieldTooLong',
    'JS accepts any recipient length; Rust caps recipients at 90 bytes'),
  row('send', 'send-safe-max-whole', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: 90071992 },
    `payload:tl20;alice;1;${b36(90071992n)}`,
    'largest whole-token amount whose atoms (9007199200000000) stay within 2^53-1'),
  row('send', 'send-half-token', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 5, amount: 0.5 },
    `payload:tl20;alice;5;${b36(50000000n)}~`,
    'DIVERGENCE: JS omits ~ on fractional amounts, so its decoder reads 5e7 whole tokens; Rust emits the ~ atomic form'),
  row('send', 'send-one-atom', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 5, amount: '0.00000001' },
    `payload:tl20;alice;5;${b36(1n)}~`,
    'DIVERGENCE: one atomic unit; JS emits 1 (decoded as one whole token); Rust emits 1~'),
  row('send', 'send-safe-max-atoms', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 5, amount: '90071992.54740991' },
    `payload:tl20;alice;5;${b36(MAX_SAFE)}~`,
    'DIVERGENCE: largest exact atomic amount (2^53-1); JS drops ~ for fractional input'),
  row('send', 'send-u64max-atoms', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: '184467440737.09551615' },
    'error:AmountExceedsLegacyPrecision',
    'DIVERGENCE: u64::MAX atoms; JS emits exact base36 without ~, which its decoder reads through parseInt above 2^53'),
  row('send', 'send-float-text', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: '1.5' },
    'error:NonIntegerAmount',
    'DIVERGENCE: float text is rejected by parse_atomic_amount before any builder runs; JS emits a fraction without ~'),
  row('send', 'send-negative-number', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: -1 },
    'error:NegativeAmount',
    'DIVERGENCE: JS emits the sign verbatim'),
  row('send', 'send-u64max-decimal-text', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: '18446744073709551615' },
    'error:AmountExceedsLegacyPrecision',
    'DIVERGENCE: a string amount bypasses radix conversion and is emitted as decimal text'),
  row('send', 'send-overflow-text', 'encodeSend',
    { sendAll: false, address: 'alice', propertyId: 1, amount: '18446744073709551616' },
    'error:AmountOverflow',
    'DIVERGENCE: 2^64 emitted verbatim as decimal text'),
  row('send', 'send-empty-recipient', 'encodeSend',
    { sendAll: false, address: '', propertyId: 5, amount: 2 },
    'error:EmptyField',
    'DIVERGENCE: JS emits an empty recipient field'),
  row('send', 'send-semicolon-recipient', 'encodeSend',
    { sendAll: false, address: 'a;b', propertyId: 5, amount: 2 },
    'error:ForbiddenByte',
    'DIVERGENCE: the embedded ; splits the body into a five-field layout the JS decoder misreads'),

  // ---- commit to channel: legacy type 4 ----
  row('commit', 'commit-basic', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: 'chan', payEnabled: true, clearLists: 7, isColoredOutput: false },
    `payload:tl4${b36(7)},${b36(150000000n)},chan,1,7,0`,
    'amount 1.5 tokens = 150000000 atoms in base36'),
  row('commit', 'commit-property-max-clearlist-id-max', 'commitToChannel',
    { propertyId: 4294967295, amount: '0.00000001', channelAddress: 'chan', payEnabled: true, clearLists: 1, isColoredOutput: true, commitClearlistId: 4294967295 },
    `payload:tl4${b36(4294967295n)},1,chan,1,1,1,${b36(4294967295n)}`,
    'property id and optional clearlist id at 2^32-1'),
  row('commit', 'commit-zero-amount-no-clearlist', 'encodeCommit',
    { propertyId: 1, amount: 0, channelAddress: 'chan', payEnabled: false, isColoredOutput: false },
    'payload:tl41,0,chan,0,,0',
    'absent clearLists encodes as an empty field'),
  row('commit', 'commit-clearlist-id-zero', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: 'chan', payEnabled: false, clearLists: 7, isColoredOutput: false, commitClearlistId: 0 },
    `payload:tl47,${b36(150000000n)},chan,0,7,0,0`,
    'optional clearlist id 0 is emitted (JS Number.isInteger)'),
  row('commit', 'commit-safe-max-atoms', 'encodeCommit',
    { propertyId: 7, amount: '90071992.54740991', channelAddress: 'chan', payEnabled: false, clearLists: 7, isColoredOutput: false },
    `payload:tl47,${b36(MAX_SAFE)},chan,0,7,0`,
    'largest exact atomic amount (2^53-1); commit amounts carry no ~ in either implementation'),
  row('commit', 'commit-long-channel-reference', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: LONG, ref: 5, payEnabled: false, clearLists: 7, isColoredOutput: false },
    `payload:tl47,${b36(150000000n)},ref:5,0,7,0`,
    'channel longer than 42 bytes uses an explicit reference'),
  row('commit', 'commit-reference-zero', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: LONG, ref: 0, payEnabled: false, clearLists: 7, isColoredOutput: false },
    `payload:tl47,${b36(150000000n)},ref:0,0,7,0`,
    'explicit reference 0'),
  row('commit', 'commit-long-channel-no-reference', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: LONG, payEnabled: false, clearLists: 7, isColoredOutput: false },
    'error:ChannelAddressRequiresReference',
    'DIVERGENCE: JS substitutes ref:0 when ref is absent (ref || 0), silently redirecting the commit'),
  row('commit', 'commit-empty-channel', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: '', payEnabled: false, clearLists: 7, isColoredOutput: false },
    'error:EmptyField',
    'DIVERGENCE: JS emits an empty channel field'),
  row('commit', 'commit-clearlist-zero', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: 'chan', payEnabled: false, clearLists: 0, isColoredOutput: false },
    `payload:tl47,${b36(150000000n)},chan,0,0,0`,
    'DIVERGENCE: JS treats clearLists 0 as falsy and drops the field; Rust emits 0'),
  row('commit', 'commit-clearlist-array', 'encodeCommit',
    { propertyId: 7, amount: '1.5', channelAddress: 'chan', payEnabled: false, clearLists: [7, 11], isColoredOutput: false },
    'unrepresentable:array-clearlist-split-by-decoder',
    'DIVERGENCE: JS emits [7,11] which the decoder splits on comma before bracket parsing; the Rust type has no array form'),
  row('commit', 'commit-subatomic-text', 'encodeCommit',
    { propertyId: 7, amount: '0.000000001', channelAddress: 'chan', payEnabled: false, clearLists: 7, isColoredOutput: false },
    'unrepresentable:integer-atoms-api',
    'DIVERGENCE: JS emits a fractional base36 amount for a sub-atomic input; Rust accepts only integer atoms'),
  row('commit', 'commit-u64max-atoms', 'encodeCommit',
    { propertyId: 7, amount: '184467440737.09551615', channelAddress: 'chan', payEnabled: false, clearLists: 7, isColoredOutput: false },
    'error:AmountExceedsLegacyPrecision',
    'DIVERGENCE: u64::MAX atoms; the JS decoder converts through Number and loses precision'),
  row('commit', 'commit-framed-body-property', 'encodeCommit',
    { propertyId: 1065, amount: '1', channelAddress: 'chan', payEnabled: false, clearLists: 7, isColoredOutput: false },
    'error:AmbiguousFramedBody',
    'DIVERGENCE: property 1065 is base36 "tl", so the body begins tl; the crate decoder rejects framed bodies, JS decodes it'),

  // ---- attestation (issue or revoke): legacy type 9 ----
  row('attest', 'attest-revoke-zero', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'alice', metaData: 'OK' },
    `payload:tl90,${b36(7)},alice,OK`,
    'issue (revoke 0)'),
  row('attest', 'attest-revoke-one', 'encodeIssueOrRevokeAttestation',
    { revoke: 1, id: 7, targetAddress: 'alice', metaData: 'OK' },
    `payload:tl91,${b36(7)},alice,OK`,
    'revoke (numeric 1)'),
  row('attest', 'attest-id-max', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 4294967295, targetAddress: 'alice', metaData: 'OK' },
    `payload:tl90,${b36(4294967295n)},alice,OK`,
    'id 2^32-1 boundary'),
  row('attest', 'attest-empty-metadata', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'alice', metaData: '' },
    'payload:tl90,7,alice,',
    'empty metadata is allowed'),
  row('attest', 'attest-long-metadata', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'alice', metaData: 'META-'.repeat(60) },
    `payload:tl90,7,alice,${'META-'.repeat(60)}`,
    '300-byte uppercase metadata'),
  row('attest', 'attest-revoke-boolean-true', 'encodeIssueOrRevokeAttestation',
    { revoke: true, id: 7, targetAddress: 'alice', metaData: 'OK' },
    'payload:tl91,7,alice,OK',
    'DIVERGENCE: JS writes Boolean true as the word true, which the decoder reads as revoke=false (an issue, not a revoke)'),
  row('attest', 'attest-comma-metadata', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'alice', metaData: 'A,B' },
    'error:ForbiddenByte',
    'DIVERGENCE: the comma adds a field; the decoder silently keeps only A'),
  row('attest', 'attest-lowercase-metadata', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'alice', metaData: 'ok' },
    'error:MetadataNotUppercaseStable',
    'DIVERGENCE: the decoder upper-cases metadata, so lowercase would not round-trip'),
  row('attest', 'attest-empty-target', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: '', metaData: 'OK' },
    'error:EmptyField',
    'DIVERGENCE: JS emits an empty target field'),
  row('attest', 'attest-target-91', 'encodeIssueOrRevokeAttestation',
    { revoke: 0, id: 7, targetAddress: 'a'.repeat(91), metaData: 'OK' },
    'error:FieldTooLong',
    'DIVERGENCE: JS accepts any target length; Rust caps targets at 90 bytes'),

  // ---- withdrawal from channel: legacy type 21 (no Rust builder) ----
  row('withdraw', 'withdraw-all-true', 'encodeWithdrawal',
    { withdrawAll: true, propertyId: 7, amountOffered: 1, column: 1, channelAddress: 'chan', ref: 0 },
    'not-implemented',
    'NO BUILDER: JS writes the literal true, which decodeWithdrawal reads as withdrawAll=false'),
  row('withdraw', 'withdraw-one-token', 'encodeWithdrawal',
    { withdrawAll: false, propertyId: 7, amountOffered: 1, column: 0, channelAddress: 'chan', ref: 0 },
    'not-implemented',
    'NO BUILDER: JS writes decimal atoms (100000000) that decodeWithdrawal reads as base36 (about 28211 tokens, not 1)'),
  row('withdraw', 'withdraw-u64max-text', 'encodeWithdrawal',
    { withdrawAll: false, propertyId: 7, amountOffered: '184467440737.09551615', column: 1, channelAddress: 'chan', ref: 0 },
    'not-implemented',
    'NO BUILDER: JS rounds through Number before emitting decimal text'),
];

function row(family, id, jsMethod, jsInput, rustOutcome, note) {
  return { family, id, jsMethod, jsInput, rustOutcome, note };
}

function encodeWithJs(method, input) {
  const fn = method === 'commitToChannel' ? Encode.encodeCommit : Encode[method];
  try {
    return fn(input);
  } catch (error) {
    return `THROW:${error.message}`;
  }
}

function decodeWithJs(family, payload) {
  if (payload.startsWith('THROW:')) return 'NOT_DECODED';
  try {
    const value = DECODERS[family](payload.slice(3));
    return JSON.stringify(value ?? null);
  } catch (error) {
    return `THROW:${error.message}`;
  }
}

const HEADER = [
  '# Payload-builder JS parity. Frozen src/txEncoder.js and src/txDecoder.js (tradelayer.js dca0888, git blobs 94201b4e and 4728f1bd) run in a vm with vendored bignumber.js 9.1.2 (sha256 20db2d8d).',
  '# rust_outcome is the result the Rust builders must produce (tests/payload_builders.rs). parity is match only when it is byte-identical to js_payload.',
  ['id', 'family', 'js_method', 'js_input', 'js_payload', 'js_decoded', 'rust_outcome', 'parity', 'note'].join('\t'),
];

function generate() {
  const lines = [...HEADER];
  for (const entry of ROWS) {
    const jsPayload = encodeWithJs(entry.jsMethod, entry.jsInput);
    const jsDecoded = decodeWithJs(entry.family, jsPayload);
    const matches = entry.rustOutcome.startsWith('payload:')
      && entry.rustOutcome.slice('payload:'.length) === jsPayload;
    const parity = matches ? 'match' : 'divergent';
    if (parity === 'divergent') {
      assert.ok(entry.note.length > 0, `${entry.id}: a divergent row must be documented in its note`);
    }
    const cells = [
      entry.id,
      entry.family,
      entry.jsMethod,
      JSON.stringify(entry.jsInput),
      jsPayload,
      jsDecoded,
      entry.rustOutcome,
      parity,
      entry.note,
    ];
    for (const cell of cells) {
      assert.ok(!/[\t\n\r]/.test(cell), `${entry.id}: TSV cell contains a delimiter`);
    }
    lines.push(cells.join('\t'));
  }
  const ids = new Set(ROWS.map((entry) => entry.id));
  assert.equal(ids.size, ROWS.length, 'row ids must be unique');
  return `${lines.join('\n')}\n`;
}

const text = generate();

if (process.argv.includes('--check')) {
  const committed = readFileSync(join(ROOT, FIXTURE), 'utf8');
  if (committed !== text) {
    console.error(`${FIXTURE} is stale; regenerate with: node scripts/generate_payload_builder_vectors.mjs > ${FIXTURE}`);
    process.exit(1);
  }
  const rows = ROWS.length;
  const matched = text.split('\n').filter((line) => line.includes('\tmatch\t')).length;
  console.log(`payload builder vectors ok: ${rows} rows, ${matched} byte-identical to JS, ${rows - matched} documented divergences`);
} else {
  process.stdout.write(text);
}
