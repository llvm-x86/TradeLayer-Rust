#!/usr/bin/env python3
"""Bounded Bitcoin Core RPC -> Rust read-only candidate index stream.

BLOCKS uses batched getblockhash / getblock(hash, 0), avoiding N per-TX RPCs.
SCRIPT uses bitcoin/bitcoin#36629 getscripthistory for exact scripts only.
Requires a locally reachable Bitcoin Core RPC with an explicitly supplied
cookie file. No fallback silently converts targeted history to global history.
This is discovery from one node, never independent consensus authentication.
"""
import argparse
import base64
import http.client
import json
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlsplit

MAX_BLOCK_BYTES = 4_000_000
MAX_HTTP_BYTES = 128 * 1024 * 1024
MAX_BATCH = 8
MAX_SCRIPT_RECORDS = 200_000


class RPCError(Exception):
    def __init__(self, code, message):
        super().__init__(f"Bitcoin Core RPC {code}: {message}")
        self.code = code
        self.message = message


class IncompleteHistory(Exception):
    pass


class BitcoinCoreRPC:
    """Persistent standard-library JSON RPC client; batch replies keyed by ID."""

    def __init__(self, url, cookie_file, timeout=120):
        target = urlsplit(url)
        if (target.scheme != "http" or target.hostname not in
                ("127.0.0.1", "localhost", "::1") or target.username or
                target.password or target.query or target.fragment):
            raise ValueError("RPC requires loopback HTTP; use a local SSH tunnel for remote nodes")
        if not target.port:
            raise ValueError("RPC URL must include Bitcoin Core's RPC port")
        credential = Path(cookie_file).read_text(encoding="utf-8").strip()
        if credential.count(":") < 1 or "\n" in credential:
            raise ValueError("invalid Bitcoin Core RPC cookie")
        token = base64.b64encode(credential.encode()).decode("ascii")
        self.headers = {"Authorization": "Basic " + token, "Content-Type": "application/json"}
        self.connection = http.client.HTTPConnection(target.hostname, target.port, timeout=timeout)
        self.path = target.path or "/"
        self.counter = 0
        self.rpc_requests = 0
        self.http_requests = 0

    def batch(self, calls):
        if not calls:
            return []
        if len(calls) > MAX_BATCH:
            raise ValueError("RPC batch exceeds maximum")
        requests = []
        for method, params in calls:
            self.counter += 1
            requests.append({"jsonrpc": "2.0", "id": self.counter,
                             "method": method, "params": params})
        data = json.dumps(requests, separators=(",", ":")).encode()
        self.connection.request("POST", self.path, body=data, headers=self.headers)
        response = self.connection.getresponse()
        content = response.read(MAX_HTTP_BYTES + 1)
        self.http_requests += 1
        self.rpc_requests += len(requests)
        if len(content) > MAX_HTTP_BYTES:
            raise ValueError("Bitcoin Core RPC response exceeds bounded size")
        if response.status not in (200, 500):
            raise RuntimeError(f"Bitcoin Core HTTP status {response.status}: " +
                               content[:200].decode("utf-8", errors="replace"))
        replies = json.loads(content)
        if not isinstance(replies, list) or len(replies) != len(requests):
            raise ValueError("malformed Bitcoin Core batch reply")
        by_id = {}
        for reply in replies:
            if not isinstance(reply, dict) or type(reply.get("id")) is not int:
                raise ValueError("invalid RPC reply ID")
            if reply["id"] in by_id:
                raise ValueError("duplicate RPC reply ID")
            by_id[reply["id"]] = reply
        ordered = []
        for request in requests:
            reply = by_id.get(request["id"])
            if reply is None:
                raise ValueError("missing RPC reply ID")
            if reply.get("error") is not None:
                error = reply["error"]
                if not isinstance(error, dict):
                    raise ValueError("malformed RPC error")
                raise RPCError(error.get("code"), error.get("message", "unknown"))
            ordered.append(reply.get("result"))
        if response.status != 200:
            raise RuntimeError("Bitcoin Core returned HTTP 500 without a JSON-RPC error")
        return ordered

    def call(self, method, *params):
        return self.batch([(method, list(params))])[0]


def valid_hash(value):
    return (isinstance(value, str) and len(value) == 64 and
            all(char in "0123456789abcdef" for char in value))


