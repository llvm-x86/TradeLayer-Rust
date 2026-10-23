#!/usr/bin/env node
'use strict';
// Stage 23B: replay node-backed Litecoin candidates through the PINNED legacy
// tradelayer.js modules (txIndex, txUtils, types/validity, logic), offline.
//
// Three separate reports are produced and never merged:
//   syntax            txIndex.DecodeRawTransaction vs Rust's strict reader
//   actor_reference   txUtils.getSender / getTransactionOutputs (vout 0/1) /
//                     getReference (last non-nulldata) vs the source trace
//   decisions         Types.decodePayload + Logic.typeSwitch, replayed with the
//                     legacy catch-up ordering of main.js processTxSet
//
// The RPC client is replaced by a recorder that answers ONLY from the
// verified manifest. Any other RPC (validateaddress, wallet calls, unknown
// transactions) is a "miss": it throws inside the legacy code exactly as an
// unreachable node would, and the affected decision becomes INCONCLUSIVE.
//
// SECURITY: requiring the legacy modules executes their top-level code. Run
// only inside a network namespace with no interfaces but loopback, e.g.
//   unshare -rn node scripts/legacy_js_replay.cjs ...
// The script refuses to start otherwise, and refuses a checkout that is not
// exactly the pinned commit with a clean worktree.
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const Module = require('node:module');
const os = require('node:os');
const path = require('node:path');

const PINNED_COMMIT = 'dca0888b827c5bc28fa9189bc9e1d507f5c5670c';
// processTxSet/constructConsensusFromIndex below are transliterated from this
// exact src/main.js; any byte change invalidates the transliteration.
const PINNED_MAIN_JS_SHA256 = '006d59ba3d28d802b0e0c5be2d9d661b58caf1c919a704b3f0d40bc9613c4009';
const ORACLE_TYPES = new Set([0, 1, 2, 11, 12]);
// main.js GENESIS_BLOCK_HEIGHTS.LTC at the pinned commit; the legacy indexer
// never reads blocks below these heights.
const LEGACY_GENESIS = { 'litecoin-testnet': 3082500, 'litecoin-mainnet': 2819000 };
const FUNDING_TYPES = new Set([4, 20]);
const STATE_CATEGORIES = ['activations', 'propertyList', 'tallyMap', 'orderBooks', 'clearlists',
  'attestations', 'vaults', 'syntheticTokens', 'channels', 'withdrawQueue', 'marginMaps'];

const sha256 = (data) => crypto.createHash('sha256').update(data).digest('hex');

// Core prints values with 8 decimals; JSON.parse yields the nearest double.
function satsToJsonNumber(sats) {
  const s = BigInt(sats);
  const whole = s / 100000000n;
  const frac = (s % 100000000n).toString().padStart(8, '0');
  return Number(`${whole}.${frac}`);
}

function assertIsolated(interfaces = os.networkInterfaces()) {
  const external = Object.keys(interfaces).filter((name) => name !== 'lo');
  if (external.length) {
    throw Error(`refusing to load legacy JS with network interfaces present: ${external.join(',')}; run under "unshare -rn"`);
  }
}

function assertPinned(jsRoot) {
  const git = (...args) => execFileSync('git', ['-C', jsRoot, ...args], { encoding: 'utf8' }).trim();
  const commit = git('rev-parse', 'HEAD');
  if (commit !== PINNED_COMMIT) throw Error(`legacy JS checkout is ${commit}, expected ${PINNED_COMMIT}`);
  if (git('status', '--porcelain') !== '') throw Error('legacy JS worktree is not clean');
  const mainSha = sha256(fs.readFileSync(path.join(jsRoot, 'src', 'main.js')));
  if (mainSha !== PINNED_MAIN_JS_SHA256) throw Error(`src/main.js digest ${mainSha} differs from pin`);
  const lockfile = sha256(fs.readFileSync(path.join(jsRoot, 'package-lock.json')));
  const installed = path.join(jsRoot, 'node_modules', '.package-lock.json');
  return {
    commit, main_js_sha256: mainSha, package_lock_sha256: lockfile,
    installed_lock_sha256: fs.existsSync(installed) ? sha256(fs.readFileSync(installed)) : null,
    node: process.version,
  };
}

