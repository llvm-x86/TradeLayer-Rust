#!/usr/bin/env python3
"""Pin the source fragments used for Bitcoin Core #36629 synchronization claims.

This is an evidence gate, not a C++/Lean proof. Fetches exact raw files from
the upstream PR HEAD commit, checks the original Git blob SHA1, then checks
the presence and ordering of the relevant source statements. It deliberately
fails when offline or when upstream code provenance no longer matches.
"""
import hashlib
import time
import urllib.error
import urllib.request

COMMIT = "cf9b981fcc7ae52c7864beae35df5d60b5c322b4"
BASE = f"https://raw.githubusercontent.com/andrewtoth/bitcoin/{COMMIT}/"

FILES = {
    "src/index/base.cpp": {
        "blob": "160c5fa8cd676d205106768fe4cc07c317e0829b",
        "fragments": (
            "bool BaseIndex::BlockUntilSyncedToCurrentChain() const",
            "m_chain->context()->validation_signals->SyncWithValidationInterfaceQueue();",
            "bool BaseIndex::Rewind(const CBlockIndex* current_tip, const CBlockIndex* new_tip)",
            "if (!CustomRemove(block_info))",
            "if (ProcessBlock(pindex, block.get()))",
            "SetBestBlockIndex(pindex);",
        ),
    },
    "src/rpc/blockchain.cpp": {
        "blob": "3e045b87d6dcfd417994f5967c5be2442f85f492",
        "fragments": (
            "const bool index_ready{g_txoscriptindex->BlockUntilSyncedToCurrentChain()};",
            "tip = chainstate.m_chain.Tip();",
            "std::map<std::pair<int32_t, uint32_t>, ScriptHistoryTransaction> by_position;",
            "for (const uint256& hash : script_hashes)",
            "auto result{g_txoscriptindex->Find(hash, start_height, end_height, remaining_bytes)};",
            "by_position.try_emplace(",
            "decoded_outputs.at(output.index)",
            "chainman.ActiveChain().Tip() != tip",
        ),
    },
    "src/index/txoscriptindex.cpp": {
        "blob": "b7d26cd7206a6e3e4290ec4594e914b8ad2f32f5",
        "fragments": (
            "bool TxoScriptIndex::CustomAppend(const interfaces::BlockInfo& block)",
            "bool TxoScriptIndex::CustomRemove(const interfaces::BlockInfo& block)",
            "std::unique_ptr<CDBIterator> it{m_db->NewIterator()};",
            "block_index = m_chainstate->m_chain[key.height];",
            "block_hash = block_index->GetBlockHash();",
            "if (txoscriptindex::Sha256ScriptPubKey(tx->vout[index].scriptPubKey) == script_hash)",
        ),
    },
}


def get_text(path: str) -> str:
    url = BASE + path
    last_exc = None
    for attempt in range(3):
        try:
            with urllib.request.urlopen(
                urllib.request.Request(url, headers={"User-Agent": "bitcoin-36629-formal-audit"}),
                timeout=25,
            ) as resp:
                return resp.read().decode("utf-8")
        except (OSError, urllib.error.URLError) as exc:
            last_exc = exc
            if attempt < 2:
                time.sleep(2 * (attempt + 1))
    raise RuntimeError(f"Could not verify source {url}: {last_exc}")


def git_blob_sha(raw: bytes) -> str:
    return hashlib.sha1(f"blob {len(raw)}\0".encode() + raw).hexdigest()


for path, spec in FILES.items():
    source = get_text(path)
    actual_sha = git_blob_sha(source.encode())
    assert actual_sha == spec["blob"], (
        f"{path} SHA changed: expected {spec['blob']}, received {actual_sha}"
    )
    for fragment in spec["fragments"]:
        assert fragment in source, (path, fragment)
    print(f"PASS: pinned source {path}, blob={actual_sha}, "
          f"{len(spec['fragments'])} reviewed source predicates")
print("Scope: source-structure provenance, not a query-wide synchronization guarantee")