def raw_hex(value, max_bytes):
    if (not isinstance(value, str) or len(value) < 2 or
            len(value) % 2 or len(value) // 2 > max_bytes or not value.isascii()):
        raise ValueError("invalid or oversized raw block/transaction hex in RPC reply")
    return value


def active_tip(rpc, network):
    chain = rpc.call("getblockchaininfo")
    if not isinstance(chain, dict) or chain.get("chain") != network:
        raise ValueError(f"wrong Bitcoin network: expected {network}, got {chain!r}")
    height = chain.get("blocks")
    if type(height) is not int or height < 0:
        raise ValueError("invalid Bitcoin Core active-chain height")
    return height


def block_batch(rpc, heights):
    """Stage complete, cross-check all active block hashes, then emit."""
    hashes = rpc.batch([("getblockhash", [height]) for height in heights])
    if not all(valid_hash(block_hash) for block_hash in hashes):
        raise ValueError("malformed getblockhash reply")
    raw_blocks = rpc.batch([("getblock", [block_hash, 0]) for block_hash in hashes])
    raw_blocks = [raw_hex(raw, MAX_BLOCK_BYTES) for raw in raw_blocks]
    rechecked = rpc.batch([("getblockhash", [height]) for height in heights])
    if rechecked != hashes:
        raise ValueError("Bitcoin active chain changed during block RPC batch")
    return list(zip(heights, hashes, raw_blocks))


def stream_blocks(rpc, network, start_height, end_height, batch_size, output):
    tip = active_tip(rpc, network)
    if not (0 <= start_height <= end_height <= tip):
        raise ValueError(f"height range must lie inside active chain [0,{tip}]")
    pinned_end = rpc.call("getblockhash", end_height)
    if not valid_hash(pinned_end):
        raise ValueError("malformed end-block chain anchor")
    count = 0
    for first in range(start_height, end_height + 1, batch_size):
        heights = list(range(first, min(end_height + 1, first + batch_size)))
        for height, blockhash, raw in block_batch(rpc, heights):
            output.write(f"B\t{height}\t{blockhash}\t{raw}\n")
            count += 1
        output.flush()
    if rpc.call("getblockhash", end_height) != pinned_end:
        raise ValueError("Bitcoin active chain changed before stream completion")
    output.write(f"E\tBLOCKS\t{count}\n")
    output.flush()
    return count


def budget_failure(error):
    return isinstance(error, RPCError) and (
        "Script history index lookup limit exceeded" in error.message or
        "Script history exceeds the" in error.message)


def fetch_script_pages(rpc, query, start_height, end_height, descriptor_range=None):
    """Bounded recursive height bisection on PR #36629's 50 MiB work limit."""
    options = {"start_height": start_height, "end_height": end_height,
               "include_mempool": False, "include_spending_transactions": False}
    if descriptor_range is not None:
        options["range"] = descriptor_range
    try:
        page = rpc.call("getscripthistory", query, 0, options)
    except RPCError as error:
        if not budget_failure(error):
            raise
        if start_height == end_height:
            raise IncompleteHistory(f"lookup budget exceeded for block {start_height}; "
                                    "narrow descriptor range") from error
        mid = (start_height + end_height) // 2
        yield from fetch_script_pages(rpc, query, start_height, mid, descriptor_range)
        yield from fetch_script_pages(rpc, query, mid + 1, end_height, descriptor_range)
        return
    if not isinstance(page, dict) or not isinstance(page.get("transactions"), list):
        raise ValueError("malformed getscripthistory result")
    pruned = page.get("pruned_blocks", [])
    if not isinstance(pruned, list) or any(not valid_hash(p) for p in pruned):
        raise ValueError("malformed getscripthistory pruned_blocks")
    if pruned:
        raise IncompleteHistory(f"missing pruned block data: {','.join(pruned[:8])}; "
                                "recover and retry before claiming complete history")
    if page.get("spending_transactions"):
        raise ValueError("unexpected spending_transactions despite disabled option")
    yield page["transactions"]


def history_rows(transactions):
    for item in transactions:
        if not isinstance(item, dict) or not valid_hash(item.get("txid")):
            raise ValueError("malformed script-history transaction TXID")
        raw = raw_hex(item.get("hex"), 1_000_000)
        blockhash = item.get("blockhash", "-")
        if blockhash != "-" and not valid_hash(blockhash):
            raise ValueError("invalid script-history blockhash")
        if not isinstance(item.get("vout"), list) or not item["vout"]:
            raise ValueError("script-history lacks matching vout")
        indexes = []
        for output in item["vout"]:
            n = output.get("n") if isinstance(output, dict) else None
            if type(n) is not int or not 0 <= n <= 100_000:
                raise ValueError("malformed script-history vout index")
            indexes.append(n)
        if indexes != sorted(set(indexes)):
            raise ValueError("duplicate or unsorted script-history vout indexes")
        yield f"S\t{item['txid']}\t{blockhash}\t{raw}\t{','.join(map(str, indexes))}\n"


def stream_script(rpc, network, query, start_height, end_height, descriptor_range, output):
    tip = active_tip(rpc, network)
    if not (0 <= start_height <= end_height <= tip):
        raise ValueError(f"height range must lie inside active chain [0,{tip}]")
    pinned_end = rpc.call("getblockhash", end_height)
    if not valid_hash(pinned_end):
        raise ValueError("malformed start chain anchor")
    count = 0
    for page in fetch_script_pages(rpc, query, start_height, end_height, descriptor_range):
        rows = list(history_rows(page))  # validate the page before emitting it
        if count + len(rows) > MAX_SCRIPT_RECORDS:
            raise IncompleteHistory("script-history stream exceeds 200,000 records")
        if rpc.call("getblockhash", end_height) != pinned_end:
            raise IncompleteHistory("Bitcoin chain reorganized during script history")
        output.writelines(rows)
        output.flush()
        count += len(rows)
    if rpc.call("getblockhash", end_height) != pinned_end:
        raise IncompleteHistory("Bitcoin chain reorganized before completing script history")
    output.write(f"E\tSCRIPT\t{count}\n")
    output.flush()
    return count


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("blocks", "script"), required=True)
    parser.add_argument("--rpc-url", default="http://127.0.0.1:8332/")
    parser.add_argument("--cookie-file", required=True)
    parser.add_argument("--network", choices=("main", "test", "testnet4", "signet", "regtest"),
                        default="main")
    parser.add_argument("--start-height", type=int, default=0)
    parser.add_argument("--end-height", type=int)
    parser.add_argument("--batch-size", type=int, default=4)
    parser.add_argument("--script", help="exact address, Electrum scripthash, or Core descriptor")
    parser.add_argument("--descriptor-range", help="ranged descriptor index interval: start:end")
    args = parser.parse_args(argv)
    if not 1 <= args.batch_size <= MAX_BATCH:
        parser.error(f"--batch-size must be between 1 and {MAX_BATCH}")
    if args.mode == "script" and not args.script:
        parser.error("--script is required for script mode")
    if args.mode == "blocks" and (args.script or args.descriptor_range):
        parser.error("--script and --descriptor-range belong to script mode")
    descriptor_range = None
    if args.descriptor_range:
        try:
            low, high = (int(v) for v in args.descriptor_range.split(":"))
            if not 0 <= low <= high < 2**31:
                raise ValueError
            descriptor_range = [low, high]
        except (ValueError, TypeError):
            parser.error("--descriptor-range must be start:end with 0 <= start <= end")
    rpc = BitcoinCoreRPC(args.rpc_url, args.cookie_file)
    begun = time.monotonic()
    tip = active_tip(rpc, args.network) if args.end_height is None else args.end_height
    if args.mode == "blocks":
        count = stream_blocks(rpc, args.network, args.start_height, tip,
                              args.batch_size, sys.stdout)
    else:
        count = stream_script(rpc, args.network, args.script, args.start_height,
                              tip, descriptor_range, sys.stdout)
    seconds = max(time.monotonic() - begun, 1e-6)
    print(f"source=BITCOIN_CORE_RPC mode={args.mode} records={count} "
          f"rpc_methods={rpc.rpc_requests} http_roundtrips={rpc.http_requests} "
          f"elapsed_s={seconds:.3f} records_per_s={count / seconds:.1f} "
          "consensus=UNVERIFIED", file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except (RPCError, IncompleteHistory, ValueError, OSError, RuntimeError) as error:
        print(f"INCOMPLETE Bitcoin Core stream: {error}", file=sys.stderr)
        sys.exit(1)
