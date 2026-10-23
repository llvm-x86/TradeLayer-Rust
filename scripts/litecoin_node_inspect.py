#!/usr/bin/env python3
"""Stage 23B read-only Litecoin Core suitability report.

Prints OS, the litecoin-cli path (located, never executed), the loopback RPC
endpoint, chain, tip, prune/IBD state, txindex status, node version and the
height-zero hash compared with pinned Litecoin chainparams. Uses only
getblockchaininfo, getblockhash, getnetworkinfo and getindexinfo. Credentials,
peers, local addresses and wallet state are never requested or printed.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import sys

import collect_litecoin_rpc as base
import litecoin_source_trace as trace


def load_credential(cookie_file: Path | None, network: str) -> str:
    user, password = os.getenv("TL_RPC_USER"), os.getenv("TL_RPC_PASSWORD")
    if user is not None and password is not None:
        return f"{user}:{password}"
    cookie = cookie_file or base.default_cookie_file(network)
    credential = cookie.read_text(encoding="utf-8").strip()
    if ":" not in credential:
        raise base.EvidenceError("RPC cookie missing username/password separator")
    return credential


def report(rpc, network: str, url: str) -> dict:
    record = trace.inspect_node(rpc, network)
    record.update({
        "os": f"{platform.system()} {platform.release()}",
        "litecoin_cli_path": shutil.which("litecoin-cli"),
        "rpc_endpoint": url,
        "source": "read-only local Litecoin Core JSON-RPC",
        "consensus_note": "node-reported state; no independent Scrypt/PoW verification",
    })
    return record


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", required=True, choices=tuple(base.NETWORK_PORTS))
    parser.add_argument("--rpc-url", help="loopback JSON-RPC endpoint only")
    parser.add_argument("--cookie-file", type=Path)
    args = parser.parse_args(argv)
    url = args.rpc_url or f"http://127.0.0.1:{base.NETWORK_PORTS[args.network]}"
    try:
        rpc = base.LitecoinRPC(url, load_credential(args.cookie_file, args.network))
        record = report(rpc, args.network, url)
    except (OSError, base.EvidenceError) as exc:
        print(f"node inspection failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0 if record["suitable_for_source_trace"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
