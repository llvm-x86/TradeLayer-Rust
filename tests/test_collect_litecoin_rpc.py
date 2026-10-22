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
        self.block_hash = '20' * 32
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


if __name__ == '__main__':
    unittest.main()
