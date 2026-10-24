#!/usr/bin/env python3
"""Independent Python byte-order oracle for real Rust AccountId/PropertyId.

Generate a temporary integration test, compile it with pinned Rust 1.90,
and compare native Ord, std binary_search_by, and a separately written
half-open cursor implementation against Python tuple ordering.
The production OrderedColumns::locate has its own Rust linear-scan unit
oracle. The generated test is removed and alters no production source.
"""
from __future__ import annotations

import argparse
import bisect
import os
from pathlib import Path
import random
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
TEMP_RUST = ROOT / "tests" / "__native_wallet_order_audit_generated.rs"
SEED = 0x202613B00
PROPERTIES = (1, 2, 3, 7, 65535, 2147483647, 4294967295)
PRINTABLE = tuple(range(33, 127))

# Executed against ACTUAL public PropertyId and AccountId types. The
# std binary search here is a distinct comparator oracle; OrderedColumns
# now uses an explicit lower-bound loop tested in its own unit tests.
RUST_TEST = r'''
use std::cmp::Ordering;
use std::fs;
use tradelayer_core::ledger::{AccountId, PropertyId};

fn parse_key(line: &str) -> (PropertyId, AccountId) {
    let mut fields = line.split('\t');
    let property: u32 = fields.next().expect("property").parse().expect("u32");
    let hex = fields.next().expect("hex");
    assert_eq!(hex.len() % 2, 0);
    let bytes = hex.as_bytes().chunks_exact(2).map(|part| {
        u8::from_str_radix(std::str::from_utf8(part).expect("ASCII hex"), 16)
            .expect("hex byte")
    }).collect::<Vec<_>>();
    let name = std::str::from_utf8(&bytes).expect("ASCII");
    (PropertyId::try_from(property).expect("valid property"),
     AccountId::try_from(name).expect("valid account"))
}

#[test]
fn native_tuple_order_std_and_half_open_search_match_python_reference() {
    let keyfile = std::env::var("TL_ORDER_KEYS").expect("keys path");
    let queryfile = std::env::var("TL_ORDER_QUERIES").expect("queries path");
    let text = fs::read_to_string(keyfile).expect("keys");
    let entries: Vec<((PropertyId, AccountId), u128)> =
        text.lines().enumerate().map(|(i, line)| (parse_key(line), (i + 1) as u128))
            .collect();
    assert!(entries.len() >= 600);
    assert!(entries.windows(2).all(|p| p[0].0 < p[1].0));
    for (i, (a, _)) in entries.iter().enumerate() {
        for (j, (b, _)) in entries.iter().enumerate() {
            assert_eq!(a.cmp(b), i.cmp(&j), "Ord mismatch at {i} vs {j}");
        }
    }
    for invalid in ["", "with space", "a\nb", "a\tb", "\u{00e9}"] {
        assert!(AccountId::try_from(invalid).is_err());
    }
    assert!(AccountId::try_from("A".repeat(91).as_str()).is_err());
    assert!(PropertyId::try_from(0).is_err());
    let queries = fs::read_to_string(queryfile).expect("query vectors");
    let mut hits = 0usize;
    let mut misses = 0usize;
    for (test_index, line) in queries.lines().enumerate() {
        let mut fields = line.split('\t');
        let property = fields.next().unwrap();
        let account = fields.next().unwrap();
        let present = fields.next().unwrap();
        let index: usize = fields.next().unwrap().parse().unwrap();
        assert!(fields.next().is_none());
        let key = parse_key(&format!("{property}\t{account}"));
        let expected = if present == "1" {
            hits += 1;
            Ok(index)
        } else {
            assert_eq!(present, "0");
            misses += 1;
            Err(index)
        };
        let actual = entries.binary_search_by(|(stored, _)| stored.cmp(&key));
        assert_eq!(actual, expected, "std binary_search_by at query {test_index}");
        // Independent rendition of the explicit production cursor algorithm.
        // Production's actual OrderedColumns::locate is checked separately
        // against a complete linear scan in src/ordered_columns.rs tests.
        let mut left = 0usize;
        let mut right = entries.len();
        while left < right {
            let middle = left + (right - left) / 2;
            if entries[middle].0.cmp(&key).is_lt() {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        assert_eq!(left, index, "half-open lower bound at query {test_index}");
        let partition = entries.partition_point(|(stored, _)| stored < &key);
        assert_eq!(partition, index, "partition_point at {test_index}");
        assert!(index <= entries.len());
        if let Ok(i) = actual {
            assert_eq!(entries[i].0.cmp(&key), Ordering::Equal);
        } else {
            assert!(entries[..index].iter().all(|(stored, _)| stored < &key));
            assert!(entries[index..].iter().all(|(stored, _)| stored > &key));
        }
    }
    assert!(hits >= 500 && misses >= 500);
}
'''

