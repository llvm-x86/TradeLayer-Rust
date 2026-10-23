'use strict';
// Stage 23B legacy JS replay harness tests.
//
// The default section uses only Node builtins and synthetic records (CI).
// Set TL_LEGACY_JS=/path/to/tradelayer.js (pinned dca0888b, dependencies
// installed with `npm ci --ignore-scripts`) to also run the synthetic
// end-to-end replay inside `unshare -rn` against the actual legacy modules.
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const H = require('../scripts/legacy_js_replay.cjs');

const ROOT = path.resolve(__dirname, '..');
let passed = 0;
async function test(name, fn) {
  await fn();
  passed += 1;
  console.log(`ok - ${name}`);
}

function output(vout, sats, type, legacy, modern) {
  return { vout, satoshis: sats, script_hex: type === 'nulldata' ? '6a02746c' : '0014' + '11'.repeat(20),
    script_type: type, address: { legacy_addresses: legacy, modern_address: modern,
      legacy_js_selected_address: legacy ? legacy[0] : null } };
}

function manifest(candidates) {
  return { network: 'litecoin-testnet', start_height: 10, end_height: 10, pinned_litecoin_genesis: 'g',
    transactions: candidates.map((c, i) => ({ txid: c.txid, height: 10, transaction_position: i + 1,
      blockhash: 'bb', carrier_class: 'strict-and-legacy-js', source_trace: {
        all_prevouts: [{ txid: c.parent, vout: 0 }],
        decoded_outputs: [output(0, 0, 'nulldata', null, null), output(1, 546, 'witness_v0_keyhash', ['tltc1x'], null)],
        sender_prevout: { ...output(0, c.parentSats ?? 1000, 'witness_v0_keyhash', ['tltc1s'], null), txid: c.parent, output_count: 2 },
        indexed_reference_outputs_0_1: [], last_non_nulldata_reference: null } })) };
}