function decodedOutput(o) {
  const spk = { hex: o.script_hex, type: o.script_type };
  // Keep the node's schema exactly as recorded: Litecoin Core 0.21 emits
  // addresses[]; newer RPC schemas emit address. Neither is synthesized.
  if (o.address.legacy_addresses != null) spk.addresses = o.address.legacy_addresses;
  if (o.address.modern_address != null) spk.address = o.address.modern_address;
  return { n: o.vout, value: satsToJsonNumber(o.satoshis), scriptPubKey: spk };
}

function loadEvidence(manifest, corpusText, verification) {
  const raw = new Map();
  for (const line of corpusText.split(/\r?\n/)) {
    if (!line || line.startsWith('#')) continue;
    const f = line.split('\t');
    if (f.length !== 6) throw Error('corpus row must have 6 fields');
    raw.set(f[4], f[5]);
  }
  const statuses = new Map();
  let priorState = 'INCOMPLETE';
  if (verification) {
    priorState = verification.coverage.prior_state;
    for (const m of verification.manifests) for (const c of m.candidates) statuses.set(c.txid, c.parity_input);
  }
  const candidates = [];
  const decoded = new Map();
  const parents = new Map();
  for (const c of manifest.transactions) {
    const st = c.source_trace;
    if (!st || !raw.has(c.txid)) throw Error(`candidate ${c.txid} lacks source trace or raw bytes`);
    decoded.set(c.txid, {
      txid: c.txid, hex: raw.get(c.txid),
      vin: st.all_prevouts.map((p) => ({ txid: p.txid, vout: p.vout })),
      vout: st.decoded_outputs.map(decodedOutput),
    });
    const s = st.sender_prevout;
    const vout = new Array(s.output_count).fill(null);
    vout[s.vout] = decodedOutput(s);
    const prior = parents.get(s.txid);
    if (prior && JSON.stringify(prior.vout[s.vout] ?? decodedOutput(s)) !== JSON.stringify(decodedOutput(s))) {
      throw Error(`conflicting records for parent ${s.txid}`);
    }
    if (prior) prior.vout[s.vout] = decodedOutput(s);
    else parents.set(s.txid, { txid: s.txid, vout });
    candidates.push({ ...c, raw: raw.get(c.txid), evidence: statuses.get(c.txid) ?? 'UNVERIFIED' });
  }
  return { network: manifest.network, start: manifest.start_height, end: manifest.end_height,
    candidates, decoded, parents, priorState };
}

class RecordedClient {
  constructor(evidence, testnet, recordings = null) {
    this.evidence = evidence;
    this.testnet = testnet;
    this.chain = 'LTC';
    this.misses = [];
    // Node answers recorded by scripts/litecoin_record_rpc.py; only
    // validateaddress is accepted, keyed by the exact address string.
    this.recorded = new Map(Object.entries((recordings && recordings.validateaddress) || {}));
    this.byHex = new Map([...evidence.decoded.values()].map((d) => [d.hex, d]));
  }
  miss(method, detail) {
    this.misses.push(`${method}${detail ? ' ' + detail : ''}`);
    throw Error(`harness: unrecorded RPC ${method}`);
  }
  async answer(method, args) {
    if (method === 'validateAddress' && args.length === 1 && this.recorded.has(String(args[0]))) {
      return structuredClone(this.recorded.get(String(args[0])));
    }
    return this.miss(method, JSON.stringify(args));
  }
  async getChain() { return 'LTC'; }
  async getTest() { return this.testnet; }
  async getRawTransaction(txid, verbose = true) {
    const d = this.evidence.decoded.get(txid);
    if (d) return verbose ? structuredClone(d) : d.hex;
    const p = this.evidence.parents.get(txid);
    if (p && verbose) return structuredClone(p);
    return this.miss('getrawtransaction', txid);
  }
  async decoderawtransaction(hex) {
    const d = this.byHex.get(hex);
    return d ? structuredClone(d) : this.miss('decoderawtransaction');
  }
  static proxy(target) {
    return new Proxy(target, {
      get(t, prop) {
        if (prop in t || typeof prop === 'symbol' || prop === 'then' || prop === 'toJSON') return t[prop];
        return async (...args) => t.answer(String(prop), args);
      },
    });
  }
}

