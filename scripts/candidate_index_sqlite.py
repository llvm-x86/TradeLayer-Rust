#!/usr/bin/env python3
"""Atomic SQLite store for UNVERIFIED TradeLayer candidate evidence.

Consumes paired completed Bitcoin Core B/E block frames and the existing Rust
C/END candidate frames. No trusted chain or ledger privileges are conferred.
Only the block mode is admissible: script history lacks empty-block evidence.
"""
import argparse
from dataclasses import dataclass
import hashlib
from pathlib import Path
import sqlite3
import sys

MAX_FRAME = 8_000_512
MAX_BLOCK_BYTES = 4_000_000
MAX_TX_BYTES = 1_000_000
MAX_BLOCKS = 4096
MAX_CARRIERS = 200_000
MAX_TOTAL_TX = 1_000_000
NETWORKS = ("main", "test", "testnet4", "signet", "regtest")
SCHEMA = "TLCAND1"


class Incomplete(Exception):
    """Incomplete, incompatible, or untrusted evidence. Entire batch denied."""


@dataclass(frozen=True)
class Block:
    height: int
    hash: str
    parent: str
    tx_count: int


def is_hash(value):
    return len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def check_hash(value):
    if not is_hash(value):
        raise Incomplete("expected a 64-digit lowercase block/TX hash")
    return value


def number(value, limit=2**31-1):
    if not value.isascii() or not value.isdecimal() or (len(value) > 1 and value[0] == "0"):
        raise Incomplete("noncanonical unsigned decimal field")
    n = int(value)
    if n > limit:
        raise Incomplete("numeric field exceeds configured bound")
    return n


def frames(path):
    with Path(path).open("rb") as source:
        while True:
            raw = source.readline(MAX_FRAME + 1)
            if not raw:
                break
            if len(raw) > MAX_FRAME or not raw.endswith(b"\n"):
                raise Incomplete("overlarge or unterminated stream frame")
            try:
                line = raw[:-1].decode("ascii")
            except UnicodeDecodeError as exc:
                raise Incomplete("stream contains non-ASCII data") from exc
            if "\r" in line or "\x00" in line:
                raise Incomplete("noncanonical frame delimiters")
            yield line.split("\t")


def compact_count(raw):
    if len(raw) <= 80:
        raise Incomplete("truncated Bitcoin block transaction count")
    marker = raw[80]
    if marker < 253:
        count = marker
    else:
        length = {253: 2, 254: 4, 255: 8}[marker]
        if len(raw) < 81 + length:
            raise Incomplete("truncated Bitcoin CompactSize")
        count = int.from_bytes(raw[81:81+length], "little")
        if count < 253 or (marker == 254 and count <= 65535) or (marker == 255 and count <= 2**32-1):
            raise Incomplete("noncanonical Bitcoin CompactSize")
    if not 1 <= count <= MAX_TOTAL_TX:
        raise Incomplete("invalid raw block transaction count")
    return count


def parse_blocks(path):
    blocks = []
    ended = False
    tx_sum = 0
    for columns in frames(path):
        if ended:
            raise Incomplete("data after B/E block completion marker")
        if len(columns) == 4 and columns[0] == "B":
            _, height_s, advertised, rawhex = columns
            height = number(height_s)
            check_hash(advertised)
            if len(rawhex) % 2 or len(rawhex) // 2 > MAX_BLOCK_BYTES:
                raise Incomplete("raw block size exceeds bound")
            try:
                raw = bytes.fromhex(rawhex)
            except ValueError as exc:
                raise Incomplete("malformed raw block hex") from exc
            if len(raw) < 81:
                raise Incomplete("truncated raw block")
            digest = hashlib.sha256(hashlib.sha256(raw[:80]).digest()).digest()
            if digest[::-1].hex() != advertised:
                raise Incomplete("block header hash does not match raw header")
            parent = raw[4:36][::-1].hex()
            count = compact_count(raw)
            if blocks:
                previous = blocks[-1]
                if height != previous.height + 1 or parent != previous.hash:
                    raise Incomplete("block stream is not contiguous and parent-linked")
            blocks.append(Block(height, advertised, parent, count))
            tx_sum += count
            if len(blocks) > MAX_BLOCKS or tx_sum > MAX_TOTAL_TX:
                raise Incomplete("block batch exceeds resource cap")
        elif len(columns) == 3 and columns[:2] == ["E", "BLOCKS"]:
            if number(columns[2], MAX_BLOCKS) != len(blocks) or not blocks:
                raise Incomplete("invalid block completion count")
            ended = True
        else:
            raise Incomplete("unexpected or malformed raw-block stream frame")
    if not ended:
        raise Incomplete("truncated raw-block stream: missing E sentinel")
    return blocks, tx_sum


