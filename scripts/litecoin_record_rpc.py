#!/usr/bin/env python3
"""Stage 23B: record node answers that the pinned legacy JS consults.

The legacy validateSend falls back to RPC validateaddress whenever its local
check fails (all Litecoin-testnet bech32 addresses do; see
docs/STAGE_23_LITECOIN_REPLAY.md). legacy_js_replay.cjs lists such requests
as rpc_requests. This script answers only validateaddress, from a suitable
local Litecoin Core, and keeps only address-derived fields. Every other
request kind is reported as unrecordable: it marks an evidence gap.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import collect_litecoin_rpc as base
import litecoin_node_inspect as inspect_cli
import litecoin_source_trace as trace

LABELS = {"main": "litecoin-mainnet", "test": "litecoin-testnet", "regtest": "litecoin-regtest"}
KEPT_FIELDS = ("isvalid", "address", "scriptPubKey", "isscript", "iswitness",
               "witness_version", "witness_program")
MAX_REQUESTS = 10_000


def parse_requests(requests: list[str]) -> tuple[list[str], list[str]]:
    addresses, unrecordable = [], []
    for item in requests:
        method, _, raw_args = item.partition(" ")
        try:
            args = json.loads(raw_args) if raw_args else []
        except ValueError:
            args = None
        if method == "validateAddress" and isinstance(args, list) and len(args) == 1 \
                and isinstance(args[0], str) and 0 < len(args[0]) <= 128:
            addresses.append(args[0])
        else:
            unrecordable.append(item)
    return sorted(set(addresses)), sorted(set(unrecordable))


def record(rpc, network: str, requests: list[str]) -> dict:
    if len(requests) > MAX_REQUESTS:
        raise base.EvidenceError("too many RPC requests to record")
    node = trace.inspect_node(rpc, network)
    if not node["suitable_for_source_trace"]:
        raise base.EvidenceError("node unsuitable: " + "; ".join(node["unsuitable_reasons"]))
    addresses, unrecordable = parse_requests(requests)
    answers = {}
    for address in addresses:
        result = rpc.call("validateaddress", [address])
        if not isinstance(result, dict) or not isinstance(result.get("isvalid"), bool):
            raise base.EvidenceError("invalid validateaddress response")
        answers[address] = {key: result[key] for key in KEPT_FIELDS if key in result}
    return {"format": "litecoin-rpc-recordings-v1", "network": LABELS[network],
            "pinned_litecoin_genesis": node["genesis"], "node_inspection": node,
            "validateaddress": answers, "unrecordable_requests": unrecordable,
            "authority": "node answers for legacy JS replay only; no TradeLayer authority"}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", required=True, choices=tuple(base.NETWORK_PORTS))
    parser.add_argument("--replay", type=Path, required=True, help="legacy_js_replay.cjs report")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--cookie-file", type=Path)
    args = parser.parse_args(argv)
    try:
        requests = json.loads(args.replay.read_text(encoding="utf-8")).get("rpc_requests", [])
        url = args.rpc_url or f"http://127.0.0.1:{base.NETWORK_PORTS[args.network]}"
        rpc = base.LitecoinRPC(url, inspect_cli.load_credential(args.cookie_file, args.network))
        recordings = record(rpc, args.network, requests)
        base.atomic_write(args.output, json.dumps(recordings, indent=2, sort_keys=True) + "\n")
    except (OSError, ValueError, base.EvidenceError) as exc:
        print(f"recording failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps({"validateaddress": len(recordings["validateaddress"]),
                      "unrecordable": len(recordings["unrecordable_requests"])}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
