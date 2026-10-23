"""Hermetic Litecoin RPC integration tests; never contact a live node."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

MODULE = Path(__file__).resolve().parents[1] / 'scripts' / 'collect_litecoin_rpc.py'
spec = importlib.util.spec_from_file_location('collect_litecoin_rpc', MODULE)
collector = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = collector
spec.loader.exec_module(collector)

SYNTHETIC = Path(__file__).resolve().parents[1] / 'fixtures' / 'synthetic_transactions.tsv'
PUBLIC = Path(__file__).resolve().parents[1] / 'fixtures' / 'real_transactions.tsv'


def fake_header(prev, merkle, nonce=0):
    """Synthetic header; its SHA256d becomes the fake block hash (no PoW)."""
    raw = (b'\x00\x00\x00\x20' + bytes.fromhex(prev)[::-1] + bytes.fromhex(merkle)[::-1]
           + (1_700_000_000).to_bytes(4, 'little') + (0x1e0ffff0).to_bytes(4, 'little')
           + nonce.to_bytes(4, 'little'))
    return raw.hex(), collector.sha256d(raw)[::-1].hex()


def fixture(path):
    return [line.split('\t') for line in path.read_text().splitlines() if line and not line.startswith('#')]


class FakeRpc:
    def __init__(self, raw, height=101):
        self.height = height
        self.txids, self.txs = [], []
        for row in raw:
            hex_value = row[-1]
            txid, scripts = collector.inspect_raw_transaction(hex_value)
            assert txid == row[-2]
            self.txids.append(txid)
            self.txs.append({'txid': txid, 'hex': hex_value, 'vout': [
                {'n': n, 'scriptPubKey': {'hex': script}} for n, script in enumerate(scripts)]})
        self.parent = '10' * 32
        self.header, self.block_hash = fake_header(self.parent, collector.merkle_root(self.txids))
        self.block = {
            'hash': self.block_hash,
            'height': height,
            'previousblockhash': self.parent,
            'confirmations': 30,
            'merkleroot': collector.merkle_root(self.txids),
            'tx': self.txs,
        }
        self.reorg = False
        self.hash_queries = 0

    def call(self, method, params=None):
        if method == 'getblockchaininfo':
            return {'chain': 'test', 'pruned': False, 'initialblockdownload': False}
        if method == 'getblockcount':
            return 150
        if method == 'getblockhash':
            if params[0] == self.height - 1:
                return self.parent
            if params[0] == self.height:
                self.hash_queries += 1
                return self.block_hash if not self.reorg or self.hash_queries == 1 else '30' * 32
            raise AssertionError(params)
        if method == 'getblockheader':
            if params != [self.block_hash, False]:
                raise AssertionError(params)
            return self.header
        if method == 'getblock':
            if params != [self.block_hash, 2]:
                raise AssertionError(params)
            return self.block
        if method == 'getrawtransaction':
            txid, verbose, blockhash = params
            if verbose is not False or blockhash != self.block_hash:
                raise AssertionError(params)
            return self.txs[self.txids.index(txid)]['hex']
        raise AssertionError(method)


class CollectorTests(unittest.TestCase):
    def test_merkle_root_single_and_odd(self):
        sample = fixture(SYNTHETIC)
        self.assertEqual(collector.merkle_root([sample[0][-2]]), sample[0][-2])
        triple = [row[-2] for row in sample]
        self.assertEqual(collector.merkle_root(triple), collector.merkle_root(triple))
        self.assertNotEqual(collector.merkle_root(triple), collector.merkle_root(triple[:2]))

    def test_read_only_collection_and_provenance(self):
        rows = fixture(SYNTHETIC)
        rpc = FakeRpc(rows)
        tsv, manifest = collector.scan_blocks(rpc, 'test', 101, 101)
        self.assertEqual(manifest['transactions_selected'], 3)
        self.assertEqual(len(manifest['transactions']), 3)
        self.assertIn('ltc-h101-i0', tsv)
        self.assertIn('litecoin-testnet\t101', tsv)
        self.assertEqual(manifest['transactions'][0]['outputs'][0]['payload_hex'], '746c32303b616c6963653b353b32')
        self.assertTrue(manifest['merkle_root_checked'])
        self.assertEqual(manifest['last_block_hash'], rpc.block_hash)
        self.assertTrue(manifest['canonical_range_rechecked'])

    def test_txid_and_script_verified(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        rpc.txs[0]['vout'][0]['scriptPubKey']['hex'] = '6a02746c'
        with self.assertRaisesRegex(collector.EvidenceError, 'script differs'):
            collector.scan_blocks(rpc, 'test', 101, 101)

    def test_merkle_tampering_fails(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        rpc.block['merkleroot'] = 'ff' * 32
        with self.assertRaisesRegex(collector.EvidenceError, 'Merkle root mismatch'):
            collector.scan_blocks(rpc, 'test', 101, 101)

    def test_missing_confirmations_pruned_network_and_reorg(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        rpc.block['confirmations'] = 1
        with self.assertRaisesRegex(collector.EvidenceError, 'sufficiently confirmed'):
            collector.scan_blocks(rpc, 'test', 101, 101)
        rpc.block['confirmations'] = 30
        rpc.hash_queries = 0
        rpc.reorg = True
        with self.assertRaisesRegex(collector.EvidenceError, 'hash mismatch|reorganized'):
            collector.scan_blocks(rpc, 'test', 101, 101)
        with self.assertRaisesRegex(collector.EvidenceError, 'different chain'):
            collector.scan_blocks(rpc, 'main', 101, 101)

    def test_out_of_bound_interval(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        with self.assertRaisesRegex(collector.EvidenceError, 'maximum'):
            collector.scan_blocks(rpc, 'test', 1, 999)

    def test_local_rpc_only(self):
        for url in ['https://localhost:9332', 'http://example.com:9332',
                    'http://127.0.0.1:9332?x=y', 'http://alice:password@127.0.0.1:9332']:
            with self.assertRaises(collector.EvidenceError):
                collector.LitecoinRPC(url, 'user:password')
        self.assertIsInstance(collector.LitecoinRPC('http://127.0.0.1:9332', 'user:password'), collector.LitecoinRPC)

    def test_core_cookie_paths(self):
        home = Path('/example/home')
        self.assertEqual(collector.default_cookie_file('test', home),
                         home / '.litecoin' / 'testnet4' / '.cookie')
        self.assertEqual(collector.default_cookie_file('main', home),
                         home / '.litecoin' / '.cookie')
        self.assertEqual(collector.default_cookie_file('regtest', home),
                         home / '.litecoin' / 'regtest' / '.cookie')
        with self.assertRaises(collector.EvidenceError):
            collector.default_cookie_file('unknown', home)

    def test_transaction_hashes_for_published_bitcoin_examples(self):
        for row in fixture(PUBLIC):
            derived, outputs = collector.inspect_raw_transaction(row[-1])
            self.assertEqual(derived, row[-2])
            self.assertTrue(outputs)

    def test_canonical_op_return(self):
        self.assertEqual(collector.op_return_payload('6a02746c'), b'tl')
        self.assertIsNone(collector.op_return_payload('6a4c02746c'))
        self.assertIsNone(collector.op_return_payload('6a02746c51'))
        self.assertIsNone(collector.op_return_payload('006a02746c'))
        self.assertIsNone(collector.op_return_payload('6a'))



def synthetic_tx(scripts, flags=0, witness=None, mweb=b'', vin=1):
    """Synthetic serialization only; never presented as chain evidence."""
    body = bytearray([vin])
    for _ in range(vin):
        body += bytes(range(32)) + b'\x01\x00\x00\x00' + b'\x00' + b'\xff\xff\xff\xff'
    body.append(len(scripts))
    for script in scripts:
        body += (1000).to_bytes(8, 'little') + bytes([len(script)]) + script
    version, locktime = b'\x02\x00\x00\x00', b'\x00\x00\x00\x00'
    legacy = version + bytes(body) + locktime
    if not flags:
        return legacy.hex(), legacy
    extended = version + b'\x00' + bytes([flags]) + bytes(body)
    if flags & 1:
        extended += witness
    if flags & 8:
        extended += mweb
    return (extended + locktime).hex(), legacy


P2PKH = bytes.fromhex('76a914' + '11' * 20 + '88ac')


class MwebAndLegacyCarrierTests(unittest.TestCase):
    def test_hogex_and_opaque_mweb_txid_excludes_mweb_and_witness(self):
        for flags, witness, mweb in [(8, None, b'\x00'), (9, b'\x01\x01\x07', b'\x00'),
                                     (8, None, b'\x01' + b'\xee' * 40)]:
            hex_value, legacy = synthetic_tx([P2PKH], flags, witness, mweb)
            parsed = collector.parse_transaction(hex_value)
            self.assertEqual(parsed['txid'], collector.sha256d(legacy)[::-1].hex())
            self.assertEqual(parsed['outputs'], [(1000, P2PKH.hex())])
            self.assertEqual(parsed['mweb'], 'hogex' if mweb == b'\x00' else 'opaque')
            self.assertEqual(parsed['witness'], bool(flags & 1))

    def test_malformed_mweb_and_unknown_flags_fail_closed(self):
        cases = [(8, None, b'\x02'), (8, None, b''), (2, None, b''), (0x18, None, b'\x00')]
        for flags, witness, mweb in cases:
            hex_value, _ = synthetic_tx([P2PKH], flags, witness, mweb)
            with self.assertRaises(collector.EvidenceError):
                collector.parse_transaction(hex_value)
        # MWEB-only: no canonical inputs, so no canonical txid preimage exists.
        hex_value, _ = synthetic_tx([P2PKH], 8, None, b'\x01' + b'\xee' * 8, vin=0)
        with self.assertRaisesRegex(collector.EvidenceError, 'input count'):
            collector.parse_transaction(hex_value)

    @staticmethod
    def decoded(scripts_and_types):
        return {'vout': [{'n': n, 'scriptPubKey': {'hex': h, 'type': ty}}
                         for n, (h, ty) in enumerate(scripts_and_types)]}

    def test_legacy_js_marker_positions(self):
        self.assertEqual(collector.legacy_js_carrier(self.decoded([('6a04746c3141', 'nulldata')]))['marker_nibble'], 4)
        self.assertEqual(collector.legacy_js_carrier(self.decoded([('6a4c04746c3141', 'nulldata')]))['marker_nibble'], 6)
        self.assertEqual(collector.legacy_js_carrier(self.decoded([('6a0746c0', 'nulldata')]))['marker_nibble'], 3)
        # A wrong-length push is still selected: JS has no push-length check.
        self.assertEqual(collector.legacy_js_carrier(self.decoded([('6a17746c', 'nulldata')]))['marker_nibble'], 4)
        # PUSHDATA2 puts the marker at nibble 8: JS rejects what Rust accepts.
        pushdata2 = '6a4d0001746c' + '41' * 254
        self.assertIsNone(collector.legacy_js_carrier(self.decoded([(pushdata2, 'nulldata')])))
        self.assertEqual(collector.op_return_payload(pushdata2)[:2], b'tl')
        # Only the FIRST nulldata output is examined.
        first_only = self.decoded([('6a024142', 'nulldata'), ('6a02746c', 'nulldata')])
        self.assertIsNone(collector.legacy_js_carrier(first_only))
        # A non-push-only OP_RETURN is 'nonstandard' to the node, so JS skips it.
        self.assertIsNone(collector.legacy_js_carrier(self.decoded([('6a746c', 'nonstandard')])))

    def test_scan_records_legacy_js_only_candidates(self):
        script = bytes.fromhex('6a4c04746c3141')  # strict reader refuses non-minimal push
        hex_value, _ = synthetic_tx([script, P2PKH])
        txid, _ = collector.inspect_raw_transaction(hex_value)
        rpc = FakeRpc([['x', 'litecoin-testnet', '-', '-', txid, hex_value]])
        rpc.txs[0]['vout'][0]['scriptPubKey']['type'] = 'nulldata'
        _, manifest = collector.scan_blocks(rpc, 'test', 101, 101)
        self.assertEqual(manifest['transactions_selected'], 1)
        record = manifest['transactions'][0]
        self.assertEqual(record['carrier_class'], 'legacy-js-only')
        self.assertEqual(record['outputs'], [])
        self.assertEqual(record['legacy_js_carrier']['marker_nibble'], 6)
        self.assertIn('legacy-js-first-nulldata-746c-nibble-0-6', manifest['selection_rules'])
        rpc.txs[0]['vout'][0]['scriptPubKey']['hex'] = '6a4c04746c3142'
        with self.assertRaisesRegex(collector.EvidenceError, 'script differs'):
            collector.scan_blocks(rpc, 'test', 101, 101)

    def test_rpc_method_allowlist_is_read_only(self):
        rpc = collector.LitecoinRPC('http://127.0.0.1:9332', 'user:password')
        for method in ['dumpprivkey', 'sendrawtransaction', 'getwalletinfo', 'stop',
                       'addnode', 'setnetworkactive', 'getpeerinfo']:
            with self.assertRaisesRegex(collector.EvidenceError, 'unsupported RPC method'):
                rpc.call(method)



class HeaderAndBranchTests(unittest.TestCase):
    def test_branch_roundtrip_all_positions_and_sizes(self):
        for size in range(1, 12):
            txids = [collector.sha256d(bytes([size, i]))[::-1].hex() for i in range(size)]
            root = collector.merkle_root(txids)
            for index, txid in enumerate(txids):
                branch = collector.merkle_branch(txids, index)
                self.assertEqual(collector.root_from_branch(txid, branch, index), root)
                collector.verify_merkle_position(txid, branch, index, size, root)
                if size > 1:
                    # Membership alone is position-ambiguous for a self-paired
                    # right-edge leaf; position binding must still refuse.
                    with self.assertRaises(collector.EvidenceError):
                        collector.verify_merkle_position(txid, branch, index ^ 1, size, root)
                if size % 2 and index == size - 1:
                    # An inflated count moves the right edge; the self-paired
                    # leaf no longer sits on it. (For other leaves tx_count is
                    # node-reported and not header-authenticated.)
                    with self.assertRaises(collector.EvidenceError):
                        collector.verify_merkle_position(txid, branch, index, size + 1, root)
        with self.assertRaises(collector.EvidenceError):
            collector.root_from_branch(txids[0], [], 1)

    def test_scan_records_headers_and_branches(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        _, manifest = collector.scan_blocks(rpc, 'test', 101, 101)
        self.assertEqual(manifest['block_headers'], {'101': rpc.header})
        for record in manifest['transactions']:
            root = collector.root_from_branch(record['txid'], record['merkle_branch'],
                                              record['transaction_position'])
            self.assertEqual(root, collector.parse_header(rpc.header)['merkleroot'])

    def test_header_binding_failures(self):
        rpc = FakeRpc(fixture(SYNTHETIC))
        good = rpc.header
        for header, message in [
            (fake_header('10' * 32, collector.merkle_root(rpc.txids), nonce=1)[0], 'do not hash'),
            ('00' * 79, 'exactly 80 bytes'),
        ]:
            rpc.header = header
            with self.assertRaisesRegex(collector.EvidenceError, message):
                collector.scan_blocks(rpc, 'test', 101, 101)
        rpc.header = good
        wrong_parent, wrong_parent_hash = fake_header('11' * 32, collector.merkle_root(rpc.txids))
        rpc.header, rpc.block_hash, rpc.block['hash'] = wrong_parent, wrong_parent_hash, wrong_parent_hash
        with self.assertRaisesRegex(collector.EvidenceError, 'header parent mismatch'):
            collector.scan_blocks(rpc, 'test', 101, 101)
        wrong_root, wrong_root_hash = fake_header('10' * 32, 'ee' * 32)
        rpc.header, rpc.block_hash, rpc.block['hash'] = wrong_root, wrong_root_hash, wrong_root_hash
        with self.assertRaisesRegex(collector.EvidenceError, 'header Merkle root mismatch'):
            collector.scan_blocks(rpc, 'test', 101, 101)

    def test_duplicate_txid_block_rejected(self):
        rows = fixture(SYNTHETIC)
        rpc = FakeRpc([rows[0], rows[0]])
        with self.assertRaisesRegex(collector.EvidenceError, 'duplicate txid'):
            collector.scan_blocks(rpc, 'test', 101, 101)


if __name__ == '__main__':
    unittest.main()
