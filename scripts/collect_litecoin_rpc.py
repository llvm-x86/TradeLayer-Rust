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
# Read-only allowlist; no wallet, signing, broadcast, peer or config methods.
READ_ONLY_METHODS = frozenset({"getblockchaininfo", "getblockcount", "getblockhash", "getblock",
                               "getblockheader", "getrawtransaction", "getnetworkinfo",
                               "getindexinfo", "validateaddress"})


class EvidenceError(Exception):
    pass


class RPCResponseError(EvidenceError):
    """The node answered with a JSON-RPC error object; transport loss is NOT this."""
    def __init__(self, message: str, code: object):
        super().__init__(message)
        self.code = code


class RPCMethodNotFound(RPCResponseError):
    """JSON-RPC -32601: the node predates the method."""


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


def merkle_branch(txids: list[str], index: int) -> list[str]:
    """Sibling hashes (display order) proving txids[index] under the root."""
    if not 0 <= index < len(txids):
        raise EvidenceError("Merkle branch index outside block")
    level = [bytes.fromhex(require_hash(s, "Merkle leaf"))[::-1] for s in txids]
    branch = []
    while len(level) > 1:
        if len(level) % 2:
            level.append(level[-1])
        branch.append(level[index ^ 1][::-1].hex())
        level = [sha256d(level[n] + level[n + 1]) for n in range(0, len(level), 2)]
        index //= 2
    return branch


def root_from_branch(txid: str, branch: list[str], index: int) -> str:
    node = bytes.fromhex(require_hash(txid, "Merkle leaf"))[::-1]
    for sibling in branch:
        other = bytes.fromhex(require_hash(sibling, "Merkle sibling"))[::-1]
        node = sha256d(other + node) if index & 1 else sha256d(node + other)
        index //= 2
    if index != 0:
        raise EvidenceError("Merkle branch too short for position")
    return node[::-1].hex()


def verify_merkle_position(txid: str, branch: list[str], index: int, tx_count: int,
                           root: str) -> None:
    """Bind a canonical block position, not just membership, to the root.

    The last element of an odd level is paired with itself, so a branch alone
    does not fix position. Every self-pairing must occur at the right edge of
    a level whose width follows from tx_count (as reported by the node).
    tx_count itself is not committed by the header: for a leaf that never
    self-pairs, a duplicated-tail mutation (CVE-2012-2459 shape) yields the
    same root and the same position, so position remains canonical.
    """
    if type(tx_count) is not int or type(index) is not int or not 0 <= index < tx_count:
        raise EvidenceError("Merkle position outside block")
    expected_depth = (tx_count - 1).bit_length()
    if len(branch) != expected_depth:
        raise EvidenceError("Merkle branch depth does not match transaction count")
    node, position, width = require_hash(txid, "Merkle leaf"), index, tx_count
    for sibling in branch:
        if require_hash(sibling, "Merkle sibling") == node:
            if position % 2 or position != width - 1:
                raise EvidenceError("self-paired Merkle node away from the right edge")
        elif position % 2 == 0 and position == width - 1:
            raise EvidenceError("right-edge Merkle node must pair with itself")
        node = root_from_branch(node, [sibling], position % 2)
        position //= 2
        width = (width + 1) // 2
    if node != root:
        raise EvidenceError("Merkle branch does not reach the header root")


def parse_header(header_hex: str) -> dict:
    """80-byte Bitcoin-family header; identity hash is SHA256d (PoW is Scrypt)."""
    if not isinstance(header_hex, str) or len(header_hex) != 160:
        raise EvidenceError("block header must be exactly 80 bytes")
    try:
        raw = bytes.fromhex(header_hex)
    except ValueError as err:
        raise EvidenceError("malformed block header hex") from err
    return {"hash": sha256d(raw)[::-1].hex(), "version": int.from_bytes(raw[0:4], "little"),
            "prev": raw[4:36][::-1].hex(), "merkleroot": raw[36:68][::-1].hex(),
            "time": int.from_bytes(raw[68:72], "little"),
            "bits": int.from_bytes(raw[72:76], "little"),
            "nonce": int.from_bytes(raw[76:80], "little")}


def checked_header(rpc, block_hash: str, parent: str | None, merkle: str) -> str:
    """Bind the node's block hash, parent link and Merkle root to raw header bytes."""
    header_hex = rpc.call("getblockheader", [block_hash, False])
    if not isinstance(header_hex, str):
        raise EvidenceError("invalid raw block header")
    header = parse_header(header_hex.lower())
    if header["hash"] != block_hash:
        raise EvidenceError("block header bytes do not hash to the block hash")
    if parent is not None and header["prev"] != parent:
        raise EvidenceError("block header parent mismatch")
    if header["merkleroot"] != merkle:
        raise EvidenceError("block header Merkle root mismatch")
    return header_hex.lower()


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