function withQuietConsole(fn) {
  const saved = { log: console.log, error: console.error, warn: console.warn, info: console.info };
  console.log = console.error = console.warn = console.info = () => {};
  return Promise.resolve().then(fn).finally(() => Object.assign(console, saved));
}

function loadLegacy(jsRoot, client, nedbRoot, network) {
  process.env.TL_SKIP_RPC_BOOT = '1';
  process.env.TL_NEDB_ROOT = nedbRoot;
  process.env.CHAIN = 'LTC';
  process.env.TL_FORCE_TEST = network === 'litecoin-mainnet' ? '0' : '1';
  const clientPath = path.join(jsRoot, 'src', 'client.js');
  const fake = { getInstance: async () => client };
  const original = Module._load;
  Module._load = function load(request, parent, isMain) {
    if (Module._resolveFilename(request, parent, isMain) === clientPath) return fake;
    return original.apply(this, arguments);
  };
  const req = (name) => require(path.join(jsRoot, 'src', name));
  return {
    TxIndex: req('txIndex.js'), TxUtils: req('txUtils.js'), Types: req('types.js'),
    Logic: req('logic.js'), Consensus: req('consensus.js'), Activation: req('activation.js'),
    TallyMap: req('tally.js'), Orderbook: req('orderbook.js'), VolumeIndex: req('volumeIndex.js'),
    TradeLayerManager: req('vesting.js'), Channels: req('channels.js'), Clearing: req('clearing.js'),
    db: req('db.js'),
  };
}

function rustCarriers(rustFieldsText) {
  const map = new Map();
  for (const line of (rustFieldsText || '').split(/\r?\n/)) {
    if (!line || line.startsWith('#')) continue;
    const [txid, vout, type, bodyHex] = line.split('\t');
    if (!map.has(txid)) map.set(txid, []);
    map.get(txid).push({ vout: Number(vout), type: Number(type), body: Buffer.from(bodyHex, 'hex').toString('latin1') });
  }
  return map;
}

function classifySyntax(js, rust) {
  if (!js && !rust.length) return 'NEITHER';
  if (js && !rust.length) return 'JS_ONLY';
  if (!js) return 'RUST_ONLY';
  const jsType = parseInt(js.payload.slice(0, 1), 36);
  const match = rust.some((r) => r.type === jsType && r.body === js.payload.slice(1));
  return match ? 'MATCH' : 'MISMATCH';
}

async function syntaxReport(L, ev, rust) {
  const records = [];
  for (const c of ev.candidates) {
    const result = await withQuietConsole(() => L.TxIndex.DecodeRawTransaction(c.raw));
    const js = result ? { marker: result.marker, payload: result.payload } : null;
    const r = rust.get(c.txid) || [];
    records.push({ txid: c.txid, height: c.height, carrier_class: c.carrier_class, verdict: classifySyntax(js, r),
      js, js_type: js ? parseInt(js.payload.slice(0, 1), 36) : null, rust: r });
  }
  return summarize(records);
}

function errorText(value) { return value instanceof Error ? `Error: ${value.message}` : null; }

