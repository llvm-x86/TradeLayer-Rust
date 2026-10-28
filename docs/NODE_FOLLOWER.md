# Live block follower over a JSON-RPC block source (Phase 3)

**Status: library only, unverified.** This does not validate proof of work or
consensus, and it treats the node as **trusted**. It has been exercised only
against an in-memory mock and a loopback fake JSON-RPC server. No real
Litecoin, Bitcoin or Dogecoin node was contacted. Nothing is wired into a
binary, and the README's "cannot process real funds" statement still holds.

## What it does

`src/node_follower.rs` advances a `JointChainState` by whole blocks taken from
a `BlockSource`:

1. **Profile gate.** The network's PoW family must be `BitcoinSha256d`.
   Litecoin and Dogecoin profiles are refused with `HeaderHashUnavailable`.
   Their block hashes are Scrypt (and Dogecoin uses AuxPoW), and this crate
   does not implement either. See "Litecoin and Dogecoin" below.
2. **Genesis gate.** `getblockhash(0)` must equal the network profile's pinned
   genesis, in display order. Otherwise the call fails with `GenesisMismatch`.
3. **Reorg detection.** Starting from the state tip, the follower compares the
   stored tip hash with the source's hash at that height. A mismatch means the
   tip is not on the source's active chain. The follower then calls the existing
   `JointChainState::rollback_tip`, which pops the existing bounded rollback
   history. If the history runs out first, the call fails with
   `ReorgExceedsHistory`. The follower does not implement its own rollback.
4. **Forward advance.** For each height after the common ancestor, it calls
   `getblockhash`, then `getblock(hash, 0)`. It then runs
   `UnverifiedRawBlockView::parse(raw, Some(hash))`, which checks that the
   SHA256d header hash equals the hash the source named. It then runs
   `inspect_serial()`, which checks the TXID Merkle root, duplicate TXIDs and
   the size and weight bounds. The raw block is handed to the caller's
   `BlockDecoder`. The decoder's operations are bound to the block: each
   `txid` must occur in the block, in increasing position. Finally the block is
   applied with `JointChainState::apply_unverified_block`, which enforces
   height, parent linkage, replay, activation and ledger rules.
5. **Atomicity.** All work happens on a clone of the state. The clone replaces
   the caller's state only when every rollback and every block has succeeded.
   On any failure the state is unchanged, and `checkpoint_bytes()` is
   byte-identical to its value before the call.
6. **Bounded work.** `max_blocks` limits the rollbacks plus applied blocks in
   one call. This also bounds a source that lies about its height. A call that
   exceeds the budget fails and commits nothing. Callers catch up by choosing a
   sufficient budget.

**Restart.** A checkpoint is written with `JointChainState::write_new_checkpoint`
and reloaded with `load_checkpoint`, which requires the expected network,
anchor, admin, permitted set and history limit. The next sync resumes from the
checkpoint tip. The checkpoint format is CRC32-protected only, as documented in
`docs/SINGLE_LEDGER_JOINT_REPLAY.md`. It is not authenticated.

## What is NOT checked

- Proof of work, difficulty, chainwork and whether the source's chain is the
  heaviest active chain. The source's active chain is assumed.
- Header timestamps, coinbase rules, BIP141 witness commitments, UTXOs and
  their existence, scripts and signatures.
- TradeLayer authorship. Actors and operations come from the caller's
  `BlockDecoder`. The joint executor records them as UNVERIFIED caller claims.
- That the node is honest. A lying node can serve a self-consistent false
  chain. The only checks are hash equality, Merkle consistency and linkage.

## The decoder boundary

The crate has no authorized raw-transaction decoder. `docs/CHAIN_STATE.md`
states that the `TrustedChainAdapter` has no production implementation. The
follower therefore takes a caller-supplied `BlockDecoder`, and no default
decoder exists. There is deliberately no "empty operations" fallback, because
such a fallback would advance the tip while silently ignoring TradeLayer
transactions.

## Identifiers and byte order