MWEB_HOGEX_NULL = b"\x00"


def parse_transaction(hex_value: str) -> dict:
    """Parse canonical inputs/outputs; txid excludes witness and MWEB data.

    Litecoin Core (src/primitives/transaction.h at ec1b6489) adds flag 0x08
    for MWEB data between the witness and nLockTime. A HogEx transaction
    carries a null MWEB::Tx (a single 0x00 byte). Other MWEB payloads stay
    opaque: only the canonical part is authenticated through the txid, and an
    MWEB-only transaction has no canonical txid preimage, so it is refused.
    """
    if not isinstance(hex_value, str) or len(hex_value) > MAX_TRANSACTION_BYTES * 2:
        raise EvidenceError("transaction exceeds size limit")
    try:
        raw = bytes.fromhex(hex_value)
    except ValueError as err:
        raise EvidenceError("malformed transaction hex") from err
    cursor = Cursor(raw)
    version = cursor.read(4)
    flags = 0
    if len(raw) > 5 and raw[4] == 0:
        flags = raw[5]
        if flags == 0 or flags & ~0x09:
            raise EvidenceError("unknown transaction serialization flags")
        cursor.read(2)
    start = cursor.index
    vin_count = cursor.number()
    if not 1 <= vin_count <= 100_000:
        raise EvidenceError("invalid input count")
    inputs = []
    for _ in range(vin_count):
        prev = cursor.read(32)[::-1].hex()
        inputs.append((prev, int.from_bytes(cursor.read(4), "little")))
        cursor.var_bytes()
        cursor.read(4)
    output_count = cursor.number()
    if not 1 <= output_count <= 100_000:
        raise EvidenceError("invalid output count")
    outputs = []
    for _ in range(output_count):
        satoshis = int.from_bytes(cursor.read(8), "little")
        outputs.append((satoshis, cursor.var_bytes().hex()))
    end = cursor.index
    if flags & 1:
        nonempty = False
        for _ in range(vin_count):
            witnesses = cursor.number()
            if witnesses > 100_000:
                raise EvidenceError("excessive witness count")
            for _ in range(witnesses):
                nonempty |= bool(cursor.var_bytes())
        if not nonempty:
            raise EvidenceError("superfluous SegWit witness")
    mweb = None
    if flags & 8:
        if len(raw) - cursor.index < 5:
            raise EvidenceError("truncated MWEB transaction data")
        blob = cursor.read(len(raw) - cursor.index - 4)
        if blob == MWEB_HOGEX_NULL:
            mweb = "hogex"
        elif blob[0] == 1:
            mweb = "opaque"
        else:
            raise EvidenceError("invalid MWEB optional-pointer tag")
    locktime = cursor.read(4)
    if cursor.index != len(raw):
        raise EvidenceError("trailing transaction bytes")
    stripped = version + raw[start:end] + locktime
    return {"txid": sha256d(stripped)[::-1].hex(), "inputs": inputs,
            "outputs": outputs, "witness": bool(flags & 1), "mweb": mweb}


def inspect_raw_transaction(hex_value: str) -> tuple[str, list[str]]:
    """Compute txid and exact vout scripts; handle SegWit and MWEB flags."""
    parsed = parse_transaction(hex_value)
    return parsed["txid"], [script for _, script in parsed["outputs"]]


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


LEGACY_JS_MARKER_HEX = "746c"