async function actorReport(L, ev, client) {
  const records = [];
  await L.TxUtils.init();
  for (const c of ev.candidates) {
    const st = c.source_trace;
    const before = client.misses.length;
    const [sender, outputs, reference] = await withQuietConsole(async () => [
      await L.TxUtils.getSender(c.txid), await L.TxUtils.getTransactionOutputs(c.txid),
      await L.TxUtils.getReference(c.txid)]);
    const expectedSender = st.sender_prevout.address.legacy_js_selected_address;
    const expectedRefs = st.indexed_reference_outputs_0_1.map((o) => ({
      address: o.address.legacy_js_selected_address, satoshis: o.satoshis }));
    const last = st.last_non_nulldata_reference;
    const fields = {};
    fields.sender = errorText(sender) ? (expectedSender === null ? 'match-js-error' : 'MISMATCH')
      : (sender.senderAddress === expectedSender ? 'match' : 'MISMATCH');
    // getSender returns the node's float LTC value. Rounding to atoms must
    // recover the satoshis; an inexact product is a precision note only.
    fields.sender_amount = errorText(sender) ? 'n/a'
      : (Math.round(sender.amount * 1e8) !== st.sender_prevout.satoshis ? 'MISMATCH'
        : (sender.amount * 1e8 === st.sender_prevout.satoshis ? 'exact' : 'float-needs-rounding'));
    const refs = (outputs || []).filter((o) => o.vout === 0 || o.vout === 1)
      .map((o) => ({ address: o.address, satoshis: o.satoshis }));
    fields.reference_vout_0_1 = JSON.stringify(refs) === JSON.stringify(expectedRefs) ? 'match' : 'MISMATCH';
    if (errorText(reference)) {
      fields.reference_last_non_nulldata = (!last || last.address.legacy_js_selected_address === null)
        ? 'match-js-error' : 'MISMATCH';
    } else {
      fields.reference_last_non_nulldata = last && reference.address === last.address.legacy_js_selected_address
        && reference.satoshis === last.satoshis ? 'match' : 'MISMATCH';
    }
    const misses = client.misses.slice(before);
    const mismatch = Object.values(fields).some((v) => v === 'MISMATCH');
    records.push({ txid: c.txid, height: c.height,
      verdict: misses.length ? 'INCONCLUSIVE' : (mismatch ? 'MISMATCH' : 'MATCH'), fields, misses,
      js: { sender: errorText(sender) ?? sender, reference_vout_0_1: refs, reference_last: errorText(reference) ?? reference } });
  }
  return summarize(records);
}

async function stateDigest(db) {
  const state = {};
  for (const name of STATE_CATEGORIES) {
    const store = await db.getDatabase(name);
    const docs = store ? await store.findAsync({}) : [];
    state[name] = docs.map((d) => JSON.stringify(d)).sort();
  }
  return { digest: sha256(JSON.stringify(state)), tally: state.tallyMap };
}

// main.js constructConsensusFromIndex + processTxSet at the pinned commit,
// minus logging. Post-block hooks run when a block had unprocessed txs or
// at heights divisible by 10000, as in the original.
async function legacyConsensus(L, ev, client, start, end, ordering = 'legacy') {
  const txIndexDB = await L.db.getDatabase('txIndex');
  const all = (await txIndexDB.findAsync({})).filter((d) => d._id.startsWith('tx-'));
  const byHeight = {};
  for (const d of all) {
    const h = parseInt(d._id.split('-')[1], 10);
    byHeight[h] ??= { fundingTx: [], tradeTx: [] };
    if (!d.value || !d.value.payload) continue;
    const type = parseInt(d.value.payload.slice(0, 1).toString(36), 36);
    (FUNDING_TYPES.has(type) ? byHeight[h].fundingTx : byHeight[h].tradeTx).push(d);
  }
  if (ordering === 'canonical-counterfactual') {
    // NOT legacy behaviour: one bucket in canonical block position order.
    const position = new Map(ev.candidates.map((c) => [c.txid, c.transaction_position]));
    for (const block of Object.values(byHeight)) {
      block.tradeTx = [...block.fundingTx, ...block.tradeTx]
        .sort((a, b) => position.get(a.value.txId) - position.get(b.value.txId));
      block.fundingTx = [];
    }
  }
  const activation = L.Activation.getInstance();
  const decisions = [];
  const blocks = [];
  for (let height = start; height <= end; height++) {
    const blockData = byHeight[height];
    let skip = true;
    const blockMissStart = client.misses.length;
    let processed = 0;
    if (blockData) {
      for (const set of [blockData.fundingTx, blockData.tradeTx]) {
        for (const d of set) {
          processed += 1;
          decisions.push(await processOne(L, d, height, client, activation));
        }
      }
      if (processed) skip = false;
    }
    if (height % 10000 === 0) skip = false;
    let hookError = null;
    if (!skip) {
      try {
        await withQuietConsole(async () => {
          await L.TallyMap.flushQueuedFeeAccruals(height);
          await L.Logic.settleLiquidityRewards(height);
          await L.Orderbook.processQueuedOnChainOrdersForBlock(height);
          await L.Orderbook.processQueuedChannelTradesForBlock(height);
          const cumulative = await L.VolumeIndex.getCumulativeVolumes(height);
          const thisBlock = await L.VolumeIndex.getBlockVolumes(height);
          if (thisBlock.global > 0) {
            const v = await L.TradeLayerManager.updateVesting(cumulative.ltcPairTotalVolume,
              thisBlock.ltcPairs, cumulative.globalCumulativeVolume, thisBlock.global);
            if (v != null && thisBlock != 0) {
              await L.TallyMap.applyVesting(2, v.two, height);
              await L.TallyMap.applyVesting(3, v.three, height);
            }
          }
          await L.Channels.processWithdrawals(height);
          await L.Clearing.clearingFunction(height, false);
        });
      } catch (err) {
        hookError = String(err && err.message);
      }
    }
    if (!skip) {
      const snap = await stateDigest(L.db);
      blocks.push({ height, order: blockData ? [...blockData.fundingTx, ...blockData.tradeTx].map((d) => d.value.txId) : [],
        post_block_hooks: hookError ? `ERROR ${hookError}` : 'ran', hook_misses: client.misses.slice(blockMissStart),
        state_digest: snap.digest, tally_docs: snap.tally });
    }
  }
  return { decisions, blocks };
}