def valid_name(name: bytes) -> bool:
    return 1 <= len(name) <= 90 and all(33 <= byte <= 126 for byte in name)

def ghost_rank(key: tuple[int, bytes]) -> int:
    p, name = key
    return (p + 1) * 257**90 + sum(
        (byte + 1) * 257 ** (89 - pos) for pos, byte in enumerate(name)
    )

def make_vectors():
    rng = random.Random(SEED)
    edge = [
        b"!", b"~", b"A", b"a", b"aa", b"ab", b"a!", b"a~",
        b"a" * 89, b"a" * 90, b"a" * 89 + b"b",
        b"~" * 90, b"!" * 90, b"A" * 90, b"A" * 89 + b"B",
        bytes(range(33, 127))[:90],
    ]
    records = {(p, s) for p in PROPERTIES for s in edge if valid_name(s)}

    # Exhaust the one-byte printable ASCII domain on two extreme properties.
    # Cross-check every nested prefix length for both 'A' and '~' on the
    # same properties. This is finite coverage of real String::Ord, not a
    # universal proof of arbitrarily long native account strings.
    exhaustive = {
        (p, bytes([byte]))
        for p in (1, 4294967295)
        for byte in PRINTABLE
    }
    exhaustive.update(
        (p, symbol * width)
        for p in (1, 4294967295)
        for symbol in (b"A", b"~")
        for width in range(1, 91)
    )
    records.update(exhaustive)
    for _ in range(800):
        count = rng.choice((1, 2, 3, 4, 5, 8, 16, 31, 32, 64, 89, 90))
        name = bytes(rng.randrange(33, 127) for _ in range(count))
        records.add((rng.choice(PROPERTIES), name))
    keys = sorted(records)
    assert len(keys) >= 600
    for left, right in zip(keys, keys[1:]):
        assert left < right and ghost_rank(left) < ghost_rank(right)
    queries = [rng.choice(keys) for _ in range(600)]
    for _ in range(1200):
        p, name = rng.choice(keys)
        variant = rng.randrange(5)
        if variant == 0:
            new = name[:-1] if len(name) > 1 else name + b"!"
        elif variant == 1:
            new = name[:89] + b"~" if len(name) == 90 else name + b"~"
        elif variant == 2:
            new = bytes(rng.randrange(33, 127) for _ in range(rng.randint(1, 90)))
        elif variant == 3:
            p, new = rng.choice(PROPERTIES), name
        else:
            new = b"!" * rng.randint(1, 90)
        if valid_name(new):
            queries.append((p, new))
    queries.extend((p, name) for p in PROPERTIES
        for name in (b"!", b"~" * 90, b"A" * 90))
    # Every exhausted native byte/prefix fixture is checked as a query,
    # rather than relying on the randomized hit sample alone.
    queries.extend(sorted(exhaustive))
    cases = []
    hits = misses = 0
    for query in queries:
        index = bisect.bisect_left(keys, query)
        found = index < len(keys) and keys[index] == query
        cases.append((*query, found, index))
        hits += found
        misses += not found
    assert hits >= 500 and misses >= 500
    return keys, cases

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-only", action="store_true",
                        help="check vectors without compiling Rust")
    args = parser.parse_args()
    keys, cases = make_vectors()
    if args.python_only:
        print(f"wallet-order vectors OK: {len(keys)} keys, {len(cases)} queries")
        return
    if not (ROOT / "Cargo.toml").is_file() or TEMP_RUST.exists():
        parser.error("must run in a clean TradeLayer-Rust repository checkout")
    with tempfile.TemporaryDirectory(prefix="tradelayer-key-order-") as tmp:
        keyfile = Path(tmp) / "keys.tsv"
        queryfile = Path(tmp) / "queries.tsv"
        keyfile.write_text("\n".join(f"{p}\t{name.hex()}" for p, name in keys) + "\n")
        queryfile.write_text("\n".join(
            f"{p}\t{name.hex()}\t{int(found)}\t{index}"
            for p, name, found, index in cases
        ) + "\n")
        try:
            TEMP_RUST.write_text(RUST_TEST)
            env = {**os.environ, "TL_ORDER_KEYS": str(keyfile),
                   "TL_ORDER_QUERIES": str(queryfile)}
            subprocess.run(["cargo", "+1.90.0", "test", "--frozen", "--test",
                "__native_wallet_order_audit_generated"], cwd=ROOT,
                env=env, check=True)
        finally:
            TEMP_RUST.unlink(missing_ok=True)
    print(f"Native wallet key oracle PASS: {len(keys)} keys, {len(cases)} queries")

if __name__ == "__main__":
    main()
