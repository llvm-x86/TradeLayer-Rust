#!/usr/bin/env python3
"""Stage 23C: export a source-trace manifest for Rust authentication.

Writes the raw bytes `tradelayer-ltc-authenticate` needs, in a line format
that a dependency-free Rust reader can parse: the scanned headers, then per
candidate its raw transaction, Merkle branch, position and tx count, plus the
sender parent's raw bytes, header, branch and position. Hashes are display
order. Nothing node-decoded (addresses, types) is exported: Rust re-derives
them from bytes so the comparison is independent.

  python3 scripts/export_ltc_evidence.py --corpus c.tsv --manifest m.json --output e.tsv
  cargo run --locked --bin tradelayer-ltc-authenticate -- e.tsv > rust-auth.tsv
  python3 scripts/export_ltc_evidence.py --compare rust-auth.tsv --manifest m.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import collect_litecoin_rpc as base

NETWORKS = {"litecoin-mainnet": "main", "litecoin-testnet": "test"}


def export(manifest: dict, corpus: str) -> str:
    network = NETWORKS.get(manifest.get("network"))
    if network is None:
        raise base.EvidenceError("only Litecoin mainnet/testnet evidence can be exported")
    raw = {}
    for line in corpus.splitlines():
        if line and not line.startswith("#"):
            fields = line.split("\t")
            raw[fields[4]] = fields[5]
    previous = manifest.get("previous_block_hash") or "-"
    lines = [f"# litecoin-evidence-export-v1\t{network}\t{manifest['start_height']}\t{previous}"]
    for height in range(manifest["start_height"], manifest["end_height"] + 1):
        lines.append(f"H\t{height}\t{manifest['block_headers'][str(height)]}")
    for cand in manifest["transactions"]:
        sender = cand["source_trace"]["sender_prevout"]
        lines.append("\t".join([
            "C", str(cand["height"]), str(cand["transaction_position"]), str(cand["block_tx_count"]),
            ",".join(cand["merkle_branch"]) or "-", raw[cand["txid"]],
            str(sender["height"]), str(sender["position"]), str(sender["block_tx_count"]),
            ",".join(sender["merkle_branch"]) or "-", sender["header_hex"], sender["raw_hex"],
        ]))
    return "\n".join(lines) + "\n"


def expected_sender(address: dict) -> str:
    """The Rust column a node-reported address status should correspond to."""
    status = address["status"]
    if status in ("legacy-exact", "modern-only"):
        return f"single:{address['address']}"
    if status == "ambiguous":
        return "ambiguous"
    if status == "unresolved":
        return "none"
    return f"conflict:{status}"


def reference(out: dict | None) -> str:
    if out is None:
        return "-"
    selected = out["address"]["legacy_js_selected_address"]
    return f"{out['vout']}:{selected if selected is not None else 'none'}"


def compare(rust_tsv: str, manifest: dict) -> dict:
    """Rust script-derived addresses versus node-reported legacy addresses.

    An ambiguous (multi-address) output mismatches by design: legacy JS
    silently takes addresses[0], Rust refuses to pick one.
    """
    rust, rejected = {}, []
    for line in rust_tsv.splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if fields[0] == "REJECTED":
            rejected.append(line)
            continue
        rust[fields[0]] = {"sender": fields[4], "vout_0_1": fields[6], "last": fields[7]}
    records = []
    for cand in manifest["transactions"]:
        st = cand["source_trace"]
        r = rust.get(cand["txid"])
        if r is None:
            records.append({"txid": cand["txid"], "verdict": "NOT_AUTHENTICATED"})
            continue
        sender = r["sender"].split(":", 1)[0] if not r["sender"].startswith("single:") else r["sender"]
        fields = {
            "sender": sender == expected_sender(st["sender_prevout"]["address"]),
            "vout_0_1": r["vout_0_1"] == (",".join(reference(o) for o in st["indexed_reference_outputs_0_1"]) or "-"),
            "last_non_nulldata": r["last"] == reference(st["last_non_nulldata_reference"]),
        }
        records.append({"txid": cand["txid"], "verdict": "MATCH" if all(fields.values()) else "MISMATCH",
                        "fields": fields, "rust": r})
    counts = {}
    for rec in records:
        counts[rec["verdict"]] = counts.get(rec["verdict"], 0) + 1
    return {"counts": counts, "rejected_blocks": rejected, "records": records}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--compare", type=Path, help="tradelayer-ltc-authenticate output")
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        if args.compare:
            print(json.dumps(compare(args.compare.read_text(encoding="utf-8"), manifest), indent=2))
            return 0
        if not args.corpus or not args.output:
            parser.error("--corpus and --output are required for export")
        base.atomic_write(args.output, export(manifest, args.corpus.read_text(encoding="utf-8")))
    except (OSError, KeyError, ValueError, base.EvidenceError) as exc:
        print(f"export failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
