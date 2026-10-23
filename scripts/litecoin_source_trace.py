#!/usr/bin/env python3
"""Stage 23A Litecoin Core source/reference trace for historical JS parity.

Only locally trusted Litecoin Core can supply the chain-consensus evidence.
The script independently checks raw transaction IDs, relevant outpoint bytes,
referenced transaction inclusion, output values/scripts and the active-chain
identity, then records what the pinned legacy JavaScript *would select*.

All outputs are READ ONLY. An address on an output script is neither a signer
proof nor an authorization to mutate JointChainState or settle type 3/23/31.
Requires an unpruned, fully synced Litecoin Core with txindex enabled.
"""
from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
import hashlib
import json
import os
from pathlib import Path
import sys

import collect_litecoin_rpc as base

LEGACY_JS_COMMIT = "dca0888b827c5bc28fa9189bc9e1d507f5c5670c"
SATOSHIS = 100_000_000
# Pinned Litecoin Core src/chainparams.cpp at ec1b6489a900d09cf5991e220dce089c77a232a2.
PINNED_GENESIS = {
    "main": "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2",
    "test": "4966625a4b2851d9fdee139e56211a0d88575f59ed816ff5e6a63deb4e3e29a0",
    "regtest": "530827f38f93b43ed12af0b3ad25a288dc02ed74d6d7857862df51fc56c416f9",
}


def pin_litecoin_genesis(rpc, network_label: str) -> str:
    labels = {"litecoin-mainnet": "main", "litecoin-testnet": "test",
              "litecoin-regtest": "regtest"}
    network = labels.get(network_label)
    if network is None:
        raise base.EvidenceError("unsupported TradeLayer network label")
    info = rpc.call("getblockchaininfo")
    if not isinstance(info, dict) or info.get("chain") != network:
        raise base.EvidenceError("node chain changed during source trace")
    if info.get("pruned") or info.get("initialblockdownload"):
        raise base.EvidenceError("source trace requires synced unpruned Litecoin Core")
    genesis = base.require_hash(rpc.call("getblockhash", [0]), "Litecoin genesis")
    if genesis != PINNED_GENESIS[network]:
        raise base.EvidenceError("wrong chain genesis: expected Litecoin Core")
    return genesis


def require_satoshis(value: object) -> int:
    """Decode the node's JSON decimal value without binary float rounding."""
    if isinstance(value, bool) or not isinstance(value, (float, int, str)):
        raise base.EvidenceError("invalid decoded output value")
    try:
        amount = Decimal(str(value)) * SATOSHIS
    except InvalidOperation as err:
        raise base.EvidenceError("invalid decoded output value") from err
    if not amount.is_finite() or amount < 0 or amount != amount.to_integral_value():
        raise base.EvidenceError("nonintegral or negative Litecoin output value")
    atoms = int(amount)
    if atoms > 84_000_000 * SATOSHIS:
        raise base.EvidenceError("Litecoin output exceeds supply bound")
    return atoms


def transaction_layout(hex_value: str) -> tuple[list[tuple[str, int]], list[tuple[int, str]]]:
    """Outpoints, then exact (satoshis, script) from the serialized transaction."""
    # The existing scanner checks witness shape, canonical CompactSize and txid.
    base.inspect_raw_transaction(hex_value)
    raw = bytes.fromhex(hex_value)
    cur = base.Cursor(raw)
    cur.read(4)
    if raw[4:6] == b"\x00\x01":
        cur.read(2)
        has_witness = True
    else:
        has_witness = False
    count = cur.number()
    if not 1 <= count <= 100_000:
        raise base.EvidenceError("invalid input count")
    inputs = []
    for _ in range(count):
        prev = cur.read(32)[::-1].hex()
        vout = int.from_bytes(cur.read(4), "little")
        cur.var_bytes()
        cur.read(4)
        inputs.append((prev, vout))
    count_out = cur.number()
    if not 1 <= count_out <= 100_000:
        raise base.EvidenceError("invalid output count")
    outputs = []
    for _ in range(count_out):
        sat = int.from_bytes(cur.read(8), "little")
        outputs.append((sat, cur.var_bytes().hex()))
    if has_witness:
        for _ in inputs:
            for _ in range(cur.number()):
                cur.var_bytes()
    cur.read(4)
    if cur.index != len(raw):
        raise base.EvidenceError("trailing raw transaction bytes")
    return inputs, outputs