async function processOne(L, txData, blockHeight, client, activation) {
  const before = client.misses.length;
  const valueData = txData.value;
  const txId = valueData.txId;
  let type = -1;
  let decodedParams;
  let fatal = null;
  await withQuietConsole(async () => {
    try {
      if (await L.Consensus.checkIfTxProcessed(txId)) { decodedParams = { skipped: true }; return; }
      let payload = valueData.payload;
      const marker = valueData.marker;
      type = parseInt(payload.slice(0, 1).toString(36), 36);
      payload = payload.slice(1, payload.length).toString(36);
      const senderAddress = valueData.sender.senderAddress;
      const referenceAddress = valueData.reference;
      const senderUTXO = valueData.sender.amount;
      const referenceUTXO = valueData.reference.amount / 100000000;
      decodedParams = await L.Types.decodePayload(txId, type, marker, payload, senderAddress,
        referenceAddress, senderUTXO, referenceUTXO, blockHeight);
      decodedParams.block = blockHeight;
      if (decodedParams.valid === true) {
        try {
          await L.Logic.typeSwitch(type, decodedParams);
        } catch (logicErr) {
          decodedParams.valid = false;
          decodedParams.reason = `${decodedParams.reason || ''} Logic error: ${logicErr.message}`;
        }
      }
      await L.Consensus.markTxAsProcessed(txId, decodedParams);
      await L.TxIndex.upsertTxValidityAndReason(txId, type, decodedParams.valid, decodedParams.reason);
    } catch (txErr) {
      fatal = String(txErr && txErr.message);
      decodedParams = { valid: false, reason: `Fatal tx processing error: ${fatal}`, block: blockHeight, txid: txId };
      if (txId !== 'unknown') {
        await L.Consensus.markTxAsProcessed(txId, decodedParams);
        if (type >= 0) await L.TxIndex.upsertTxValidityAndReason(txId, type, false, decodedParams.reason);
      }
    }
  });
  const activationBlock = type > 0 ? activation.getActivationBlock(type) : null;
  return { txid: txId, height: blockHeight, type, valid: decodedParams.valid === true,
    reason: decodedParams.reason ?? null, fatal, misses: client.misses.slice(before),
    // processTx (real-time mode) additionally rejects blockHeight < activation block.
    realtime_mode_differs: decodedParams.valid === true && activationBlock != null && blockHeight < activationBlock,
    sender_was_error_object: valueData.sender && valueData.sender.senderAddress === undefined,
    numeric_fields: numericFields(decodedParams) };
}