def open_database(path):
    db = sqlite3.connect(path, timeout=10)
    db.execute("PRAGMA foreign_keys=ON")
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=FULL")
    db.execute("PRAGMA busy_timeout=10000")
    return db


def metadata(db):
    try:
        result = dict(db.execute("SELECT key,value FROM metadata"))
    except sqlite3.DatabaseError as exc:
        raise Incomplete("uninitialized or corrupt candidate index") from exc
    if result.get("schema") != SCHEMA:
        raise Incomplete("candidate index schema unknown or uninitialized")
    return result


def init(path, network, anchor_height, anchor_hash):
    check_hash(anchor_hash)
    if network not in NETWORKS or not -1 <= anchor_height <= 2**31 - 1:
        raise Incomplete("invalid network or chain anchor height")
    if anchor_height == -1 and anchor_hash != "0" * 64:
        raise Incomplete("pre-genesis anchor must have zero parent")
    p = Path(path)
    if p.exists():
        raise Incomplete("index file already exists; explicit init cannot overwrite it")
    if not p.parent.is_dir():
        raise Incomplete("database parent directory does not exist")
    db = open_database(p)
    try:
        with db:
            db.executescript("""
                CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE blocks(
                    height INTEGER PRIMARY KEY,
                    hash TEXT UNIQUE NOT NULL,
                    parent TEXT NOT NULL,
                    tx_count INTEGER NOT NULL CHECK(tx_count > 0)
                );
                CREATE TABLE transactions(
                    height INTEGER NOT NULL,
                    tx_position INTEGER NOT NULL,
                    txid TEXT NOT NULL,
                    raw BLOB NOT NULL,
                    PRIMARY KEY(height, tx_position),
                    FOREIGN KEY(height) REFERENCES blocks(height) ON DELETE CASCADE
                );
                CREATE TABLE carriers(
                    height INTEGER NOT NULL,
                    tx_position INTEGER NOT NULL,
                    vout INTEGER NOT NULL,
                    format TEXT NOT NULL,
                    tx_type TEXT NOT NULL,
                    syntax_status TEXT NOT NULL,
                    PRIMARY KEY(height, tx_position, vout),
                    FOREIGN KEY(height,tx_position)
                        REFERENCES transactions(height,tx_position) ON DELETE CASCADE
                );
                CREATE INDEX carrier_txid ON transactions(txid);
            """)
            db.executemany("INSERT INTO metadata VALUES (?,?)", [
                ("schema", SCHEMA), ("network", network),
                ("anchor_height", str(anchor_height)),
                ("anchor_hash", anchor_hash), ("trust", "UNVERIFIED"),
            ])
    finally:
        db.close()


def parent_at(db, meta, height):
    anchor_height = int(meta["anchor_height"])
    if height == anchor_height:
        return meta["anchor_hash"]
    row = db.execute("SELECT hash FROM blocks WHERE height=?", (height,)).fetchone()
    return row[0] if row else None


def load_carriers(db, path, blocks, tx_sum):
    available = {block.height: block for block in blocks}
    seen = 0
    current_key = None
    tx_cache_key = None
    tx_cache_data = None
    complete = False
    for columns in frames(path):
        if complete:
            raise Incomplete("data after Rust END completion marker")
        if len(columns) == 11 and columns[:2] == ["C", "BLOCK"]:
            _, _, height_s, hash_s, position_s, txid, vout_s, fmt, tx_type, status, rawhex = columns
            height = number(height_s)
            block = available.get(height)
            if block is None or hash_s != block.hash:
                raise Incomplete("candidate is missing its exact source block")
            pos = number(position_s, block.tx_count - 1)
            vout = number(vout_s, 100_000)
            check_hash(txid)
            if len(rawhex) % 2 or not 2 <= len(rawhex) // 2 <= MAX_TX_BYTES:
                raise Incomplete("candidate transaction size invalid")
            try:
                raw = bytes.fromhex(rawhex)
            except ValueError as exc:
                raise Incomplete("invalid candidate raw transaction") from exc
            for label in (fmt, tx_type, status):
                if not 1 <= len(label) <= 100 or not all(c.isascii() and (c.isalnum() or c in "_-.:") for c in label):
                    raise Incomplete("candidate classification contains invalid characters")
            key = (height, pos, vout)
            if current_key is not None and key <= current_key:
                raise Incomplete("candidate rows must be strictly ordered by height, tx position, vout")
            current_key = key
            tx_key = (height, pos)
            tx_data = (txid, raw)
            if tx_key != tx_cache_key:
                row = db.execute("SELECT txid,raw FROM transactions WHERE height=? AND tx_position=?", tx_key).fetchone()
                if row is not None and row != tx_data:
                    raise Incomplete("conflicting raw transactions at the same block position")
                if row is None:
                    db.execute("INSERT INTO transactions VALUES (?,?,?,?)", (height, pos, txid, raw))
                tx_cache_key, tx_cache_data = tx_key, tx_data
            elif tx_cache_data != tx_data:
                raise Incomplete("conflicting raw bytes for the same transaction")
            db.execute("INSERT INTO carriers VALUES (?,?,?,?,?,?)", (height, pos, vout, fmt, tx_type, status))
            seen += 1
            if seen > MAX_CARRIERS:
                raise Incomplete("candidate batch exceeds resource cap")
        elif len(columns) == 6 and columns[:3] == ["END", "COMPLETE_UNVERIFIED", "BLOCKS"]:
            _, _, _, blocks_s, txs_s, candidates_s = columns
            if (number(blocks_s, MAX_BLOCKS) != len(blocks) or
                    number(txs_s, MAX_TOTAL_TX) != tx_sum or
                    number(candidates_s, MAX_CARRIERS) != seen):
                raise Incomplete("Rust candidate completion counts disagree with raw blocks")
            complete = True
        else:
            raise Incomplete("unknown or malformed Rust candidate frame")
    if not complete:
        raise Incomplete("truncated Rust candidate stream: missing END sentinel")
    return seen