def script_address(script: dict) -> dict:
    """Keep the older addresses[0] and current address RPC schemas distinct."""
    if not isinstance(script, dict) or not isinstance(script.get("hex"), str):
        raise base.EvidenceError("missing decoded script")
    older = script.get("addresses")
    newer = script.get("address")
    if older is not None and (
        not isinstance(older, list)
        or any(not isinstance(s, str) or not s for s in older)
    ):
        raise base.EvidenceError("invalid legacy script address array")
    if newer is not None and (not isinstance(newer, str) or not newer):
        raise base.EvidenceError("invalid modern script address")
    if older is not None and len(older) == 1:
        status = "legacy-exact"
        address = older[0]
        if newer is not None and newer != address:
            status = "schema-conflict"
            address = None
    elif older:
        status, address = "ambiguous", None
    elif newer is not None:
        status, address = "modern-only", newer
    else:
        status, address = "unresolved", None
    return {"status": status, "address": address,
            "legacy_addresses": older, "modern_address": newer}


def check_output(decoded: dict, n: int, raw_output: tuple[int, str]) -> dict:
    if not isinstance(decoded, dict) or decoded.get("n") != n:
        raise base.EvidenceError("decoded vout position mismatch")
    script = decoded.get("scriptPubKey")
    if not isinstance(script, dict) or not isinstance(script.get("hex"), str) or script["hex"].lower() != raw_output[1]:
        raise base.EvidenceError("decoded script differs from original output bytes")
    if require_satoshis(decoded.get("value")) != raw_output[0]:
        raise base.EvidenceError("decoded value differs from original output bytes")
    return {"vout": n, "satoshis": raw_output[0], "script_hex": raw_output[1],
            "script_type": script.get("type"), "address": script_address(script)}


def checked_block(rpc, block_hash: str, height: int, confirmations: int,
                  verbosity: int) -> dict:
    block = rpc.call("getblock", [block_hash, verbosity])
    if not isinstance(block, dict) or base.require_hash(block.get("hash"), "block hash") != block_hash:
        raise base.EvidenceError("block identity mismatch")
    if type(block.get("height")) is not int or block["height"] != height:
        raise base.EvidenceError("block height mismatch")
    if type(block.get("confirmations")) is not int or block["confirmations"] < confirmations:
        raise base.EvidenceError("block is no longer sufficiently confirmed")
    txs = block.get("tx")
    if not isinstance(txs, list) or not txs:
        raise base.EvidenceError("empty or malformed block transactions")
    txids = [base.require_hash(t if verbosity == 1 else t.get("txid"), "txid") for t in txs]
    if len(txids) != len(set(txids)):
        raise base.EvidenceError("duplicate txid in block")
    if base.merkle_root(txids) != base.require_hash(block.get("merkleroot"), "merkle root"):
        raise base.EvidenceError("block Merkle root mismatch")
    if base.require_hash(rpc.call("getblockhash", [height]), "active block hash") != block_hash:
        raise base.EvidenceError("block is no longer on the node's active chain")
    return block


