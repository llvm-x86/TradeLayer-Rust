#!/usr/bin/env python3
"""Stage 8D read-only Bitcoin Core bulk-vs-per-TX RPC measurement.

Compare GETBLOCK(hash, 0) against N GETRAWTRANSACTION(txid, false, hash)
calls on ONE local Bitcoin Core node. Time *RPC fetch only*, not the local
txid/byte audit. The initial verbose block read and preflight fetch mean
the program CANNOT claim cold-cache conditions.

All traffic is explicit read-only JSON-RPC to 127.0.0.1 through one Python
process, without spawning bitcoin-cli for every TX. Network responses and
their raw transactions remain UNTRUSTED, irrespective of Core's report.
This has no wallet, script execution, broadcast, chain-state, or TL authority.

Run with:
  python3 scripts/bench_bitcoin_core_rpc_ingest.py --height 850000 \
      --cookie-file ~/.bitcoin/.cookie --repeats 3 --warmups 1
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import math
from pathlib import Path
import re
import statistics
import sys
import time
from typing import Any, Callable

MAX_BLOCK_BYTES = 4_000_000
MAX_TRANSACTION_BYTES = 1_000_000  # research scanner limit; not consensus
MAX_TRANSACTIONS = 100_000
MAX_VECTOR_ITEMS = 100_000
MAX_RESPONSE_BYTES = 2 * MAX_BLOCK_BYTES + 1_000_000
HASH_RE = re.compile(r"[0-9a-fA-F]{64}\Z")
HEX_RE = re.compile(r"[0-9a-fA-F]*\Z")
READ_ONLY = frozenset({
    "getblockchaininfo", "getnetworkinfo", "getblockhash", "getblock",
    "getrawtransaction",
})


class IngestBenchmarkError(ValueError):
    """Fail-closed bad RPC data, malformed raw bytes or unsafe config."""


def require(condition: bool, explanation: str) -> None:
    if not condition:
        raise IngestBenchmarkError(explanation)


def sha256d(value: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(value).digest()).digest()


def hash_display(value: bytes) -> str:
    require(type(value) is bytes and len(value) == 32, "hash is not 32 raw bytes")
    return value[::-1].hex()


def exact_hash(value: Any, label: str) -> str:
    require(type(value) is str and HASH_RE.fullmatch(value) is not None,
            f"{label} must be exactly 64 hexadecimal characters")
    return value.lower()


def raw_hex(value: Any, limit: int, label: str) -> bytes:
    require(type(value) is str and len(value) % 2 == 0
            and 0 < len(value) <= 2 * limit and HEX_RE.fullmatch(value) is not None,
            f"{label} is not bounded raw hexadecimal data")
    return bytes.fromhex(value)


class Reader:
    def __init__(self, raw: bytes, offset: int = 0):
        require(type(raw) is bytes and type(offset) is int
                and 0 <= offset <= len(raw), "invalid read cursor")
        self.raw = raw
        self.offset = offset

    def take(self, count: int) -> bytes:
        require(type(count) is int and 0 <= count <= len(self.raw) - self.offset,
                "truncated raw block or transaction")
        left = self.offset
        self.offset += count
        return self.raw[left:self.offset]

    def u(self, width: int) -> int:
        return int.from_bytes(self.take(width), "little")

    def compact(self, maximum: int = MAX_VECTOR_ITEMS) -> int:
        prefix = self.u(1)
        if prefix < 253:
            count = prefix
        elif prefix == 253:
            count = self.u(2)
            require(count >= 253, "noncanonical CompactSize 16-bit")
        elif prefix == 254:
            count = self.u(4)
            require(count >= 65536, "noncanonical CompactSize 32-bit")
        else:
            count = self.u(8)
            require(count >= 4294967296, "noncanonical CompactSize 64-bit")
        require(count <= maximum, "CompactSize exceeds bounded research limit")
        return count

    def blob(self, maximum: int = MAX_TRANSACTION_BYTES) -> bytes:
        return self.take(self.compact(maximum))


def read_tx(reader: Reader) -> tuple[bytes, str, str]:
    """Extract exact raw transaction bytes and Bitcoin txid/wtxid.

    This is a bounded syntactic parser, NOT a Bitcoin consensus parser.
    Supports base + BIP144 flag 0x01 only, as required by the Rust preindexer.
    """
    start = reader.offset
    version = reader.take(4)
    segwit = reader.offset < len(reader.raw) and reader.raw[reader.offset] == 0
    if segwit:
        require(reader.take(1) == b"\x00" and reader.take(1) == b"\x01",
                "unsupported SegWit optional transaction flags")
    body_start = reader.offset
    vin_count = reader.compact()
    require(1 <= vin_count <= MAX_VECTOR_ITEMS, "canonical transaction needs inputs")
    for _ in range(vin_count):
        reader.take(32 + 4)
        reader.blob()
        reader.take(4)
    vout_count = reader.compact()
    require(1 <= vout_count <= MAX_VECTOR_ITEMS, "canonical transaction needs outputs")
    for _ in range(vout_count):
        reader.take(8)
        reader.blob()
    body_end = reader.offset
    if segwit:
        nonempty = False
        for _ in range(vin_count):
            n = reader.compact()
            nonempty |= n != 0
            for _ in range(n):
                reader.blob()
        require(nonempty, "superfluous SegWit serialization")
    locktime = reader.take(4)
    require(reader.offset - start <= MAX_TRANSACTION_BYTES,
            "transaction exceeds scanner's research byte limit")
    encoded = reader.raw[start:reader.offset]
    stripped = (version + reader.raw[body_start:body_end] + locktime
                if segwit else encoded)
    txid = sha256d(stripped)
    wtxid = sha256d(encoded) if segwit else txid
    return encoded, hash_display(txid), hash_display(wtxid)


def merkle_root(txids: list[str]) -> bytes:
    """Recompute Bitcoin SHA256d tree; reject equal real siblings, not padding."""
    require(1 <= len(txids) <= MAX_TRANSACTIONS, "invalid Merkle leaf count")
    nodes = [bytes.fromhex(exact_hash(txid, "Merkle txid"))[::-1] for txid in txids]
    while len(nodes) > 1:
        require(all(nodes[i] != nodes[i + 1] for i in range(0, len(nodes) - 1, 2)),
                "mutated Merkle tree with equal real siblings")
        if len(nodes) & 1:
            nodes.append(nodes[-1])
        nodes = [sha256d(nodes[i] + nodes[i + 1])
                 for i in range(0, len(nodes), 2)]
    return nodes[0]


def parse_raw_block(raw: bytes) -> tuple[str, list[tuple[bytes, str, str]]]:
    require(type(raw) is bytes and 81 <= len(raw) <= MAX_BLOCK_BYTES,
            "raw block exceeds research input bounds")
    reader = Reader(raw)
    header = reader.take(80)
    count = reader.compact(MAX_TRANSACTIONS)
    require(count > 0, "block has no transactions")
    transactions = [read_tx(reader) for _ in range(count)]
    require(reader.offset == len(raw), "trailing block serialization bytes")
    require(merkle_root([txid for _, txid, _ in transactions]) == header[36:68],
            "local canonical TXID Merkle root differs from header")
    return hash_display(sha256d(header)), transactions


class BitcoinCoreRpc:
    """Persistent-process loopback HTTP client; no user credentials in logs."""

    def __init__(self, cookie_file: Path, *, port: int = 8332, timeout: float = 20):
        require(type(port) is int and 1 <= port <= 65535, "invalid RPC port")
        require(type(timeout) in (int, float) and math.isfinite(timeout)
                and 1 <= timeout <= 120, "invalid RPC timeout")
        require(cookie_file.is_file() and not cookie_file.is_symlink(),
                "Bitcoin Core cookie file must be a regular local file")
        auth = cookie_file.read_bytes().rstrip(b"\r\n")
        require(auth.startswith(b"__cookie__:") and b"\n" not in auth
                and len(auth) <= 256, "Bitcoin Core cookie is malformed")
        self._connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
        self._auth = "Basic " + base64.b64encode(auth).decode("ascii")
        self.calls = 0
        self.response_body_bytes = 0

    def close(self) -> None:
        self._connection.close()

    def __enter__(self) -> "BitcoinCoreRpc":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __call__(self, method: str, *params: Any) -> Any:
        require(method in READ_ONLY, "non-read-only Bitcoin RPC method rejected")
        request = json.dumps(
            {"jsonrpc": "1.0", "id": "readonly-bench", "method": method, "params": params},
            separators=(",", ":"),
        ).encode("ascii")
        self.calls += 1
        try:
            self._connection.request(
                "POST", "/", body=request, headers={
                    "Authorization": self._auth, "Content-Type": "application/json",
                },
            )
            response = self._connection.getresponse()
            raw = response.read(MAX_RESPONSE_BYTES + 1)
        except (OSError, TimeoutError, http.client.HTTPException) as exc:
            raise IngestBenchmarkError(f"Bitcoin Core {method} transport failed") from exc
        self.response_body_bytes += len(raw)
        require(response.status == 200, f"Bitcoin Core {method} HTTP status {response.status}")
        require(len(raw) <= MAX_RESPONSE_BYTES,
                f"Bitcoin Core {method} response exceeds limit")
        try:
            doc = json.loads(raw)
        except (UnicodeError, ValueError) as exc:
            raise IngestBenchmarkError(f"Bitcoin Core {method} malformed JSON response") from exc
        require(type(doc) is dict and doc.get("error") is None and "result" in doc,
                f"Bitcoin Core {method} RPC response is an error")
        return doc["result"]


def _timed_call(rpc: Callable[..., Any], method: str, *args: Any) -> tuple[Any, float]:
    start = time.perf_counter_ns()
    result = rpc(method, *args)
    return result, (time.perf_counter_ns() - start) / 1e6


def _median(values: list[float]) -> float:
    require(bool(values), "no timing samples recorded")
    return statistics.median(values)


def benchmark(
    rpc: Callable[..., Any],
    *,
    height: int,
    repeats: int = 3,
    warmups: int = 0,
    max_transactions: int = 10_000,
) -> dict[str, Any]:
    """Run read-only paths; audit bytes AFTER stopping each RPC stopwatch."""
    require(type(height) is int and height >= 0, "height must be nonnegative")
    require(type(repeats) is int and 1 <= repeats <= 7, "repeats must be 1..7")
    require(type(warmups) is int and 0 <= warmups <= 3, "warmups must be 0..3")
    require(type(max_transactions) is int and 1 <= max_transactions <= MAX_TRANSACTIONS,
            "invalid maximum transaction count")

    info = rpc("getblockchaininfo")
    require(type(info) is dict and info.get("chain") == "main"
            and info.get("initialblockdownload") is False, "requires synced Bitcoin mainnet")
    tip = info.get("blocks")
    require(type(tip) is int and tip >= height, "node active height insufficient")
    require(type(info.get("headers")) is int and info["headers"] >= tip,
            "node headers height is inconsistent")
    net = rpc("getnetworkinfo")
    require(type(net) is dict and type(net.get("version")) is int
            and net["version"] > 0, "Bitcoin Core version not reported")
    blockhash = exact_hash(rpc("getblockhash", height), "active block hash")
    verbose = rpc("getblock", blockhash, 1)
    require(type(verbose) is dict
            and exact_hash(verbose.get("hash"), "verbose block hash") == blockhash
            and type(verbose.get("height")) is int and verbose["height"] == height
            and type(verbose.get("confirmations")) is int
            and verbose["confirmations"] >= 1, "block metadata not active/consistent")
    listed = verbose.get("tx")
    require(type(listed) is list and 1 <= len(listed) <= max_transactions,
            "block TX count exceeds benchmark cap")
    expected = [exact_hash(txid, "verbose txid") for txid in listed]
    require(len(expected) == len(set(expected)), "duplicate verbose transaction ID")

    # Preflight validates genuine raw bytes before any expensive N-RPC rounds.
    # This call may warm Core's caches: NEVER present later samples as cold.
    baseline_hex, preflight_bulk_ms = _timed_call(rpc, "getblock", blockhash, 0)
    original = raw_hex(baseline_hex, MAX_BLOCK_BYTES, "raw getblock response")
    derived_blockhash, parsed = parse_raw_block(original)
    require(derived_blockhash == blockhash, "raw block header hash differs from active RPC ID")
    require([txid for _, txid, _ in parsed] == expected,
            "verbose transaction IDs disagree with parsed raw block")

    bulk_ms: list[float] = []
    per_tx_ms: list[float] = []
    n = len(expected)
    for iteration in range(warmups + repeats):
        # Alternate which path gets first access to warm caches each round.
        paths = ("bulk", "per_tx") if iteration % 2 == 0 else ("per_tx", "bulk")
        measured: dict[str, tuple[float, Any]] = {}
        for path in paths:
            if path == "bulk":
                wire, elapsed = _timed_call(rpc, "getblock", blockhash, 0)
            else:
                start = time.perf_counter_ns()
                wire = [rpc("getrawtransaction", txid, False, blockhash)
                        for txid in expected]
                elapsed = (time.perf_counter_ns() - start) / 1e6
            measured[path] = (elapsed, wire)
        # Integrity checks happen after both RPC stopwatches have stopped.
        bulk_raw = raw_hex(measured["bulk"][1], MAX_BLOCK_BYTES, "bulk response")
        require(bulk_raw == original, "bulk response bytes changed between iterations")
        individual = measured["per_tx"][1]
        require(type(individual) is list and len(individual) == n,
                "per-TX response count mismatch")
        for position, (wire, expected_tx) in enumerate(zip(individual, parsed)):
            require(raw_hex(wire, MAX_TRANSACTION_BYTES, f"transaction {position}")
                    == expected_tx[0], f"individual transaction {position} differs from bulk")
        if iteration >= warmups:
            bulk_ms.append(measured["bulk"][0])
            per_tx_ms.append(measured["per_tx"][0])

    bulk_med = _median(bulk_ms)
    per_tx_med = _median(per_tx_ms)
    return {
        "status": "bitcoin_core_rpc_ingestion_benchmark_unverified",
        "network": "main",
        "block_height_claim": height,
        "block_hash_claim": blockhash,
        "node_tip_height_at_start": tip,
        "bitcoin_core_version_reported": net["version"],
        "transaction_count": n,
        "segwit_transaction_count": sum(a != b for _, a, b in parsed),
        "raw_block_bytes": len(original),
        "preflight_bulk_fetch_ms": round(preflight_bulk_ms, 3),
        "bulk_rpc_fetch_only_median_ms": round(bulk_med, 3),
        "per_tx_rpc_fetch_only_median_ms": round(per_tx_med, 3),
        "per_tx_over_bulk_fetch_ratio": round(per_tx_med / max(bulk_med, 1e-9), 3),
        "bulk_rpc_calls_per_sample": 1,
        "individual_rpc_calls_per_sample": n,
        "benchmark_warmups": warmups,
        "benchmark_repeats": repeats,
        "first_path_alternates": True,
        "individual_byte_equal_to_bulk": True,
        "hashes_and_merkle_locally_consistent": True,
        "preflight_makes_cache_cold_claim_invalid": True,
        "remote_node_network_latency": "not_separately_measured",
        "bitcoin_script_utxo_consensus": "UNVERIFIED",
        "tradelayer_state": "UNVERIFIED",
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--height", type=int, required=True, help="Mainnet block height")
    parser.add_argument("--cookie-file", type=Path, required=True,
                        help="Local Bitcoin Core RPC auth cookie (never printed)")
    parser.add_argument("--port", type=int, default=8332, help="Loopback RPC port")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--warmups", type=int, default=0)
    parser.add_argument("--max-transactions", type=int, default=10_000)
    parser.add_argument("--timeout", type=float, default=20.0)
    args = parser.parse_args(argv)
    try:
        with BitcoinCoreRpc(args.cookie_file, port=args.port, timeout=args.timeout) as rpc:
            result = benchmark(
                rpc, height=args.height, repeats=args.repeats,
                warmups=args.warmups, max_transactions=args.max_transactions,
            )
            result["rpc_calls_total"] = rpc.calls
            result["rpc_response_body_bytes_total"] = rpc.response_body_bytes
    except (IngestBenchmarkError, OSError, ValueError) as exc:
        print(f"status=bitcoin_core_rpc_ingestion_unverified; reason={exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
