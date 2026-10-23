#!/usr/bin/env python3
"""Stage 23B independent re-verification of Litecoin source-trace evidence.

Rechecks, from the stored bytes alone, what the collector and source trace
recorded: corpus SHA-256, raw txids, header hashes and parent links across
the scanned range, canonical Merkle positions, raw first-input outpoints,
parent transaction bytes and their header inclusion, decoded versus raw
output values and scripts, the node's 'nulldata' classification, both legacy
reference rules and both carrier selections. Several manifests may be given
in height order; their continuity and coverage are reported.

Each candidate is classified ACCEPTED_FOR_PARITY, INCONCLUSIVE or REJECTED.
ACCEPTED_FOR_PARITY means only "authentic enough to compare with the legacy
JS"; it is never TradeLayer acceptance or replay authority. Active-chain
membership is node-attested; --recheck-rpc repeats the final active-chain
hash comparison against a local node.

The optional per-header Scrypt check compares each stored header's
scrypt(N=1024,r=1,p=1) hash with its own nBits target. It does not validate
difficulty retargeting, chainwork or any other consensus rule.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

import collect_litecoin_rpc as base
import litecoin_node_inspect as inspect_cli
import litecoin_source_trace as trace

NETWORK_KEYS = {"litecoin-mainnet": "main", "litecoin-testnet": "test", "litecoin-regtest": "regtest"}
OP_PUSHDATA1, OP_PUSHDATA2, OP_PUSHDATA4, OP_16 = 0x4C, 0x4D, 0x4E, 0x60
# tradelayer.js@dca0888b src/main.js GENESIS_BLOCK_HEIGHTS.LTC (commented there
# as "Replace with actual ... genesis block height": an unresolved assumption).
LEGACY_JS_GENESIS = {"test": 3_082_500, "main": 2_819_000}


class Rejected(Exception):
    pass


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise Rejected(reason)


def is_push_only(script: bytes) -> bool:
    """CScript::IsPushOnly: every opcode parses and is <= OP_16."""
    i = 0
    while i < len(script):
        op = script[i]
        i += 1
        if op > OP_16:
            return False
        if op < OP_PUSHDATA1:
            size = op
        elif op == OP_PUSHDATA1:
            if i + 1 > len(script):
                return False
            size, i = script[i], i + 1
        elif op == OP_PUSHDATA2:
            if i + 2 > len(script):
                return False
            size, i = int.from_bytes(script[i:i + 2], "little"), i + 2
        elif op == OP_PUSHDATA4:
            if i + 4 > len(script):
                return False
            size, i = int.from_bytes(script[i:i + 4], "little"), i + 4
        else:
            size = 0
        if i + size > len(script):
            return False
        i += size
    return True


def is_nulldata(script_hex: str) -> bool:
    script = bytes.fromhex(script_hex)
    return len(script) >= 1 and script[0] == 0x6A and is_push_only(script[1:])


def scrypt_meets_bits(header_hex: str) -> bool:
    raw = bytes.fromhex(header_hex)
    bits = int.from_bytes(raw[72:76], "little")
    exponent, mantissa = bits >> 24, bits & 0x007FFFFF
    if bits & 0x00800000 or mantissa == 0:
        return False
    target = mantissa << (8 * (exponent - 3)) if exponent >= 3 else mantissa >> (8 * (3 - exponent))
    pow_hash = hashlib.scrypt(raw, salt=raw, n=1024, r=1, p=1, dklen=32)
    return int.from_bytes(pow_hash, "little") <= target


def read_corpus(text: str) -> list[list[str]]:
    rows = []
    for line in text.splitlines():
        if line and not line.startswith("#"):
            parts = line.split("\t")
            require(len(parts) == 6, "corpus row must have 6 fields")
            rows.append(parts)
    return rows


def verify_headers(manifest: dict, check_scrypt: bool) -> dict[int, dict]:
    headers = manifest.get("block_headers")
    require(isinstance(headers, dict), "manifest lacks offline-verifiable block headers")
    start, end = manifest["start_height"], manifest["end_height"]
    require(type(start) is int and type(end) is int and start <= end, "invalid scanned interval")
    require(sorted(headers, key=int) == [str(h) for h in range(start, end + 1)],
            "block headers do not cover exactly the scanned interval")
    require(manifest.get("blocks_scanned") == end - start + 1, "blocks_scanned disagrees with interval")
    parsed, prev = {}, manifest.get("previous_block_hash")
    for height in range(start, end + 1):
        header_hex = headers[str(height)]
        header = base.parse_header(header_hex)
        if prev is not None:
            require(header["prev"] == prev, f"header chain broken at height {height}")
        if check_scrypt:
            require(scrypt_meets_bits(header_hex), f"header at {height} misses its own Scrypt target")
        parsed[height] = header
        prev = header["hash"]
    require(prev == manifest.get("last_block_hash"), "last header differs from last_block_hash")
    return parsed


def check_outputs(decoded: list, raw_outputs: list) -> list[str]:
    """Recompare decoded outputs with raw bytes; recheck nulldata typing."""
    require(isinstance(decoded, list) and len(decoded) == len(raw_outputs), "decoded output count mismatch")
    notes = []
    for n, (item, (sat, script)) in enumerate(zip(decoded, raw_outputs)):
        require(item.get("vout") == n and item.get("satoshis") == sat and item.get("script_hex") == script,
                f"decoded output {n} differs from raw bytes")
        claimed = item.get("script_type") == "nulldata"
        require(claimed == is_nulldata(script), f"node nulldata classification of vout {n} disagrees with script")
        if item.get("script_type") == "pubkey":
            notes.append(f"vout {n} is P2PK: legacy RPC gives no addresses")
    return notes


def address_status(entry: dict | None) -> str:
    return "missing" if entry is None else entry["address"]["status"]


def verify_candidate(cand: dict, row: list[str], headers: dict[int, dict]) -> dict:
    outcome = {"label": cand.get("label"), "txid": cand.get("txid"), "height": cand.get("height"),
               "carrier_class": cand.get("carrier_class")}
    inconclusive = []
    height, position = cand["height"], cand["transaction_position"]
    require(row[0] == cand["label"] and row[2] == str(height) and row[3] == cand["blockhash"]
            and row[4] == cand["txid"], "corpus row disagrees with manifest")
    header = headers.get(height)
    require(header is not None and header["hash"] == cand["blockhash"], "candidate block not in scanned headers")
    parsed = base.parse_transaction(row[5])
    require(parsed["txid"] == cand["txid"], "raw transaction does not hash to txid")
    base.verify_merkle_position(cand["txid"], cand["merkle_branch"], position,
                                cand["block_tx_count"], header["merkleroot"])
    raw_scripts = [script for _, script in parsed["outputs"]]
    strict = []
    for n, script in enumerate(raw_scripts):
        payload = base.op_return_payload(script)
        if payload is not None and (payload.startswith(b"tl") or payload.startswith(b"TLR")):
            strict.append({"vout": n, "payload_hex": payload.hex(), "script_hex": script})
    require(strict == cand["outputs"], "strict carrier set differs from raw outputs")
    st = cand.get("source_trace")
    require(isinstance(st, dict), "candidate lacks source trace")
    outcome["notes"] = check_outputs(st["decoded_outputs"], parsed["outputs"])
    decoded_view = {"vout": [{"n": o["vout"], "scriptPubKey": {"hex": o["script_hex"], "type": o["script_type"]}}
                             for o in st["decoded_outputs"]]}
    require(base.legacy_js_carrier(decoded_view) == cand.get("legacy_js_carrier"),
            "legacy JS carrier differs from recomputation")
    require(cand.get("carrier_class") == base.carrier_class(cand), "carrier class mislabelled")
    require(all(i != ("00" * 32, 0xFFFFFFFF) for i in parsed["inputs"]), "coinbase input")
    require([{"txid": a, "vout": b} for a, b in parsed["inputs"]] == st["all_prevouts"],
            "recorded prevouts differ from raw inputs")
    sender = st["sender_prevout"]
    first_txid, first_vout = parsed["inputs"][0]
    require(sender["txid"] == first_txid and sender["vout"] == first_vout,
            "sender prevout is not the raw first input")
    parent = base.parse_transaction(sender["raw_hex"])
    require(parent["txid"] == first_txid, "parent raw bytes do not hash to the outpoint txid")
    require(first_vout < len(parent["outputs"]), "outpoint index outside parent outputs")
    require(sender.get("output_count") == len(parent["outputs"]), "parent output count differs from raw bytes")
    sat, script = parent["outputs"][first_vout]
    require(sender["satoshis"] == sat and sender["script_hex"] == script, "sender output differs from parent bytes")
    parent_header = base.parse_header(sender["header_hex"])
    require(parent_header["hash"] == sender["blockhash"], "parent header does not hash to its block")
    base.verify_merkle_position(first_txid, sender["merkle_branch"], sender["position"],
                                sender["block_tx_count"], parent_header["merkleroot"])
    require(sender["height"] < height or (sender["height"] == height and sender["position"] < position),
            "parent not earlier in canonical order")
    scanned = headers.get(sender["height"])
    if scanned is not None:
        require(scanned["hash"] == sender["blockhash"], "parent block conflicts with scanned header")
        outcome["parent_chain_membership"] = "within-scanned-headers"
    else:
        outcome["parent_chain_membership"] = "node-attested-only"
    status = sender["address"]["status"]
    if status != "legacy-exact":
        inconclusive.append(f"sender address {status}")
    with_addr = [o for o in st["decoded_outputs"] if o["address"]["legacy_js_selected_address"] is not None]
    vout01 = [o for o in with_addr if o["vout"] in (0, 1)]
    require(vout01 == st["indexed_reference_outputs_0_1"], "vout 0/1 reference recomputation differs")
    eligible = [o for o in st["decoded_outputs"] if o["script_type"] != "nulldata"]
    last = eligible[-1] if eligible else None
    require(last == st["last_non_nulldata_reference"], "last non-nulldata reference recomputation differs")
    if last is not None and address_status(last) != "legacy-exact":
        inconclusive.append(f"last non-nulldata reference address {address_status(last)}")
    outcome["reference_rules"] = {
        "vout_0_1": [o["vout"] for o in vout01],
        "last_non_nulldata": None if last is None else last["vout"],
        "agree": last is not None and [o["vout"] for o in vout01] == [last["vout"]],
    }
    require(st.get("actor_economic_authorization") is False, "source trace claims economic authorization")
    outcome["sender_address_status"] = status
    outcome["parity_input"] = "INCONCLUSIVE" if inconclusive else "ACCEPTED_FOR_PARITY"
    outcome["reasons"] = inconclusive
    return outcome


def verify_manifest(manifest: dict, corpus: str, commit: str, check_scrypt: bool) -> dict:
    network = NETWORK_KEYS.get(manifest.get("network"))
    require(network is not None, "unsupported network label")
    require(manifest.get("format") == "tradelayer-archive-evidence-v1", "unrecognized archive format")
    require(manifest.get("source_trace_format") == "litecoin-legacy-js-source-trace-v1", "not a source trace")
    require(manifest.get("pinned_litecoin_genesis") == trace.PINNED_GENESIS[network], "genesis pin mismatch")
    require(manifest.get("legacy_js_commit") == commit, "legacy JS commit differs from expected")
    require(manifest.get("joint_replay_authorized") is False, "manifest claims joint replay authority")
    node = manifest.get("node_inspection")
    require(isinstance(node, dict) and node.get("suitable_for_source_trace") is True,
            "manifest lacks a suitable node inspection")
    require(hashlib.sha256(corpus.encode("utf-8")).hexdigest() == manifest.get("corpus_sha256"),
            "corpus SHA-256 differs from manifest")
    headers = verify_headers(manifest, check_scrypt)
    rows = read_corpus(corpus)
    cands = manifest.get("transactions")
    require(isinstance(cands, list) and len(cands) == len(rows) == manifest.get("transactions_selected"),
            "candidate count mismatch")
    order = [(c["height"], c["transaction_position"]) for c in cands]
    require(order == sorted(set(order)), "candidates not in canonical order")
    results = []
    for cand, row in zip(cands, rows):
        try:
            results.append(verify_candidate(cand, row, headers))
        except (Rejected, base.EvidenceError, KeyError, TypeError, ValueError) as err:
            results.append({"label": cand.get("label"), "txid": cand.get("txid"),
                            "height": cand.get("height"), "parity_input": "REJECTED",
                            "reasons": [str(err) or type(err).__name__]})
    return {"network": network, "start_height": manifest["start_height"],
            "end_height": manifest["end_height"], "node_version": node.get("subversion"),
            "previous_block_hash": manifest.get("previous_block_hash"),
            "last_block_hash": manifest["last_block_hash"], "candidates": results,
            "header_hashes": {h: v["hash"] for h, v in headers.items()}}


def coverage(reports: list[dict]) -> dict:
    networks = {r["network"] for r in reports}
    require(len(networks) == 1, "manifests from different networks")
    legacy_genesis = LEGACY_JS_GENESIS.get(next(iter(networks)))
    intervals, contiguous = [], True
    for prev, cur in zip(reports, reports[1:]):
        if not (prev["end_height"] + 1 == cur["start_height"]
                and prev["last_block_hash"] == cur["previous_block_hash"]):
            contiguous = False
    for r in reports:
        if intervals and intervals[-1][1] + 1 == r["start_height"]:
            intervals[-1][1] = r["end_height"]
        else:
            intervals.append([r["start_height"], r["end_height"]])
    complete = (contiguous and legacy_genesis is not None
                and reports[0]["start_height"] <= legacy_genesis <= reports[-1]["end_height"])
    return {"network": networks.pop(), "intervals": intervals, "contiguous_hash_linked": contiguous,
            "pinned_legacy_js_genesis": legacy_genesis,
            "prior_state": "COMPLETE_FROM_LEGACY_JS_GENESIS" if complete else "INCOMPLETE"}


def recheck_active_chain(rpc, reports: list[dict]) -> int:
    network = reports[0]["network"]
    node = trace.inspect_node(rpc, network)
    require(node["suitable_for_source_trace"], "recheck node unsuitable: " + "; ".join(node["unsuitable_reasons"]))
    checked = 0
    for report in reports:
        for height, block_hash in report["header_hashes"].items():
            active = base.require_hash(rpc.call("getblockhash", [height]), "active hash")
            if active != block_hash:
                for cand in report["candidates"]:
                    cand["parity_input"] = "REJECTED"
                    cand.setdefault("reasons", []).append(f"reorg: height {height} no longer active")
            checked += 1
    return checked


def verify(pairs: list[tuple[str, str]], commit: str, check_scrypt: bool, rpc=None) -> dict:
    reports, seen = [], set()
    for corpus, manifest_text in pairs:
        manifest = json.loads(manifest_text)
        report = verify_manifest(manifest, corpus, commit, check_scrypt)
        for cand in report["candidates"]:
            require(cand["txid"] not in seen, "candidate repeated across manifests")
            seen.add(cand["txid"])
        reports.append(report)
    require(reports, "no manifests supplied")
    cover = coverage(reports)
    rechecked = recheck_active_chain(rpc, reports) if rpc is not None else None
    counts = {"ACCEPTED_FOR_PARITY": 0, "INCONCLUSIVE": 0, "REJECTED": 0}
    for report in reports:
        for cand in report["candidates"]:
            counts[cand["parity_input"]] += 1
        report["header_hashes"] = len(report["header_hashes"])
    return {"format": "litecoin-evidence-verification-v1", "legacy_js_commit": commit,
            "scrypt_header_check": "per-header nBits target only" if check_scrypt else "skipped",
            "active_chain": (f"rechecked {rechecked} heights against local node" if rechecked is not None
                             else "node-attested at collection; not rechecked"),
            "coverage": cover, "counts": counts, "manifests": reports,
            "authority": "evidence classification only; no TradeLayer acceptance or replay authority"}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pair", nargs=2, action="append", metavar=("CORPUS_TSV", "MANIFEST_JSON"),
                        required=True, help="repeat in ascending height order")
    parser.add_argument("--legacy-js-commit", default=trace.LEGACY_JS_COMMIT)
    parser.add_argument("--skip-scrypt", action="store_true")
    parser.add_argument("--recheck-rpc", action="store_true", help="recheck active-chain hashes on a local node")
    parser.add_argument("--rpc-url")
    parser.add_argument("--cookie-file", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        pairs = [(Path(c).read_text(encoding="utf-8"), Path(m).read_text(encoding="utf-8")) for c, m in args.pair]
        rpc = None
        if args.recheck_rpc:
            network = NETWORK_KEYS[json.loads(pairs[0][1])["network"]]
            url = args.rpc_url or f"http://127.0.0.1:{base.NETWORK_PORTS[network]}"
            rpc = base.LitecoinRPC(url, inspect_cli.load_credential(args.cookie_file, network))
        report = verify(pairs, args.legacy_js_commit, not args.skip_scrypt, rpc)
        base.atomic_write(args.output, json.dumps(report, indent=2, sort_keys=True) + "\n")
    except (OSError, ValueError, KeyError, Rejected, base.EvidenceError) as exc:
        print(f"evidence REJECTED: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"counts": report["counts"], "prior_state": report["coverage"]["prior_state"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