def checked_parent_output(rpc, prev_txid: str, prev_vout: int,
                          child_height: int, child_index: int,
                          cache: dict) -> dict:
    if prev_txid == "00" * 32 and prev_vout == 0xffffffff:
        raise base.EvidenceError("coinbase cannot be a TradeLayer sender input")
    if prev_vout >= 100_000:
        raise base.EvidenceError("outpoint exceeds supported vout limit")
    parent = rpc.call("getrawtransaction", [prev_txid, True])
    if not isinstance(parent, dict) or base.require_hash(parent.get("txid"), "parent txid") != prev_txid:
        raise base.EvidenceError("previous transaction identity mismatch")
    block_hash = base.require_hash(parent.get("blockhash"), "parent block hash")
    if type(parent.get("confirmations")) is not int or parent["confirmations"] < 1:
        raise base.EvidenceError("previous transaction is unconfirmed")
    if block_hash not in cache:
        meta = rpc.call("getblock", [block_hash, 1])
        if not isinstance(meta, dict) or type(meta.get("height")) is not int:
            raise base.EvidenceError("missing previous transaction block height")
        cache[block_hash] = checked_block(rpc, block_hash, meta["height"], 1, 1)
    block = cache[block_hash]
    if block["height"] > child_height:
        raise base.EvidenceError("outpoint spent before its creation")
    positions = [i for i, txid in enumerate(block["tx"]) if txid == prev_txid]
    if len(positions) != 1 or (
        block["height"] == child_height and positions[0] >= child_index
    ):
        raise base.EvidenceError("previous transaction missing or later in block order")
    raw_hex = rpc.call("getrawtransaction", [prev_txid, False, block_hash])
    actual, _ = base.inspect_raw_transaction(raw_hex)
    if actual != prev_txid:
        raise base.EvidenceError("previous raw txid mismatch")
    _, outputs = transaction_layout(raw_hex)
    decoded = parent.get("vout")
    if not isinstance(decoded, list) or len(decoded) != len(outputs) or prev_vout >= len(outputs):
        raise base.EvidenceError("previous transaction output list mismatch")
    item = check_output(decoded[prev_vout], prev_vout, outputs[prev_vout])
    item.update({"txid": prev_txid, "blockhash": block_hash,
                 "height": block["height"], "position": positions[0]})
    return item