def ingest(path, network, block_frames, candidate_frames):
    blocks, tx_sum = parse_blocks(block_frames)
    db = open_database(path)
    try:
        # BEGIN IMMEDIATE excludes conflicting writers and makes fork replacement,
        # raw candidate writes and completion validation one rollback-safe unit.
        db.execute("BEGIN IMMEDIATE")
        try:
            meta = metadata(db)
            if meta["network"] != network:
                raise Incomplete("wrong blockchain namespace for candidate index")
            first = blocks[0]
            parent_hash = parent_at(db, meta, first.height - 1)
            if parent_hash is None or parent_hash != first.parent:
                raise Incomplete("candidate block range does not attach to stored chain/anchor")
            db.execute("DELETE FROM blocks WHERE height>=?", (first.height,))
            db.executemany("INSERT INTO blocks VALUES (?,?,?,?)", [
                (block.height, block.hash, block.parent, block.tx_count) for block in blocks
            ])
            candidates = load_carriers(db, candidate_frames, blocks, tx_sum)
            db.execute("COMMIT")
        except BaseException:
            db.execute("ROLLBACK")
            raise
        return len(blocks), tx_sum, candidates, blocks[-1]
    finally:
        db.close()


def stats(path):
    db = open_database(path)
    try:
        meta = metadata(db)
        latest = db.execute("SELECT height,hash FROM blocks ORDER BY height DESC LIMIT 1").fetchone()
        return {
            "network": meta["network"], "trust": meta["trust"],
            "height": latest[0] if latest else int(meta["anchor_height"]),
            "hash": latest[1] if latest else meta["anchor_hash"],
            "blocks": db.execute("SELECT COUNT(*) FROM blocks").fetchone()[0],
            "transactions": db.execute("SELECT COUNT(*) FROM transactions").fetchone()[0],
            "carriers": db.execute("SELECT COUNT(*) FROM carriers").fetchone()[0],
        }
    finally:
        db.close()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="command", required=True)
    for name in ("init", "ingest", "status"):
        p = sub.add_parser(name)
        p.add_argument("--db", required=True)
        if name in ("init", "ingest"):
            p.add_argument("--network", choices=NETWORKS, required=True)
        if name == "init":
            p.add_argument("--anchor-height", required=True, type=int)
            p.add_argument("--anchor-hash", required=True)
        if name == "ingest":
            p.add_argument("--blocks", required=True)
            p.add_argument("--candidates", required=True)
    args = ap.parse_args(argv)
    if args.command == "init":
        init(args.db, args.network, args.anchor_height, args.anchor_hash)
        print("initialized UNVERIFIED candidate index")
    elif args.command == "ingest":
        blocks, txs, rows, last = ingest(args.db, args.network, args.blocks, args.candidates)
        print(f"committed=UNVERIFIED blocks={blocks} txs={txs} carriers={rows} tip={last.height}:{last.hash}")
    else:
        for key, value in stats(args.db).items():
            print(f"{key}={value}")


if __name__ == "__main__":
    try:
        main()
    except (Incomplete, OSError, sqlite3.Error) as error:
        print(f"REJECTED CANDIDATE INDEX: {error}", file=sys.stderr)
        raise SystemExit(1) from error
