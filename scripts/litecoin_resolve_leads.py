#!/usr/bin/env python3
"""Stage 23B: resolve historical TradeLayer txid LEADS on a local Litecoin Core.

Leads come from fixtures/litecoin_historical_leads.tsv (txids found in the
pinned legacy JS repository). Only leads whose claimed network equals the
node's network are queried, so testnet heights are never searched on mainnet.
A resolved lead yields its block height, Merkle-checked position, raw-txid
check and carrier class, plus a <=500-block scan window for the collector.

NOT_FOUND is a legitimate negative observation (the txid is not on this
chain); it is never upgraded to evidence. Resolution says nothing about
TradeLayer acceptance and confers no replay authority.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import collect_litecoin_rpc as base
import litecoin_node_inspect as inspect_cli
import litecoin_source_trace as trace

RPC_INVALID_ADDRESS_OR_KEY = -5  # Core: "No such mempool or blockchain transaction"
NETWORKS = ("main", "test", "regtest")


def read_leads(path: Path) -> list[dict]:
    leads, seen = [], set()
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 7:
            raise base.EvidenceError("lead row must have 7 tab-separated fields")
        lead_id, network, basis, txid, height, source, role = parts
        if network not in NETWORKS + ("unknown",):
            raise base.EvidenceError(f"lead {lead_id}: invalid claimed network")
        txid = base.require_hash(txid, f"lead {lead_id} txid")
        if lead_id in seen:
            raise base.EvidenceError(f"duplicate lead id {lead_id}")
        seen.add(lead_id)
        claimed = None
        if height != "-":
            if not height.isdigit():
                raise base.EvidenceError(f"lead {lead_id}: invalid claimed height")
            claimed = int(height)
        leads.append({"lead_id": lead_id, "claimed_network": network, "network_basis": basis,
                      "txid": txid, "claimed_height": claimed, "source": source, "role": role})
    return leads


def resolve_one(rpc, lead: dict, tip: int, confirmations: int) -> dict:
    result = {key: lead[key] for key in ("lead_id", "txid", "claimed_height", "source", "role")}
    try:
        decoded = rpc.call("getrawtransaction", [lead["txid"], True])
    except base.RPCResponseError as err:
        if err.code != RPC_INVALID_ADDRESS_OR_KEY:
            raise
        result["status"] = "NOT_FOUND_ON_THIS_CHAIN"
        return result
    if not isinstance(decoded, dict) or base.require_hash(decoded.get("txid"), "lead txid") != lead["txid"]:
        raise base.EvidenceError("node returned a different transaction for a lead")
    if decoded.get("blockhash") is None:
        result["status"] = "UNCONFIRMED"
        return result
    block_hash = base.require_hash(decoded["blockhash"], "lead block hash")
    meta = rpc.call("getblock", [block_hash, 1])
    if not isinstance(meta, dict) or type(meta.get("height")) is not int:
        raise base.EvidenceError("missing lead block height")
    height = meta["height"]
    if height + confirmations > tip + 1:
        result.update({"status": "INSUFFICIENT_CONFIRMATIONS", "height": height})
        return result
    block = trace.checked_block(rpc, block_hash, height, confirmations, 1)
    positions = [i for i, txid in enumerate(block["tx"]) if txid == lead["txid"]]
    if len(positions) != 1:
        raise base.EvidenceError("lead txid not uniquely present in its Merkle-checked block")
    raw = rpc.call("getrawtransaction", [lead["txid"], False, block_hash])
    parsed = base.parse_transaction(raw)
    if parsed["txid"] != lead["txid"]:
        raise base.EvidenceError("lead raw txid mismatch")
    vout = decoded.get("vout")
    if not isinstance(vout, list) or len(vout) != len(parsed["outputs"]):
        raise base.EvidenceError("lead decoded output count mismatch")
    for n, output in enumerate(vout):
        trace.check_output(output, n, parsed["outputs"][n])
    strict = base.selected_candidates(decoded)
    legacy = base.legacy_js_carrier(decoded)
    carrier = base.carrier_class({"outputs": strict, "legacy_js_carrier": legacy}) \
        if strict or legacy else "none"
    claimed = lead["claimed_height"]
    result.update({
        "status": "RESOLVED",
        "height": height,
        "blockhash": block_hash,
        "position": positions[0],
        "claimed_height_check": "none" if claimed is None else ("match" if claimed == height else "MISMATCH"),
        "carrier_class": carrier,
        "mweb": parsed["mweb"],
    })
    return result


def suggest_windows(results: list[dict], tip: int, confirmations: int) -> list[dict]:
    """Forward <=500-block windows from each resolved lead (a spent UTXO's
    spender, or block neighbours of a carrier). The last eligible height is
    tip - confirmations + 1 so the collector's confirmation check can pass."""
    last = tip - confirmations + 1
    windows = {}
    for item in results:
        if item.get("status") != "RESOLVED" or item["height"] > last:
            continue
        start = item["height"]
        end = min(start + base.MAX_BLOCKS - 1, last)
        windows.setdefault((start, end), []).append(item["lead_id"])
    return [{"start_height": s, "end_height": e, "lead_ids": ids}
            for (s, e), ids in sorted(windows.items())]