function numericFields(params) {
  const out = {};
  for (const [key, value] of Object.entries(params || {})) {
    if (!/amount|quantity|initial|price/i.test(key)) continue;
    if (typeof value === 'number') {
      const atoms = value * 1e8;
      out[key] = { js: String(value), precision: Number.isSafeInteger(atoms) ? 'exact-atoms' : 'PRECISION_SUSPECT' };
    } else if (value != null) {
      out[key] = { js: String(value), precision: 'non-number' };
    }
  }
  return out;
}

function classifyDecisions(raw, ev) {
  const evidence = new Map(ev.candidates.map((c) => [c.txid, c.evidence]));
  let tainted = null;
  return raw.map((d) => {
    const reasons = [];
    if (evidence.get(d.txid) !== 'ACCEPTED_FOR_PARITY') reasons.push(`evidence ${evidence.get(d.txid)}`);
    if (ev.priorState !== 'COMPLETE_FROM_LEGACY_JS_GENESIS') reasons.push('prior state incomplete');
    if (d.misses.length) reasons.push(`unrecorded RPC: ${[...new Set(d.misses)].join(', ')}`);
    if (tainted) reasons.push(`state tainted by ${tainted}`);
    if (!ORACLE_TYPES.has(d.type)) {
      if (d.valid && !tainted) tainted = `valid unsupported type ${d.type} tx ${d.txid}`;
      return { ...d, oracle: 'UNSUPPORTED_BY_ORACLE', inconclusive_reasons: reasons };
    }
    if (Object.values(d.numeric_fields).some((f) => f.precision === 'PRECISION_SUSPECT')) {
      reasons.push('JS Number amount not exactly representable in atoms');
    }
    return { ...d, oracle: reasons.length ? 'INCONCLUSIVE' : 'DECIDED', inconclusive_reasons: reasons };
  });
}

function summarize(records) {
  const counts = {};
  for (const r of records) counts[r.verdict ?? r.oracle] = (counts[r.verdict ?? r.oracle] || 0) + 1;
  return { counts, records };
}

function intraBlockOrderSensitive(blocks, ev) {
  // Legacy order: funding types first, then NeDB _id (txid hex) order. Rust
  // applies canonical block position. Report every multi-tx block where the
  // two orders differ; such blocks need ordering research before parity.
  const position = new Map(ev.candidates.map((c) => [c.txid, [c.height, c.transaction_position]]));
  return blocks.filter((b) => b.order.length > 1).map((b) => {
    const canonical = [...b.order].sort((x, y) => position.get(x)[1] - position.get(y)[1]);
    return { height: b.height, legacy_order: b.order, canonical_order: canonical,
      differs: canonical.some((t, i) => t !== b.order[i]) };
  }).filter((b) => b.differs);
}

function checkRecordings(recordings, manifest) {
  if (!recordings) return null;
  if (recordings.format !== 'litecoin-rpc-recordings-v1' || recordings.network !== manifest.network
      || recordings.pinned_litecoin_genesis !== manifest.pinned_litecoin_genesis) {
    throw Error('RPC recordings are for a different network or format');
  }
  return recordings;
}