(async () => {
  await test('JSON number emulation equals JSON.parse of Core 8-decimal values', () => {
    for (const sats of [0n, 1n, 546n, 100002n, 2900000000n, 8400000000000000n, 123456789012345n]) {
      const whole = sats / 100000000n;
      const frac = (sats % 100000000n).toString().padStart(8, '0');
      assert.equal(H.satsToJsonNumber(sats), JSON.parse(`${whole}.${frac}`));
    }
    assert.notEqual(H.satsToJsonNumber(100002n) * 1e8, 100002); // float product is inexact
    assert.equal(Math.round(H.satsToJsonNumber(100002n) * 1e8), 100002);
  });

  await test('isolation check refuses any non-loopback interface', () => {
    H.assertIsolated({ lo: [] });
    assert.throws(() => H.assertIsolated({ lo: [], eth0: [] }), /unshare -rn/);
  });

  await test('decoded outputs keep the recorded RPC address schema, never synthesize', () => {
    const legacy = H.decodedOutput(output(1, 546, 'witness_v0_keyhash', ['tltc1a'], null));
    assert.deepEqual(legacy.scriptPubKey.addresses, ['tltc1a']);
    assert.equal('address' in legacy.scriptPubKey, false);
    const modern = H.decodedOutput(output(1, 546, 'witness_v0_keyhash', null, 'tltc1b'));
    assert.equal('addresses' in modern.scriptPubKey, false);
    assert.equal(modern.scriptPubKey.address, 'tltc1b');
    const p2pk = H.decodedOutput(output(1, 546, 'pubkey', null, null));
    assert.deepEqual(Object.keys(p2pk.scriptPubKey).sort(), ['hex', 'type']);
  });

  await test('recorded client answers only from evidence and records misses with arguments', async () => {
    const ev = H.loadEvidence(manifest([{ txid: 'aa', parent: 'pp' }]), 'x\tl\t10\tbb\taa\tdeadbeef\n', null);
    const client = H.RecordedClient.proxy(new H.RecordedClient(ev, true,
      { validateaddress: { tltc1ok: { isvalid: true } } }));
    assert.equal(await client.getRawTransaction('aa', false), 'deadbeef');
    const parent = await client.getRawTransaction('pp');
    assert.equal(parent.vout.length, 2);
    assert.deepEqual(parent.vout[0].scriptPubKey.addresses, ['tltc1s']);
    assert.equal(parent.vout[1], null);
    assert.deepEqual(await client.validateAddress('tltc1ok'), { isvalid: true });
    await assert.rejects(client.validateAddress('tltc1unknown'), /unrecorded RPC validateAddress/);
    await assert.rejects(client.getRawTransaction('pp', false), /unrecorded/);
    await assert.rejects(client.dumpprivkey('x'), /unrecorded RPC dumpprivkey/);
    await assert.rejects(client.getRawTransaction('zz'), /unrecorded/);
    assert.equal(client.then, undefined); // awaiting the proxy must not call a fake RPC
    assert.deepEqual(client.misses, ['validateAddress ["tltc1unknown"]', 'getrawtransaction pp',
      'dumpprivkey ["x"]', 'getrawtransaction zz']);
  });

  await test('conflicting records for one parent output are refused', () => {
    const m = manifest([{ txid: 'aa', parent: 'pp' }, { txid: 'ab', parent: 'pp', parentSats: 999 }]);
    assert.throws(() => H.loadEvidence(m, 'x\tl\t10\tbb\taa\t00\nx\tl\t10\tbb\tab\t01\n', null), /conflicting/);
  });

  await test('syntax classes compare the JS payload with Rust type and body', () => {
    const js = { marker: 'tl', payload: '2abc' };
    assert.equal(H.classifySyntax(js, [{ vout: 0, type: 2, body: 'abc' }]), 'MATCH');
    assert.equal(H.classifySyntax(js, [{ vout: 0, type: 2, body: 'abd' }]), 'MISMATCH');
    assert.equal(H.classifySyntax(js, []), 'JS_ONLY');
    assert.equal(H.classifySyntax(null, [{ vout: 0, type: 2, body: 'abc' }]), 'RUST_ONLY');
    assert.equal(H.classifySyntax(null, []), 'NEITHER');
    const rows = H.rustCarriers('# h\naa\t0\t2\t616263\tsend\t1\t2\t3\t-\n');
    assert.deepEqual(rows.get('aa'), [{ vout: 0, type: 2, body: 'abc' }]);
  });

  await test('decision oracle: incomplete prior state, misses, taint and precision are inconclusive', () => {
    const ev = { priorState: 'COMPLETE_FROM_LEGACY_JS_GENESIS', candidates: [
      { txid: 'a', evidence: 'ACCEPTED_FOR_PARITY' }, { txid: 'b', evidence: 'ACCEPTED_FOR_PARITY' },
      { txid: 'c', evidence: 'ACCEPTED_FOR_PARITY' }, { txid: 'd', evidence: 'INCONCLUSIVE' },
      { txid: 'e', evidence: 'ACCEPTED_FOR_PARITY' }] };
    const d = (txid, type, valid, extra = {}) => ({ txid, type, valid, misses: [], numeric_fields: {}, ...extra });
    const out = H.classifyDecisions([
      d('a', 2, true, { numeric_fields: { amounts: { js: '0.1', precision: 'exact-atoms' } } }),
      d('e', 1, true, { numeric_fields: { initialAmount: { js: '1e300', precision: 'PRECISION_SUSPECT' } } }),
      d('b', 2, false, { misses: ['validateAddress ["x"]'] }),
      d('c', 5, true), d('d', 2, true)], ev);
    assert.deepEqual(out.map((x) => x.oracle), ['DECIDED', 'INCONCLUSIVE', 'INCONCLUSIVE',
      'UNSUPPORTED_BY_ORACLE', 'INCONCLUSIVE']);
    assert.match(out[1].inconclusive_reasons.join(), /not exactly representable/);
    assert.match(out[2].inconclusive_reasons.join(), /unrecorded RPC/);
    assert.match(out[4].inconclusive_reasons.join(), /evidence INCONCLUSIVE.*tainted by valid unsupported type 5/);
    const incomplete = H.classifyDecisions([d('a', 0, true)], { ...ev, priorState: 'INCOMPLETE' });
    assert.deepEqual(incomplete[0].inconclusive_reasons, ['prior state incomplete']);
  });

  await test('numeric fields flag amounts that are not safe integers in atoms', () => {
    const f = H.numericFields({ amounts: 10, initialAmount: 1e9, price: '3', other: 1.5, amountX: 9e15 });
    assert.equal(f.amounts.precision, 'exact-atoms');
    assert.equal(f.initialAmount.precision, 'PRECISION_SUSPECT');
    assert.equal(f.price.precision, 'non-number');
    assert.equal(f.amountX.precision, 'PRECISION_SUSPECT');
    assert.equal('other' in f, false);
  });

  await test('intra-block order divergence compares legacy txid order with block position', () => {
    const ev = { candidates: [{ txid: 'f0', height: 9, transaction_position: 1 },
      { txid: '0a', height: 9, transaction_position: 2 }, { txid: 'zz', height: 8, transaction_position: 1 }] };
    const out = H.intraBlockOrderSensitive([{ height: 9, order: ['0a', 'f0'] }, { height: 8, order: ['zz'] }], ev);
    assert.deepEqual(out, [{ height: 9, legacy_order: ['0a', 'f0'], canonical_order: ['f0', '0a'], differs: true }]);
  });

  await test('recordings must match network and genesis', () => {
    const m = { network: 'litecoin-testnet', pinned_litecoin_genesis: 'g' };
    assert.equal(H.checkRecordings(null, m), null);
    const ok = { format: 'litecoin-rpc-recordings-v1', network: 'litecoin-testnet', pinned_litecoin_genesis: 'g' };
    assert.equal(H.checkRecordings(ok, m), ok);
    assert.throws(() => H.checkRecordings({ ...ok, network: 'litecoin-mainnet' }, m), /different network/);
    assert.throws(() => H.checkRecordings({ ...ok, pinned_litecoin_genesis: 'h' }, m), /different network/);
  });

  await test('pins match the documented legacy commit and genesis heights', () => {
    assert.equal(H.PINNED_COMMIT, 'dca0888b827c5bc28fa9189bc9e1d507f5c5670c');
    assert.deepEqual(H.LEGACY_GENESIS, { 'litecoin-testnet': 3082500, 'litecoin-mainnet': 2819000 });
  });

  if (process.env.TL_LEGACY_JS) {
    await test('OPT-IN synthetic end-to-end replay through the actual pinned modules', () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tl-legacy-e2e-'));
      const run = (cmd, args) => execFileSync(cmd, args, { cwd: ROOT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
      run('python3', ['tests/synthetic_ltc_chain.py', dir]);
      fs.writeFileSync(path.join(dir, 'rust-fields.tsv'),
        run('cargo', ['run', '-q', '--locked', '--bin', 'tradelayer-legacy-fields', '--', path.join(dir, 'corpus.tsv')]));
      const replay = (out, extra = []) => {
        run('unshare', ['-rn', 'node', 'scripts/legacy_js_replay.cjs', '--js', process.env.TL_LEGACY_JS,
          '--manifest', path.join(dir, 'manifest.json'), '--corpus', path.join(dir, 'corpus.tsv'),
          '--verification', path.join(dir, 'verification.json'), '--rust-fields', path.join(dir, 'rust-fields.tsv'),
          '--output', path.join(dir, out), ...extra]);
        return JSON.parse(fs.readFileSync(path.join(dir, out), 'utf8'));
      };
      const first = replay('replay1.json');
      assert.deepEqual(first.syntax.counts, { MATCH: 3 });
      assert.deepEqual(first.actor_reference.counts, { MATCH: 3 });
      assert.deepEqual(first.rpc_requests.map((r) => r.split(' ')[0]), ['validateAddress']);
      run('python3', ['tests/synthetic_ltc_chain.py', dir, path.join(dir, 'replay1.json')]);
      const rec = ['--rpc-recordings', path.join(dir, 'recordings.json')];
      const legacy = replay('replay2.json', rec);
      const counter = replay('replay3.json', [...rec, '--ordering', 'canonical-counterfactual']);
      const byType = (r) => Object.fromEntries(r.decisions.records.map((d) => [d.type, d.valid]));
      assert.deepEqual(legacy.decisions.counts, { DECIDED: 3 });
      assert.deepEqual(byType(legacy), { 0: true, 1: true, 2: false });   // send processed before issue
      assert.deepEqual(byType(counter), { 0: true, 1: true, 2: true });   // block-position order
      assert.notEqual(legacy.blocks[0].state_digest, counter.blocks[0].state_digest);
      assert.equal(legacy.intra_block_order_divergence.length, 1);
      assert.equal(legacy.legacy_js.commit, H.PINNED_COMMIT);
    });
  }
  console.log(`${passed} legacy replay harness tests passed`);
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