- `BlockId`, `getblockhash` output and genesis constants use **display** order
  (big-endian hex).
- A block's identity is `reverse(SHA256d(header[0..80]))`. Its parent is
  `reverse(header[4..36])`.
- `TxId` is the SHA256d of the serialized transaction in **internal** order,
  the same value as `raw_block_ingest`'s `observation.txid`. Explorer-style
  txids are the byte reverse of this value.

## Litecoin and Dogecoin

Litecoin block hashes are Scrypt of the header, and Dogecoin uses Scrypt with
AuxPoW. Neither is implemented here, so the follower cannot compare a
Litecoin block's hash with the source's claim. It therefore refuses those
profiles with `HeaderHashUnavailable`. Live follower work for the Litecoin
TradeLayer cannot start until a Scrypt header-hash implementation has been
written and reviewed. That is an explicit blocker, not something this change
works around.

## JSON-RPC client (`src/node_rpc.rs`)

- `RpcBlockSource` implements `BlockSource` using `getblockcount`,
  `getblockhash` and `getblock(hash, 0)`. It uses only `std::net`, with one
  connection per request and `Connection: close`.
- **Endpoint.** The address must be an IP literal. It must be loopback unless
  the caller passes `EndpointPolicy::ExplicitRemoteOptIn`, which sends
  credentials in cleartext.
- **Credentials.** They come from `TRADELAYER_RPC_USER` and
  `TRADELAYER_RPC_PASSWORD`, using HTTP Basic auth. `RpcCredentials` redacts
  its Debug output. Error values carry no credentials, no node message text
  and no response body. The variant `MissingCredential` names the environment
  variable, never its value.
- **Bounds.** The whole HTTP body is capped at 8 MiB and must declare
  `Content-Length`. Chunked encoding is rejected. Headers are capped at 16 KiB.
  A raw block is capped at 4,000,000 bytes, or 8,000,000 hex characters. JSON
  depth is limited to 8, and duplicate keys are rejected. Default timeouts are
  5 s to connect and 30 s for each I/O operation. Both can be changed with
  `with_timeouts`.
- **Envelope.** A non-null JSON-RPC `error` yields only its numeric code. On
  success, the response `id` must be 1 and `result` must be present.

## Tests

- `tests/node_follower.rs` (24 tests). It covers linear advance against a
  straight replay, reorgs of depth 1 and 3 reaching the straight replay of the
  winning branch, failing blocks (including a reorg whose new branch fails),
  checkpoint restart, a lying best height, the per-call budget, short and
  oversized raw blocks, a `getblockhash` versus header-hash mismatch, refused
  profiles and genesis, decoder txid binding, reorgs deeper than the history,
  the HTTP client against a loopback fake server (basic auth end to end, short
  bodies, chunked encoding, oversized Content-Length, lying envelopes,
  oversized block hex, timeouts, refused connections, endpoint policy and
  environment credentials), and credentials absent from all error text.
- `src/node_rpc.rs` unit tests cover base64 and JSON parsing and the
  endpoint policy.
- Guard removal checks. Each guard below was removed in an isolated copy of the
  crate, and the named tests failed:
  - genesis gate: `source_on_a_different_genesis_is_refused`
  - header-hash check: `getblockhash_versus_header_hash_mismatch_is_rejected`
  - atomic commit: `failing_block_leaves_state_byte_identical`
  - loopback guard: `node_rpc::tests::remote_endpoint_requires_explicit_policy`,
    `remote_endpoint_and_missing_env_are_refused_with_safe_messages`,
    `credentials_never_appear_in_any_error_text`
  - response-size cap: `rpc_rejects_oversized_content_length_before_reading_body`

## Not done

- Scrypt header hashing for Litecoin, and Dogecoin AuxPoW.
- Any run against a real Litecoin Core node.
- A production `TrustedChainAdapter` or raw-transaction decoder.
- Crash-consistent durable checkpoints and authenticated checkpoints. These are
  inherited limits from the existing TLJC format.
- Persistent HTTP connections. The client opens one connection per request,
  which keeps the code small.
