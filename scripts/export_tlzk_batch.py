#!/usr/bin/env python3
"""Extract a zkTL checkpoint as a checked, read-only Rust audit transcript.

Checks zkTL's *canonical JSON commitments*. It does not verify an STWO proof,
a Cairo trace, economic validity, Bitcoin inclusion, or chain identity.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

MAX_INPUT = 8 * 1024 * 1024
MAX_RECEIPTS = 10_000
RECEIPT_FIELDS = {
    'protocol', 'txid', 'txType', 'block', 'oldStateRoot', 'newStateRoot',
    'payloadHash', 'witnessRoot', 'supportLevel', 'economicMutation', 'valid', 'reason'
}
BATCH_FIELDS = {
    'protocol', 'startHeight', 'endHeight', 'transitionRoot',
    'firstStateRoot', 'finalStateRoot', 'receiptIds'
}


class CheckError(ValueError):
    pass


def canonical(value):
    """Match zkTL/src/hash_utils.js for ASCII string keys and integer scalars."""
    if isinstance(value, dict):
        if not all(isinstance(k, str) and k.isascii() for k in value):
            raise CheckError('non-ASCII JSON key unsupported')
        return '{' + ','.join(
            json.dumps(k, ensure_ascii=False) + ':' + canonical(value[k])
            for k in sorted(value)
        ) + '}'
    if isinstance(value, list):
        return '[' + ','.join(canonical(v) for v in value) + ']'
    if value is None:
        return 'null'
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if type(value) is int:
        if abs(value) > 9007199254740991:
            raise CheckError('integer exceeds JS safe precision')
        return str(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False, separators=(',', ':'))
    raise CheckError('unsupported JSON scalar in hash input')


def digest(value):
    return hashlib.sha256(canonical(value).encode('utf-8')).hexdigest()


def checked_hex(value, name):
    if not isinstance(value, str) or len(value) != 64 or value != value.lower() or any(c not in '0123456789abcdef' for c in value):
        raise CheckError(name + ' must be a lowercase hex32')
    return value


def checked_integer(value, name):
    if type(value) is not int or value < 0 or value > 9007199254740991:
        raise CheckError(name + ' must be a nonnegative safe integer')
    return value


def validate_batch(batch):
    if not isinstance(batch, dict) or batch.get('kind') != 'tlzk_aggregated_batch_checkpoint':
        raise CheckError('invalid checkpoint kind')
    core = batch.get('batchCore')
    receipts = batch.get('transitionReceipts')
    if not isinstance(core, dict) or set(core) != BATCH_FIELDS or core['protocol'] != 'tlzk_aggregated_batch_checkpoint_v1':
        raise CheckError('unknown batch schema')
    if not isinstance(receipts, list) or not (1 <= len(receipts) <= MAX_RECEIPTS):
        raise CheckError('empty or excessive receipt count')
    start = checked_integer(core['startHeight'], 'startHeight')
    end = checked_integer(core['endHeight'], 'endHeight')
    if start > end:
        raise CheckError('invalid interval')
    first = checked_hex(core['firstStateRoot'], 'first root')
    previous = first
    last_height = start
    seen = set()
    ids = []
    rows = []
    for n, receipt in enumerate(receipts):
        if not isinstance(receipt, dict) or receipt.get('kind') != 'tlzk_trace_transition_receipt':
            raise CheckError(f'unsupported receipt shape {n}')
        data = receipt.get('receiptCore')
        if not isinstance(data, dict) or set(data) != RECEIPT_FIELDS or data['protocol'] != 'tlzk_trace_transition_receipt_v1':
            raise CheckError(f'unsupported receipt core {n}')
        if data['reason'] != '' or data['valid'] is not True:
            raise CheckError(f'invalid/noncanonical receipt {n}')
        if data['supportLevel'] != 'state_kernel':
            raise CheckError(f'audit-only receipt {n} cannot stand for an economic proof')
        if type(data['economicMutation']) is not bool:
            raise CheckError(f'bad economicMutation {n}')
        tx_type = checked_integer(data['txType'], 'txType')
        if tx_type > 35 or tx_type == 34:
            raise CheckError(f'tx type {tx_type} conflicts with TradeLayer.js or is unknown')
        height = checked_integer(data['block'], 'block')
        if height < last_height or height > end:
            raise CheckError(f'block ordering error {n}')
        txid = checked_hex(data['txid'], 'txid')
        if txid in seen:
            raise CheckError(f'duplicate txid {n}')
        seen.add(txid)
        old = checked_hex(data['oldStateRoot'], 'oldStateRoot')
        new = checked_hex(data['newStateRoot'], 'newStateRoot')
        if old != previous:
            raise CheckError(f'state root chain break {n}')
        checked_hex(data['payloadHash'], 'payloadHash')
        checked_hex(data['witnessRoot'], 'witnessRoot')
        expected_id = digest(data)
        rid = checked_hex(receipt.get('receiptId'), 'receiptId')
        if rid != expected_id:
            raise CheckError(f'receipt digest mismatch {n}')
        ids.append(rid)
        rows.append('\t'.join([
            'R', rid, txid, str(tx_type), str(height), old, new,
            data['payloadHash'], data['witnessRoot'], data['supportLevel'],
            '1' if data['economicMutation'] else '0', '1'
        ]))
        previous = new
        last_height = height
    if previous != checked_hex(core['finalStateRoot'], 'finalStateRoot'):
        raise CheckError('final root mismatch')
    if core['receiptIds'] != ids:
        raise CheckError('receipt order mismatch')
    if digest(ids) != checked_hex(core['transitionRoot'], 'transitionRoot'):
        raise CheckError('transitionRoot mismatch')
    bid = checked_hex(batch.get('batchId'), 'batchId')
    if digest(core) != bid:
        raise CheckError('batchId mismatch')
    return '\n'.join([
        '\t'.join(['TLZK1', bid, str(start), str(end), core['transitionRoot'], first, core['finalStateRoot']]),
        *rows, ''
    ])


def load_checked(path):
    data = path.read_bytes()
    if len(data) > MAX_INPUT:
        raise CheckError('input too large')

    def unique_pairs(pairs):
        value = {}
        for key, val in pairs:
            if key in value:
                raise CheckError('duplicate JSON key')
            value[key] = val
        return value

    def reject_constant(value):
        raise CheckError('noncanonical JSON constant ' + value)

    return json.loads(data.decode('utf-8'), object_pairs_hook=unique_pairs, parse_constant=reject_constant)


def main(argv=None):
    parser = argparse.ArgumentParser(description='Strict zkTL batch metadata export; PROOF IS UNVERIFIED')
    parser.add_argument('batch_json', type=Path)
    parser.add_argument('--out', type=Path, help='TSV output, defaults to stdout')
    args = parser.parse_args(argv)
    try:
        out = validate_batch(load_checked(args.batch_json))
        if args.out:
            args.out.write_text(out, encoding='ascii')
        else:
            sys.stdout.write(out)
    except (CheckError, OSError, ValueError, UnicodeDecodeError) as e:
        print(f'REJECTED: {e}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
