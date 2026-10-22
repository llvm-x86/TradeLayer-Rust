"""Unit fixtures for zkTL's canonical hashing and fail-closed exporter."""
import copy
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('tlzk_export', ROOT / 'scripts' / 'export_tlzk_batch.py')
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class BridgeTests(unittest.TestCase):
    def fixture(self):
        x = '1' * 64
        y = '2' * 64
        core = dict(protocol='tlzk_trace_transition_receipt_v1', txid='3' * 64, txType=2, block=99,
                    oldStateRoot=x, newStateRoot=y, payloadHash='4' * 64, witnessRoot='5' * 64,
                    supportLevel='state_kernel', economicMutation=True, valid=True, reason='')
        receipt = {'kind': 'tlzk_trace_transition_receipt', 'receiptId': bridge.digest(core), 'receiptCore': core}
        batch_core = dict(protocol='tlzk_aggregated_batch_checkpoint_v1', startHeight=99, endHeight=99,
                          firstStateRoot=x, finalStateRoot=y, receiptIds=[receipt['receiptId']],
                          transitionRoot=bridge.digest([receipt['receiptId']]))
        return {'kind':'tlzk_aggregated_batch_checkpoint', 'batchId':bridge.digest(batch_core),
                'batchCore':batch_core, 'transitionReceipts':[receipt]}

    def test_well_formed_metadata(self):
        output = bridge.validate_batch(self.fixture())
        self.assertEqual(output.splitlines()[0].split('\t')[0], 'TLZK1')
        self.assertEqual(len(output.splitlines()), 2)

    def test_mutated_receipt_root(self):
        a = self.fixture()
        a['transitionReceipts'][0]['receiptCore']['newStateRoot'] = '6'*64
        with self.assertRaises(bridge.CheckError): bridge.validate_batch(a)

    def test_audit_replay_is_not_economic_proof(self):
        a = self.fixture()
        a['transitionReceipts'][0]['receiptCore']['supportLevel'] = 'audit_replay'
        with self.assertRaisesRegex(bridge.CheckError, 'audit-only'): bridge.validate_batch(a)

    def test_colliding_type_34(self):
        a = self.fixture()
        a['transitionReceipts'][0]['receiptCore']['txType'] = 34
        with self.assertRaisesRegex(bridge.CheckError, 'conflicts'): bridge.validate_batch(a)

    def test_invalid_receipt_and_bad_numeric_type(self):
        a = self.fixture()
        a['transitionReceipts'][0]['receiptCore']['valid'] = False
        with self.assertRaises(bridge.CheckError): bridge.validate_batch(a)
        a = self.fixture()
        a['transitionReceipts'][0]['receiptCore']['txType'] = 36
        with self.assertRaises(bridge.CheckError): bridge.validate_batch(a)

    def test_duplicate_transaction_id(self):
        a = self.fixture()
        a['transitionReceipts'].append(copy.deepcopy(a['transitionReceipts'][0]))
        with self.assertRaises(bridge.CheckError): bridge.validate_batch(a)

    def test_canonical_hash_matches_known_sorted_case(self):
        self.assertEqual(bridge.canonical({'b':True,'a':5}), '{"a":5,"b":true}')
        self.assertEqual(bridge.digest([]), '4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945')


if __name__ == '__main__': unittest.main()
