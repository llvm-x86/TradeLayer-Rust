# Stage 21: Three independent TradeLayers and Bitcoin-only Raito/Cairo

**Network topology:** Bitcoin TradeLayer, Litecoin TradeLayer and Dogecoin
TradeLayer are three independent protocol deployments. Each has its own
genesis-identified chain namespace, block history, registry activations,
property issuance, replay set, ledger, state roots and checkpoint history.
They share Rust protocol implementation code, and there is no canonical
cross-chain TradeLayer, global ordering service, or pooled ledger.

## Source-pinned mainnet genesis identities

| TradeLayer | Consensus parent | Mainnet genesis hash (display order) |
|---|---|---|
| Bitcoin | Bitcoin SHA256d | `000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f` |
| Litecoin | Litecoin Scrypt | `12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2` |
| Dogecoin | Dogecoin Scrypt, including later AuxPoW rules | `1a91e3dace36e2be3bf030a65679fe821aa1d6ef92e7c9902eb318182c355691` |

Pinned upstream references, all mainnet assertions:
[Bitcoin Core chainparams at 152a553](https://github.com/bitcoin/bitcoin/blob/152a55397e451fed9c10041bddf01aa6839847d5/src/kernel/chainparams.cpp);
[Litecoin Core chainparams at ec1b648](https://github.com/litecoin-project/litecoin/blob/ec1b6489a900d09cf5991e220dce089c77a232a2/src/chainparams.cpp);
[Dogecoin Core chainparams at 3c1679e](https://github.com/dogecoin/dogecoin/blob/3c1679e0fd4a99f5d640c317671ad9c0f341ea65/src/chainparams.cpp).

`TradeLayerMainnet` is a closed enum with three stable numeric tags. Its
`chain_id()` is the pinned mainnet genesis hash in canonical display byte
order. This stage introduces `NetworkAssetId { network, property }` to
make explicit that property 7 on Bitcoin and property 7 on Litecoin are
distinct assets with independent issuance and balances.

The genesis-height anchor of every new profile-bound instance must match
its pinned genesis. A higher-height anchor remains a **caller claim**. No
profile tag, hash comparison or checkpoint CRC independently verifies the
active chain or high-height checkpoint provenance.

## Proving capability is per-network and deliberately asymmetric

| Feature | Bitcoin TradeLayer | Litecoin TradeLayer | Dogecoin TradeLayer |
|---|---|---|---|
| Underlying L1 PoW | SHA256d | Scrypt | Scrypt / AuxPoW |
| Current in-crate L1 validation | Partial offline Bitcoin header work and observed forks | No independent Scrypt verifier | No independent Scrypt/AuxPoW verifier |
| Full node / best chain authentication | Not yet integrated | Trusted Litecoin Core required | Trusted Dogecoin Core required |
| Raito/Cairo Bitcoin L1 research path | Eligible (Bitcoin only) | Explicitly unavailable | Explicitly unavailable |
| Cairo/STWO *economic* proof verification | Metadata auditing only | Metadata auditing only | Metadata auditing only |
| Economic settlement authorization | Default deny | Default deny | Default deny |

Scrypt's ROMix RAM shuffle and memory-hard lookup workload make the
present Raito/Cairo Bitcoin-oriented SHA256d pipeline unsuitable as a
Litecoin/Dogecoin proof backend. Dogecoin's merged-mining AuxPoW validation
also requires distinct consensus logic.

`require_raito_research_eligibility` succeeds only for the Bitcoin
profile, indicating that Bitcoin is the only supported **research
integration target**. It does not return a verified-proof receipt.
`economic_proof_authorizes_settlement` is false for every profile.
Existing zkTL receipt and TLPT hashes remain audit metadata until
independently authenticated L1/economic verifiers are available.

## New versioned profile-bound replay API

`NetworkBoundUnified` wraps exactly one `UnifiedChainState`, retaining
the existing registry+ledger and settlement authorization rules.
`NetworkBoundSpot` wraps exactly one `SpotChainState`, preserving the
experimental type-5/6 matching and escrow/reorg behavior.

Both wrappers derive their inner `ChainId` from the selected pinned
genesis, and keep the network enum private and immutable.

`TLNP\x01` is a new, optional **profile and lane envelope** around
existing inner checkpoint formats:

```text
TLNP(5) | network_tag(1) | replay_lane(1) |
mainnet_genesis(32) | ChainId(32) |
inner_checkpoint_bytes_length(4, big endian) |
TLUC or TLSC bytes | CRC32(4)
```

The replay lane is `UnifiedProtocol` or `SpotResearch`. A TLUC and a
TLSC from the same network cannot be interchanged, and a Litecoin TLNP
cannot restore a Bitcoin or Dogecoin instance. Restoration requires a
**caller-supplied expected network** and also validates the nested
checkpoint's chain ID, expected anchor, history policy, and protocol admin
or permitted token policy. Canonical re-encoding and nested CRC protect
against accidental corruption and ambiguous byte layouts.

The old TLUC and TLSC formats retain backward compatibility within their
original unverified APIs. The profile-bound wrappers never silently import
one of those unlabelled checkpoints. Migrating an old deployment into a
named mainnet requires external chain provenance and an explicit one-time
migration process. The new TLNP magic never appears in pre-Stage-21 files.

`checkpoint_fingerprint_unverified` hashes the canonical envelope under
`TLR_PROFILE_CHECKPOINT_AUDIT_V1`. This is a read-only audit fingerprint,
whose inputs include network, lane and nested rollback states. It is
**not** a canonical economic state root, Bitcoin validity proof,
cryptographic attestation or a proof of full chain consensus.

Checkpoints use immutable `create_new` file writes and file fsync.
Methods that apply a block and checkpoint stage the entire instance and
publish the new in-memory state only after the file succeeds.

## Security and implementation boundaries

- Separate mainnets cannot replay each other's blocks because the profile
  wrapper constructs its own genesis-derived ChainId and never accepts a
  caller-supplied ChainId for initialization.
- Property identifiers can coincide numerically across chains while
  `NetworkAssetId` distinguishes their economic ownership domains.
- The wrappers are currently **unverified replay coordinators** and the
  in-crate trusted chain adapters remain interfaces. A synthetic adapter
  that accepts everything is safe only as an explicitly labelled test.
- **Within one network**, the unified registry+ledger lane and the Stage 20
  spot lane remain separate experimental accounting simulations. Running
  both as spendable engines would double-count balances; a versioned
  *single-ledger, single-replay-domain* migration is still required.
- No protocol bridge is inferred from two independent property IDs, Merkle
  inclusion, a claimed high-height checkpoint or a Raito research target.
  Any future cross-chain bridge would be an explicit, separately authenticated
  protocol with its own conservation and finality proofs.

## Reproduce

```sh
cargo test --locked --test network_profiles
python3 scripts/audit_network_profiles.py
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Source regression tests cover original mainnet hash bytes; three separate
registries and ledgers using identical local transaction and property IDs;
profile-bound replay/rollback; cross-chain checkpoint rejection; wrong
genesis, protocol admin or policy; spot-vs-unified lane rejection; immutable
checkpoint-write failure; and Raito eligibility limited to Bitcoin.

The next research step is to test replay semantics against
confirmed Litecoin TradeLayer.js transaction history while keeping Bitcoin
and Dogecoin histories and proving support independent.
