#!/usr/bin/env python3
"""Read-only Litecoin Core archive collector for TradeLayer wire evidence.

Writes transactions *only* when a locally validated, unpruned Litecoin Core
instance reports them in a sufficiently confirmed canonical block and the
collector independently verifies the whole block's txid Merkle root, raw
transaction txids, and candidate script bytes. This is NOT proof of TradeLayer
protocol acceptance or a standalone proof of chain consensus.

Requires only Python 3 standard library and local RPC; no external packages,
private keys, broadcast or wallet RPCs. Credentials stay out of output files.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

MAX_BLOCKS = 500
MAX_RESPONSE_BYTES = 32 * 1024 * 1024
MAX_TRANSACTION_BYTES = 1_000_000
MAX_CANDIDATES = 10_000
NETWORK_PORTS = {"main": 9332, "test": 19332, "regtest": 19443}
HASH_RE = re.compile(r"^[0-9a-fA-F]{64}$")


class EvidenceError(Exception):
    pass


def require_hash(value: object, name: str) -> str:
    if not isinstance(value, str) or not HASH_RE.fullmatch(value):
        raise EvidenceError(f"invalid {name}")
    return value.lower()


def sha256d(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def merkle_root(txids: list[str]) -> str:
    """Bitcoin-family non-witness txid Merkle root, display-order result."""
    if not txids:
        raise EvidenceError("empty transaction list")
    level = [bytes.fromhex(require_hash(s, "Merkle leaf"))[::-1] for s in txids]
    while len(level) > 1:
        if len(level) % 2:
            level.append(level[-1])
        level = [sha256d(level[n] + level[n + 1]) for n in range(0, len(level), 2)]
    return level[0][::-1].hex()


class Cursor:
    def __init__(self, data: bytes):
        self.data = data
        self.index = 0

    def read(self, n: int) -> bytes:
        if n < 0 or n > MAX_TRANSACTION_BYTES or self.index + n > len(self.data):
            raise EvidenceError("invalid or truncated transaction")
        start = self.index
        self.index += n
        return self.data[start:self.index]

    def number(self) -> int:
        prefix = self.read(1)[0]
        if prefix < 253:
            return prefix
        widths = {253: 2, 254: 4, 255: 8}
        result = int.from_bytes(self.read(widths[prefix]), "little")
        if result < {253: 253, 254: 65536, 255: 4294967296}[prefix]:
            raise EvidenceError("noncanonical CompactSize")
        return result

    def var_bytes(self) -> bytes:
        return self.read(self.number())


def inspect_raw_transaction(hex_value: str) -> tuple[str, list[str]]:
    """Compute txid and exact vout scripts; handle segregated witnesses."""
    if not isinstance(hex_value, str) or len(hex_value) > MAX_TRANSACTION_BYTES * 2:
        raise EvidenceError("transaction exceeds size limit")
    try:
        raw = bytes.fromhex(hex_value)
    except ValueError as err:
        raise EvidenceError("malformed transaction hex") from err
    cursor = Cursor(raw)
    version = cursor.read(4)
    has_witness = raw[4:6] == b"\x00\x01"
    if has_witness:
        cursor.read(2)
    start = cursor.index
    vin_count = cursor.number()
    if not 1 <= vin_count <= 100_000:
        raise EvidenceError("invalid input count")
    for _ in range(vin_count):
        cursor.read(36)
        cursor.var_bytes()
        cursor.read(4)
    output_count = cursor.number()
    if not 1 <= output_count <= 100_000:
        raise EvidenceError("invalid output count")
    scripts = []
    for _ in range(output_count):
        cursor.read(8)
        scripts.append(cursor.var_bytes().hex())
    end = cursor.index
    if has_witness:
        nonempty = False
        for _ in range(vin_count):
            witnesses = cursor.number()
            if witnesses > 100_000:
                raise EvidenceError("excessive witness count")
            for _ in range(witnesses):
                nonempty |= bool(cursor.var_bytes())
        if not nonempty:
            raise EvidenceError("superfluous SegWit witness")
    locktime = cursor.read(4)
    if cursor.index != len(raw):
        raise EvidenceError("trailing transaction bytes")
    stripped = version + raw[start:end] + locktime if has_witness else raw
    return sha256d(stripped)[::-1].hex(), scripts


def op_return_payload(script_hex: str) -> bytes | None:
    """Exactly one canonical OP_RETURN push, mirroring Rust scanner."""
    try:
        data = bytes.fromhex(script_hex)
    except (ValueError, TypeError):
        raise EvidenceError("invalid vout script hex") from None
    if len(data) < 2 or data[0] != 0x6a:
        return None
    op = data[1]
    if 1 <= op <= 75:
        size, start = op, 2
    elif op == 0x4c and len(data) >= 3 and data[2] > 75:
        size, start = data[2], 3
    elif op == 0x4d and len(data) >= 4:
        size, start = int.from_bytes(data[2:4], "little"), 4
        if size <= 255:
            return None
    elif op == 0x4e and len(data) >= 6:
        size, start = int.from_bytes(data[2:6], "little"), 6
        if size <= 65535:
            return None
    else:
        return None
    if size != len(data) - start:
        return None
    return data[start:]


def selected_candidates(tx: dict) -> list[tuple[int, bytes, str]]:
    if not isinstance(tx, dict) or not isinstance(tx.get("vout"), list):
        raise EvidenceError("missing decoded transaction outputs")
    found = []
    for position, output in enumerate(tx["vout"]):
        if output.get("n") != position:
            raise EvidenceError("noncanonical vout index")
        script = output.get("scriptPubKey")
        if not isinstance(script, dict) or not isinstance(script.get("hex"), str):
            raise EvidenceError("missing output script")
        payload = op_return_payload(script["hex"])
        if payload is not None and (payload.startswith(b"tl") or payload.startswith(b"TLR")):
            found.append((position, payload, script["hex"].lower()))
    return found


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        raise EvidenceError("RPC redirect refused")


class LitecoinRPC:
    def __init__(self, url: str, credential: str):
        parsed = urllib.parse.urlparse(url)
        host = parsed.hostname
        if parsed.scheme != "http" or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
            raise EvidenceError("RPC URL must be a plain loopback HTTP address")
        try:
            is_loopback = host == "localhost" or ipaddress.ip_address(host or "").is_loopback
        except ValueError:
            is_loopback = False
        if not is_loopback:
            raise EvidenceError("remote RPC refused to protect node credentials")
        self.url = url
        self.auth = "Basic " + base64.b64encode(credential.encode("utf-8")).decode("ascii")
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def call(self, method: str, params: list | None = None):
        if method not in {"getblockchaininfo", "getblockcount", "getblockhash", "getblock", "getrawtransaction"}:
            raise EvidenceError("unsupported RPC method")
        data = json.dumps({"jsonrpc": "1.0", "id": "tradelayer-evidence", "method": method, "params": params or []}).encode()
        req = urllib.request.Request(self.url, data=data, headers={"Content-Type": "application/json", "Authorization": self.auth}, method="POST")
        try:
            with self.opener.open(req, timeout=30) as resp:
                payload = resp.read(MAX_RESPONSE_BYTES + 1)
        except (urllib.error.URLError, OSError) as err:
            raise EvidenceError(f"local RPC failed ({method}); verify node and credentials") from err
        if len(payload) > MAX_RESPONSE_BYTES:
            raise EvidenceError("RPC response too large; use a smaller block range")
        try:
            response = json.loads(payload)
        except (ValueError, UnicodeDecodeError) as err:
            raise EvidenceError("invalid RPC response") from err
        if not isinstance(response, dict) or response.get("error") is not None:
            raise EvidenceError(f"RPC error in {method}; inspect your node logs")
        return response.get("result")


def scan_blocks(rpc, network: str, start: int, end: int, confirmations: int = 6) -> tuple[str, dict]:
    if start < 0 or end < start or end - start + 1 > MAX_BLOCKS:
        raise EvidenceError(f"invalid height interval (maximum {MAX_BLOCKS} blocks per run)")
    if confirmations < 1:
        raise EvidenceError("at least one confirmation required")
    info = rpc.call("getblockchaininfo")
    if not isinstance(info, dict) or info.get("chain") != network:
        raise EvidenceError("RPC reports a different chain than requested")
    if info.get("pruned") or info.get("initialblockdownload"):
        raise EvidenceError("an unpruned, fully synced Litecoin Core node is required")
    original_tip = rpc.call("getblockcount")
    if type(original_tip) is not int or end + confirmations > original_tip + 1:
        raise EvidenceError("selected blocks lack requested confirmations")
    anchor = rpc.call("getblockhash", [start - 1]) if start else None
    if anchor is not None:
        anchor = require_hash(anchor, "prior block hash")
    block_hashes = {}
    rows = ["# label\tnetwork\treported_height\treported_block_hash\texpected_txid\traw_transaction_hex"]
    evidence = []
    chain_label = {"main": "litecoin-mainnet", "test": "litecoin-testnet", "regtest": "litecoin-regtest"}[network]
    parent = anchor
    for height in range(start, end + 1):
        block_hash = require_hash(rpc.call("getblockhash", [height]), "canonical block hash")
        block = rpc.call("getblock", [block_hash, 2])
        if not isinstance(block, dict) or require_hash(block.get("hash"), "block hash") != block_hash:
            raise EvidenceError("RPC block hash mismatch")
        if type(block.get("height")) is not int or block["height"] != height:
            raise EvidenceError("RPC block height mismatch")
        if not isinstance(block.get("confirmations"), int) or block["confirmations"] < confirmations:
            raise EvidenceError("block no longer sufficiently confirmed")
        if parent is not None and require_hash(block.get("previousblockhash"), "parent") != parent:
            raise EvidenceError("block chain parent mismatch")
        txs = block.get("tx")
        if not isinstance(txs, list) or not txs or not all(isinstance(tx, dict) for tx in txs):
            raise EvidenceError("getblock verbosity=2 requires decoded transactions")
        txids = [require_hash(tx.get("txid"), "txid") for tx in txs]
        if merkle_root(txids) != require_hash(block.get("merkleroot"), "block Merkle root"):
            raise EvidenceError("block Merkle root mismatch")
        selected = []
        for index, tx in enumerate(txs):
            carriers = selected_candidates(tx)
            if not carriers:
                continue
            txid = txids[index]
            raw_hex = rpc.call("getrawtransaction", [txid, False, block_hash])
            derived, scripts = inspect_raw_transaction(raw_hex)
            if derived != txid:
                raise EvidenceError("raw txid differs from node's block txid")
            if isinstance(tx.get("hex"), str) and tx["hex"].lower() != raw_hex.lower():
                raise EvidenceError("node returned inconsistent transaction serializations")
            for vout, payload, decoded_script in carriers:
                if vout >= len(scripts) or scripts[vout] != decoded_script:
                    raise EvidenceError("output script differs from raw transaction bytes")
            selected.append({"txid": txid, "index": index, "raw_hex": raw_hex.lower(),
                             "outputs": [{"vout": n, "payload_hex": p.hex(), "script_hex": s} for n, p, s in carriers]})
            if len(evidence) + len(selected) > MAX_CANDIDATES:
                raise EvidenceError("candidate count cap reached")
        for record in selected:
            label = f"ltc-h{height}-i{record['index']}"
            rows.append("\t".join([label, chain_label, str(height), block_hash, record["txid"], record["raw_hex"]]))
            evidence.append({"label": label, "height": height, "blockhash": block_hash,
                             "transaction_position": record["index"], "txid": record["txid"],
                             "outputs": record["outputs"]})
        block_hashes[height] = block_hash
        parent = block_hash
    # Recheck the full observed range in the node's active chain, detecting
    # reorgs before files are published, including changes inside the interval.
    for height, original in block_hashes.items():
        if require_hash(rpc.call("getblockhash", [height]), "final canonical block hash") != original:
            raise EvidenceError("chain reorganized during scan; rerun on the new branch")
    manifest = {
        "format": "tradelayer-archive-evidence-v1",
        "network": chain_label,
        "source": "local-unpruned-litecoin-core-json-rpc",
        "consensus_verified_by": "local node; not independently revalidated by this collector",
        "merkle_root_checked": True,
        "raw_transaction_txids_checked": True,
        "raw_output_scripts_checked": True,
        "canonical_range_rechecked": True,
        "reported_start_tip_height": original_tip,
        "start_height": start,
        "end_height": end,
        "min_confirmations": confirmations,
        "previous_block_hash": anchor,
        "last_block_hash": parent,
        "blocks_scanned": len(block_hashes),
        "transactions_selected": len(evidence),
        "transactions": evidence,
    }
    return "\n".join(rows) + "\n", manifest


def atomic_write(path: Path, data: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".tradelayer-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def default_cookie_file(network: str, home: Path | None = None) -> Path:
    """Litecoin Core's network-specific default RPC cookie location."""
    if network not in NETWORK_PORTS:
        raise EvidenceError("unsupported Litecoin network")
    directory = {"main": "", "test": "testnet3", "regtest": "regtest"}[network]
    return (home if home is not None else Path.home()) / ".litecoin" / directory / ".cookie"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=tuple(NETWORK_PORTS), required=True)
    parser.add_argument("--start-height", type=int, required=True)
    parser.add_argument("--end-height", type=int, required=True)
    parser.add_argument("--min-confirmations", type=int, default=6)
    parser.add_argument("--rpc-url", help="loopback JSON-RPC endpoint only")
    parser.add_argument("--cookie-file", type=Path, help="local Litecoin Core auth cookie")
    parser.add_argument("--output", required=True, type=Path, help="output TSV path")
    parser.add_argument("--manifest", required=True, type=Path, help="provenance JSON file")
    args = parser.parse_args(argv)
    if args.output.resolve() == args.manifest.resolve():
        parser.error("output and manifest must have different paths")
    try:
        user, password = os.getenv("TL_RPC_USER"), os.getenv("TL_RPC_PASSWORD")
        if user is not None and password is not None:
            credential = f"{user}:{password}"
        else:
            cookie = args.cookie_file or default_cookie_file(args.network)
            credential = cookie.read_text(encoding="utf-8").strip()
            if credential.count(":") < 1:
                raise EvidenceError("RPC cookie has no username/password separator")
        rpc = LitecoinRPC(args.rpc_url or f"http://127.0.0.1:{NETWORK_PORTS[args.network]}", credential)
        corpus, provenance = scan_blocks(rpc, args.network, args.start_height, args.end_height, args.min_confirmations)
        provenance["corpus_sha256"] = hashlib.sha256(corpus.encode("utf-8")).hexdigest()
        # Publish only when the scan has fully succeeded. Keep source artifacts
        # separate so malformed provenance cannot be mistaken for signed state.
        atomic_write(args.output, corpus)
        atomic_write(args.manifest, json.dumps(provenance, indent=2, sort_keys=True) + "\n")
        print(f"Scanned {provenance['blocks_scanned']} blocks, selected {provenance['transactions_selected']} candidates")
        return 0
    except (EvidenceError, OSError) as err:
        print(f"collection failed: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