async function run({ jsRoot, manifest, corpus, verification, rustFields, nedbRoot, recordings,
  ordering = 'legacy' }) {
  if (!['legacy', 'canonical-counterfactual'].includes(ordering)) throw Error(`unknown ordering ${ordering}`);
  assertIsolated();
  const provenance = assertPinned(jsRoot);
  const ev = loadEvidence(manifest, corpus, verification);
  const client = RecordedClient.proxy(new RecordedClient(ev, ev.network !== 'litecoin-mainnet',
    checkRecordings(recordings, manifest)));
  const L = await withQuietConsole(() => loadLegacy(jsRoot, client, nedbRoot, ev.network));
  await withQuietConsole(async () => {
    await L.db.init('LTC');
    await L.Activation.getInstance().init();
  });
  L.TxIndex.client = client;
  const syntax = await syntaxReport(L, ev, rustCarriers(rustFields));
  const actor = await actorReport(L, ev, client);
  // Index the candidate blocks with the legacy indexer, then replay.
  const genesis = LEGACY_GENESIS[ev.network];
  if (genesis === undefined) throw Error(`no legacy JS genesis for ${ev.network}`);
  const beforeGenesis = ev.candidates.filter((c) => c.height < genesis).map((c) => c.txid);
  const byBlock = new Map();
  for (const c of ev.candidates.filter((x) => x.height >= genesis)) {
    if (!byBlock.has(c.height)) byBlock.set(c.height, { hash: c.blockhash, tx: [] });
    byBlock.get(c.height).tx.push(c.txid);
  }
  for (const [height, block] of [...byBlock.entries()].sort((a, b) => a[0] - b[0])) {
    await withQuietConsole(() => L.TxIndex.processBlockData(block, height));
  }
  const replay = ev.end >= genesis
    ? await legacyConsensus(L, ev, client, Math.max(ev.start, genesis), ev.end, ordering)
    : { decisions: [], blocks: [] };
  let classified = classifyDecisions(replay.decisions, ev);
  if (ordering !== 'legacy') classified = classified.map((d) => ({ ...d, oracle: 'COUNTERFACTUAL' }));
  const decisions = summarize(classified);
  return {
    format: 'litecoin-legacy-js-replay-v1',
    // Inputs for scripts/litecoin_record_rpc.py on the next pass.
    rpc_requests: [...new Set(client.misses)].sort(),
    legacy_js: provenance,
    network: ev.network, scanned: [ev.start, ev.end], prior_state: ev.priorState,
    ordering: ordering === 'legacy' ? 'LEGACY (funding types 4/20 first, then NeDB _id = txid order)'
      : 'CANONICAL_COUNTERFACTUAL (block position; NOT legacy behaviour)',
    legacy_js_genesis: genesis, before_legacy_genesis_not_indexed: beforeGenesis,
    isolation: 'no network interfaces except loopback; RPC answered only from manifest',
    syntax, actor_reference: actor, decisions,
    blocks: replay.blocks,
    intra_block_order_divergence: intraBlockOrderSensitive(replay.blocks, ev),
    authority: 'diagnostic only; no TradeLayer acceptance, parity claim or replay authority',
  };
}

if (require.main === module) {
  const args = process.argv.slice(2);
  const opt = {};
  for (let i = 0; i < args.length; i += 2) opt[args[i].replace(/^--/, '')] = args[i + 1];
  if (!opt.js || !opt.manifest || !opt.corpus || !opt.output) {
    console.error('usage: unshare -rn node scripts/legacy_js_replay.cjs --js <pinned tradelayer.js> '
      + '--manifest <source-trace.json> --corpus <candidates.tsv> [--verification <verify.json>] '
      + '[--rust-fields <rust-fields.tsv>] [--rpc-recordings <recordings.json>] '
      + '[--ordering legacy|canonical-counterfactual] --output <report.json>');
    process.exit(2);
  }
  const nedbRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'tl-legacy-nedb-'));
  run({
    jsRoot: path.resolve(opt.js),
    manifest: JSON.parse(fs.readFileSync(opt.manifest, 'utf8')),
    corpus: fs.readFileSync(opt.corpus, 'utf8'),
    verification: opt.verification ? JSON.parse(fs.readFileSync(opt.verification, 'utf8')) : null,
    rustFields: opt['rust-fields'] ? fs.readFileSync(opt['rust-fields'], 'utf8') : '',
    recordings: opt['rpc-recordings'] ? JSON.parse(fs.readFileSync(opt['rpc-recordings'], 'utf8')) : null,
    ordering: opt.ordering || 'legacy',
    nedbRoot,
  }).then((report) => {
    fs.writeFileSync(opt.output, JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify({ syntax: report.syntax.counts, actor_reference: report.actor_reference.counts,
      decisions: report.decisions.counts, prior_state: report.prior_state }));
    process.exit(0);
  }, (err) => {
    console.error(`legacy replay refused: ${err.message}`);
    process.exit(1);
  });
}

module.exports = { satsToJsonNumber, assertIsolated, decodedOutput, loadEvidence, RecordedClient, checkRecordings,
  classifySyntax, classifyDecisions, numericFields, intraBlockOrderSensitive, rustCarriers,
  PINNED_COMMIT, PINNED_MAIN_JS_SHA256, LEGACY_GENESIS };