def legacy_js_carrier(tx: dict) -> dict | None:
    """Selection predicate of txIndex.DecodeRawTransaction at the pinned JS.

    The JS examines only the FIRST output that the node decodes as
    'nulldata', then searches its script hex for "746c" at nibble offsets
    0..6, including odd offsets and with no push-length check. Printability
    and payload decoding are left to the pinned JS harness; this only widens
    collection so that JS-selected transactions are not silently dropped.
    """
    for position, output in enumerate(tx["vout"]):
        script = output.get("scriptPubKey")
        if isinstance(script, dict) and script.get("type") == "nulldata":
            script_hex = script.get("hex")
            if not isinstance(script_hex, str):
                raise EvidenceError("missing output script")
            marker = script_hex.find(LEGACY_JS_MARKER_HEX)
            if marker == -1 or marker > 6:
                return None
            return {"vout": position, "marker_nibble": marker,
                    "script_hex": script_hex.lower()}
    return None


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
        if method not in READ_ONLY_METHODS:
            raise EvidenceError("unsupported RPC method")
        data = json.dumps({"jsonrpc": "1.0", "id": "tradelayer-evidence", "method": method, "params": params or []}).encode()
        req = urllib.request.Request(self.url, data=data, headers={"Content-Type": "application/json", "Authorization": self.auth}, method="POST")
        http_error = False
        try:
            with self.opener.open(req, timeout=30) as resp:
                payload = resp.read(MAX_RESPONSE_BYTES + 1)
        except urllib.error.HTTPError as err:
            # Core reports JSON-RPC errors as HTTP 400/404/500 with a JSON body;
            # 401/403 (credentials) and anything else stay transport failures.
            if err.code not in (400, 404, 500):
                raise EvidenceError(f"local RPC failed ({method}); verify node and credentials") from err
            payload = err.read(MAX_RESPONSE_BYTES + 1)
            http_error = True
        except (urllib.error.URLError, OSError) as err:
            raise EvidenceError(f"local RPC failed ({method}); verify node and credentials") from err
        if len(payload) > MAX_RESPONSE_BYTES:
            raise EvidenceError("RPC response too large; use a smaller block range")
        try:
            response = json.loads(payload)
        except (ValueError, UnicodeDecodeError) as err:
            raise EvidenceError("invalid RPC response") from err
        if not isinstance(response, dict):
            raise EvidenceError("invalid RPC response")
        error = response.get("error")
        if error is not None:
            code = error.get("code") if isinstance(error, dict) else None
            if code == -32601:
                raise RPCMethodNotFound(f"RPC method {method} not available on this node", code)
            raise RPCResponseError(f"RPC error in {method}; inspect your node logs", code)
        if http_error:
            raise EvidenceError(f"local RPC failed ({method}); HTTP error without RPC error object")
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
    headers = {}
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
        if len(txids) != len(set(txids)):
            raise EvidenceError("duplicate txid in block")
        root = merkle_root(txids)
        if root != require_hash(block.get("merkleroot"), "block Merkle root"):
            raise EvidenceError("block Merkle root mismatch")
        headers[str(height)] = checked_header(rpc, block_hash, parent, root)
        selected = []
        for index, tx in enumerate(txs):
            carriers = selected_candidates(tx)
            legacy = legacy_js_carrier(tx)
            if not carriers and legacy is None:
                continue
            txid = txids[index]
            raw_hex = rpc.call("getrawtransaction", [txid, False, block_hash])
            derived, scripts = inspect_raw_transaction(raw_hex)
            if derived != txid:
                raise EvidenceError("raw txid differs from node's block txid")
            if isinstance(tx.get("hex"), str) and tx["hex"].lower() != raw_hex.lower():
                raise EvidenceError("node returned inconsistent transaction serializations")
            checked = [(vout, script) for vout, _, script in carriers]
            if legacy is not None:
                checked.append((legacy["vout"], legacy["script_hex"]))
            for vout, decoded_script in checked:
                if vout >= len(scripts) or scripts[vout] != decoded_script:
                    raise EvidenceError("output script differs from raw transaction bytes")
            selected.append({"txid": txid, "index": index, "raw_hex": raw_hex.lower(),
                             "outputs": [{"vout": n, "payload_hex": p.hex(), "script_hex": s} for n, p, s in carriers],
                             "legacy_js_carrier": legacy})
            if len(evidence) + len(selected) > MAX_CANDIDATES:
                raise EvidenceError("candidate count cap reached")
        for record in selected:
            label = f"ltc-h{height}-i{record['index']}"
            rows.append("\t".join([label, chain_label, str(height), block_hash, record["txid"], record["raw_hex"]]))
            evidence.append({"label": label, "height": height, "blockhash": block_hash,
                             "transaction_position": record["index"], "txid": record["txid"],
                             "merkle_branch": merkle_branch(txids, record["index"]),
                             "block_tx_count": len(txids),
                             "outputs": record["outputs"],
                             "legacy_js_carrier": record["legacy_js_carrier"],
                             "carrier_class": carrier_class(record)})
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
        "selection_rules": ["rust-strict-canonical-op-return-tl-or-TLR",
                            "legacy-js-first-nulldata-746c-nibble-0-6"],
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
        "block_headers": headers,
        "transactions_selected": len(evidence),
        "transactions": evidence,
    }
    return "\n".join(rows) + "\n", manifest


def carrier_class(record: dict) -> str:
    """Which selection rule(s) admitted the transaction; never a validity verdict."""
    strict = bool(record["outputs"])
    legacy = record["legacy_js_carrier"] is not None
    if strict and legacy:
        return "strict-and-legacy-js"
    return "strict-only" if strict else "legacy-js-only"


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
    # Litecoin Core src/chainparamsbase.cpp at ec1b6489: testnet uses "testnet4".
    directory = {"main": "", "test": "testnet4", "regtest": "regtest"}[network]
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
