"""SYNTHETIC Litecoin-testnet block at the pinned legacy JS genesis height.

Builds one parent block and one candidate block through the real collector,
source-trace and verifier code paths, using a fake RPC. Nothing here is
chain evidence: block hashes have no proof of work (the verifier's Scrypt
check is skipped) and the transactions were never broadcast. It exists so the
legacy JS harness can be exercised end to end, including intra-block order.

The sender is the testnet admin address hardcoded in tradelayer.js@dca0888b
src/activation.js; scripts are real P2WPKH encodings of the bech32 programs.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import collect_litecoin_rpc as base  # noqa: E402
import litecoin_source_trace as trace  # noqa: E402
import verify_litecoin_evidence as verifier  # noqa: E402

CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
ADMIN = "tltc1qa0kd2d39nmeph3hvcx8ytv65ztcywg5sazhtw8"
GENESIS = verifier.LEGACY_JS_GENESIS["test"]


def _polymod(values):
    gen = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for v in values:
        top = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ v
        for i in range(5):
            chk ^= gen[i] if (top >> i) & 1 else 0
    return chk


def _hrp_expand(hrp):
    return [ord(x) >> 5 for x in hrp] + [0] + [ord(x) & 31 for x in hrp]


def _convertbits(data, frombits, tobits, pad):
    acc = bits = 0
    out = []
    for value in data:
        acc = (acc << frombits) | value
        bits += frombits
        while bits >= tobits:
            bits -= tobits
            out.append((acc >> bits) & ((1 << tobits) - 1))
    if pad and bits:
        out.append((acc << (tobits - bits)) & ((1 << tobits) - 1))
    elif not pad and (bits >= frombits or ((acc << (tobits - bits)) & ((1 << tobits) - 1))):
        raise ValueError("invalid padding")
    return out


def segwit_v0_program(address: str) -> bytes:
    hrp, data = address.rsplit("1", 1)
    values = [CHARSET.index(c) for c in data]
    if _polymod(_hrp_expand(hrp) + values) != 1 or values[0] != 0:
        raise ValueError("not a bech32 v0 address")
    return bytes(_convertbits(values[1:-6], 5, 8, False))


def segwit_v0_address(hrp: str, program: bytes) -> str:
    data = [0] + _convertbits(program, 8, 5, True)
    polymod = _polymod(_hrp_expand(hrp) + data + [0] * 6) ^ 1
    checksum = [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]
    return hrp + "1" + "".join(CHARSET[d] for d in data + checksum)


RECIPIENT = segwit_v0_address("tltc", bytes(range(20)))


def p2wpkh(address: str) -> bytes:
    return b"\x00\x14" + segwit_v0_program(address)


def raw_tx(inputs, outputs) -> str:
    data = bytearray(b"\x02\x00\x00\x00")
    data.append(len(inputs))
    for prev, vout in inputs:
        data += bytes.fromhex(prev)[::-1] + vout.to_bytes(4, "little") + b"\x00" + b"\xff\xff\xff\xff"
    data.append(len(outputs))
    for value, script in outputs:
        data += value.to_bytes(8, "little") + bytes([len(script)]) + script
    return (bytes(data) + bytes(4)).hex()


def header(prev: str, merkle: str):
    raw = (b"\x00\x00\x00\x20" + bytes.fromhex(prev)[::-1] + bytes.fromhex(merkle)[::-1]
           + (1_700_000_000).to_bytes(4, "little") + (0x1E0FFFF0).to_bytes(4, "little") + bytes(4))
    return raw.hex(), base.sha256d(raw)[::-1].hex()


def decoded_output(n, sats, script: bytes, address=None):
    if script[:1] == b"\x6a":
        spk = {"hex": script.hex(), "type": "nulldata"}
    else:
        spk = {"hex": script.hex(), "type": "witness_v0_keyhash", "addresses": [address]}
    return {"n": n, "value": float(f"{sats / 1e8:.8f}"), "scriptPubKey": spk}


def opreturn(payload: str) -> bytes:
    data = payload.encode("ascii")
    return bytes([0x6A, len(data)]) + data


class SyntheticChain:
    """Parent block at GENESIS-1, candidate block at GENESIS."""

    def __init__(self, payloads: list[str]):
        self.txs, self.blocks, self.by_height = {}, {}, {}
        coinbase_parent = raw_tx([("00" * 32, 0xFFFFFFFF)], [(1_000, p2wpkh(ADMIN))])
        parent_outputs = [(100_000 + i, p2wpkh(ADMIN)) for i in range(len(payloads))]
        funding = raw_tx([("ab" * 32, 0)], parent_outputs)
        self._block(GENESIS - 1, "77" * 32, [(coinbase_parent, [(1_000, p2wpkh(ADMIN), ADMIN)]),
                                             (funding, [(v, s, ADMIN) for v, s in parent_outputs])])
        funding_id = base.parse_transaction(funding)["txid"]
        coinbase = raw_tx([("00" * 32, 0xFFFFFFFF)], [(5_000, p2wpkh(RECIPIENT))])
        entries = [(coinbase, [(5_000, p2wpkh(RECIPIENT), RECIPIENT)])]
        for i, payload in enumerate(payloads):
            outs = [(0, opreturn(payload), None), (546, p2wpkh(RECIPIENT), RECIPIENT)]
            entries.append((raw_tx([(funding_id, i)], [(v, s) for v, s, _ in outs]), outs))
        self._block(GENESIS, self.by_height[GENESIS - 1], entries)
        self.tip = GENESIS + 10

    def _block(self, height, prev, entries):
        ids = []
        for raw, outs in entries:
            txid = base.parse_transaction(raw)["txid"]
            ids.append(txid)
            self.txs[txid] = {"raw": raw, "decoded": {
                "txid": txid, "hex": raw,
                "vin": [{"txid": p, "vout": v} for p, v in base.parse_transaction(raw)["inputs"]],
                "vout": [decoded_output(n, v, s, a) for n, (v, s, a) in enumerate(outs)]}}
        head, block_hash = header(prev, base.merkle_root(ids))
        for txid in ids:
            self.txs[txid]["blockhash"] = block_hash
        self.blocks[block_hash] = {"hash": block_hash, "height": height, "previousblockhash": prev,
                                   "merkleroot": base.merkle_root(ids), "tx": ids, "header": head}
        self.by_height[height] = block_hash

    def call(self, method, params=None):
        params = params or []
        if method == "getblockchaininfo":
            return {"chain": "test", "pruned": False, "initialblockdownload": False,
                    "blocks": self.tip, "headers": self.tip}
        if method == "getnetworkinfo":
            return {"version": 0, "subversion": "/synthetic-fake-rpc/", "protocolversion": 0}
        if method == "getindexinfo":
            return {"txindex": {"synced": True, "best_block_height": self.tip}}
        if method == "getblockcount":
            return self.tip
        if method == "getblockhash":
            if params == [0]:
                return trace.PINNED_GENESIS["test"]
            return self.by_height[params[0]]
        if method == "getblockheader":
            return self.blocks[params[0]]["header"]
        if method == "getblock":
            block = dict(self.blocks[params[0]])
            block["confirmations"] = self.tip - block["height"] + 1
            if params[1] == 2:
                block["tx"] = [self.txs[t]["decoded"] for t in block["tx"]]
            return block
        if method == "validateaddress":
            address = params[0]
            ok = address.startswith("tltc1")
            return {"isvalid": ok, "address": address, "iswitness": ok, "ismine": "must-not-be-kept"}
        if method == "getrawtransaction":
            tx = self.txs[params[0]]
            if params[1] is False:
                return tx["raw"]
            return dict(tx["decoded"], blockhash=tx["blockhash"], confirmations=1)
        raise AssertionError((method, params))


def write_fixture(out_dir: Path, payloads: list[str]) -> dict:
    chain = SyntheticChain(payloads)
    corpus, manifest = base.scan_blocks(chain, "test", GENESIS, GENESIS, confirmations=6)
    manifest = trace.augment_with_sources(chain, corpus, manifest)
    manifest["SYNTHETIC"] = "fake RPC; no proof of work; never chain evidence"
    report = verifier.verify([(corpus, json.dumps(manifest))], trace.LEGACY_JS_COMMIT, False)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "corpus.tsv").write_text(corpus)
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2))
    (out_dir / "verification.json").write_text(json.dumps(report, indent=2))
    return report


DEFAULT_PAYLOADS = [
    "tl01;2,1",                                  # admin activates types 1 and 2
    "tl1rs,SYN,,0,,0,0,",                        # issue 1000 SYN
    "tl20;" + RECIPIENT + ";1;a",                # send 10 of property 1 (issued above)
]

def write_recordings(out_dir: Path, replay_report: Path) -> dict:
    import litecoin_record_rpc as recorder
    requests = json.loads(replay_report.read_text())["rpc_requests"]
    recordings = recorder.record(SyntheticChain(DEFAULT_PAYLOADS), "test", requests)
    recordings["SYNTHETIC"] = "fake RPC answers; never chain evidence"
    (out_dir / "recordings.json").write_text(json.dumps(recordings, indent=2))
    return recordings


if __name__ == "__main__":
    if len(sys.argv) == 3:
        print(json.dumps(write_recordings(Path(sys.argv[1]), Path(sys.argv[2]))["validateaddress"]))
    else:
        print(json.dumps(write_fixture(Path(sys.argv[1]), DEFAULT_PAYLOADS)["counts"]))