def augment_with_sources(rpc, tsv: str, manifest: dict) -> dict:
    """Return a fresh manifest; fail closed on any malformed or stale node evidence."""
    if manifest.get("format") != "tradelayer-archive-evidence-v1":
        raise base.EvidenceError("unrecognized archive manifest")
    if not manifest.get("canonical_range_rechecked"):
        raise base.EvidenceError("archive was not canonically rechecked")
    genesis = pin_litecoin_genesis(rpc, manifest.get("network"))
    known = {}
    for line in tsv.splitlines():
        if line and not line.startswith("#"):
            parts = line.split("\t")
            if len(parts) != 6:
                raise base.EvidenceError("invalid candidate TSV")
            if parts[4] in known:
                raise base.EvidenceError("duplicate candidate in corpus")
            known[parts[4]] = parts
    if len(known) != manifest["transactions_selected"]:
        raise base.EvidenceError("candidate TSV and manifest disagree")
    data = json.loads(json.dumps(manifest))  # avoid mutating caller's manifest
    cached_blocks, cached_parents = {}, {}
    for candidate in data["transactions"]:
        height = candidate["height"]
        bh = base.require_hash(candidate["blockhash"], "candidate block")
        txid = base.require_hash(candidate["txid"], "candidate txid")
        row = known.get(txid)
        if row is None or row[0] != candidate["label"] or row[2] != str(height) or row[3] != bh:
            raise base.EvidenceError("candidate corpus order/provenance mismatch")
        if bh not in cached_blocks:
            cached_blocks[bh] = checked_block(rpc, bh, height, manifest["min_confirmations"], 2)
        block = cached_blocks[bh]
        index = candidate["transaction_position"]
        if type(index) is not int or index < 0 or index >= len(block["tx"]):
            raise base.EvidenceError("candidate position outside block")
        tx = block["tx"][index]
        if base.require_hash(tx.get("txid"), "candidate txid") != txid:
            raise base.EvidenceError("candidate transaction position mismatch")
        raw = rpc.call("getrawtransaction", [txid, False, bh])
        derived, _ = base.inspect_raw_transaction(raw)
        if derived != txid or raw.lower() != row[5]:
            raise base.EvidenceError("candidate raw transaction changed")
        inputs, outputs = transaction_layout(raw)
        decoded = tx.get("vout")
        if not isinstance(decoded, list) or len(decoded) != len(outputs):
            raise base.EvidenceError("candidate decoded output count mismatch")
        output_provenance = [check_output(out, i, outputs[i]) for i, out in enumerate(decoded)]
        vin = tx.get("vin")
        if not isinstance(vin, list) or len(vin) != len(inputs):
            raise base.EvidenceError("candidate decoded input count mismatch")
        for i, (prev, vout) in enumerate(inputs):
            if (base.require_hash(vin[i].get("txid"), "candidate prevout txid") != prev
                    or vin[i].get("vout") != vout):
                raise base.EvidenceError("candidate decoded input disagrees with raw outpoint")
        sender = checked_parent_output(rpc, *inputs[0], height, index, cached_parents)
        # txIndex.processTransaction calls getTransactionOutputs then keeps vout 0/1.
        indexed_reference = [out for out in output_provenance[:2]
                             if out["address"]["status"] != "unresolved"]
        # txUtils.getReference instead chooses the LAST non-nulldata output.
        eligible = [out for out in output_provenance if out["script_type"] != "nulldata"]
        candidate["source_trace"] = {
            "actor_rule": "legacy-first-input-parent-vout",
            "sender_prevout": sender,
            "indexed_reference_outputs_0_1": indexed_reference,
            "last_non_nulldata_reference": eligible[-1] if eligible else None,
            "all_prevouts": [{"txid": a, "vout": b} for a, b in inputs],
            "actor_economic_authorization": False,
        }
    # An active-chain recheck after parent resolution prevents returning a
    # successful trace across a reorg of a candidate or a referenced parent.
    for bh, block in {**cached_blocks, **cached_parents}.items():
        if base.require_hash(rpc.call("getblockhash", [block["height"]]), "final active hash") != bh:
            raise base.EvidenceError("chain reorganized while resolving source outpoints")
    data["source_trace_format"] = "litecoin-legacy-js-source-trace-v1"
    data["pinned_litecoin_genesis"] = genesis
    data["legacy_js_commit"] = LEGACY_JS_COMMIT
    data["source_trace_evidence"] = "locally-trusted-litecoin-core-plus-raw-prevout-checks"
    data["sender_identity_status"] = "output-script-address-only; signer/ownership semantics unresolved"
    data["joint_replay_authorized"] = False
    data["txindex_required"] = True
    data["corpus_sha256"] = hashlib.sha256(tsv.encode("utf-8")).hexdigest()
    return data


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", required=True, choices=tuple(base.NETWORK_PORTS))
    parser.add_argument("--start-height", type=int, required=True)
    parser.add_argument("--end-height", type=int, required=True)
    parser.add_argument("--min-confirmations", type=int, default=6)
    parser.add_argument("--rpc-url")
    parser.add_argument("--cookie-file", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.output.resolve() == args.manifest.resolve():
        parser.error("corpus and manifest paths must differ")
    try:
        user, password = os.getenv("TL_RPC_USER"), os.getenv("TL_RPC_PASSWORD")
        if user is not None and password is not None:
            credential = f"{user}:{password}"
        else:
            cookie = args.cookie_file or base.default_cookie_file(args.network)
            credential = cookie.read_text(encoding="utf-8").strip()
            if ":" not in credential:
                raise base.EvidenceError("RPC cookie missing username/password separator")
        rpc = base.LitecoinRPC(args.rpc_url or
                               f"http://127.0.0.1:{base.NETWORK_PORTS[args.network]}", credential)
        tsv, manifest = base.scan_blocks(rpc, args.network, args.start_height,
                                         args.end_height, args.min_confirmations)
        manifest = augment_with_sources(rpc, tsv, manifest)
        base.atomic_write(args.output, tsv)
        base.atomic_write(args.manifest, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        print(f"Scanned {manifest['blocks_scanned']} blocks, traced "
              f"{manifest['transactions_selected']} candidates; joint replay remains denied")
        return 0
    except (OSError, base.EvidenceError, TypeError, KeyError, ValueError) as exc:
        print(f"source trace failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