def resolve_leads(rpc, network: str, leads: list[dict], confirmations: int,
                  include_unknown: bool) -> dict:
    if confirmations < 1:
        raise base.EvidenceError("at least one confirmation required")
    node = trace.inspect_node(rpc, network)
    if not node["suitable_for_source_trace"]:
        raise base.EvidenceError("node unsuitable: " + "; ".join(node["unsuitable_reasons"]))
    tip = rpc.call("getblockcount")
    if type(tip) is not int:
        raise base.EvidenceError("invalid block count")
    wanted = {network, "unknown"} if include_unknown else {network}
    results = []
    for lead in leads:
        if lead["claimed_network"] not in wanted:
            results.append({"lead_id": lead["lead_id"], "txid": lead["txid"],
                            "status": "SKIPPED_OTHER_NETWORK",
                            "claimed_network": lead["claimed_network"]})
            continue
        results.append(resolve_one(rpc, lead, tip, confirmations))
    for item in results:
        if item.get("status") == "RESOLVED":
            active = base.require_hash(rpc.call("getblockhash", [item["height"]]), "final active hash")
            if active != item["blockhash"]:
                raise base.EvidenceError("chain reorganized while resolving leads")
    return {
        "format": "litecoin-lead-resolution-v1",
        "network": network,
        "node_inspection": node,
        "reported_tip_height": tip,
        "min_confirmations": confirmations,
        "legacy_js_commit": trace.LEGACY_JS_COMMIT,
        "evidence_status": "lead resolution only; no TradeLayer acceptance or replay authority",
        "leads": results,
        "suggested_windows": suggest_windows(results, tip, confirmations),
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", required=True, choices=NETWORKS)
    parser.add_argument("--leads", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--min-confirmations", type=int, default=6)
    parser.add_argument("--include-unknown", action="store_true",
                        help="also query leads whose network could not be inferred")
    parser.add_argument("--rpc-url")
    parser.add_argument("--cookie-file", type=Path)
    args = parser.parse_args(argv)
    try:
        url = args.rpc_url or f"http://127.0.0.1:{base.NETWORK_PORTS[args.network]}"
        rpc = base.LitecoinRPC(url, inspect_cli.load_credential(args.cookie_file, args.network))
        report = resolve_leads(rpc, args.network, read_leads(args.leads),
                               args.min_confirmations, args.include_unknown)
        base.atomic_write(args.output, json.dumps(report, indent=2, sort_keys=True) + "\n")
    except (OSError, base.EvidenceError) as exc:
        print(f"lead resolution failed: {exc}", file=sys.stderr)
        return 1
    counts = {}
    for item in report["leads"]:
        counts[item["status"]] = counts.get(item["status"], 0) + 1
    print(json.dumps(counts, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
