# Module 3/49: channels.js deterministic direct-commit parity

**Offline research-only behavioral characterization, not a full production channel port.** Legacy pinned source: [tradelayer.js@dca0888b src/channels.js](https://github.com/patrickdugan/tradelayer.js/blob/dca0888b827c5bc28fa9189bc9e1d507f5c5670c/src/channels.js) (1,181 lines). Rust baseline: [main e96130b5](https://github.com/patrickdugan/TradeLayer-Rust/commit/e96130b522a878200a3f0e31a1e8edcd1e29a4e2).

This module keeps the substantial existing Rust Ledger/Channels and ClearingSandbox untouched. The new legacy_channel_audit.rs is a separate, private-field projection of what the pinned JS **direct recordCommitToChannel path** would do, given an externally CLAIMED positive pubkey/multisig check. It cannot verify source chain inputs and never constructs settlement authority. Zero new third-party Rust crates.

## Source rules and hazards

| JS function or area | This PR | Not covered / explicit source problem |
|---|---|---|
| assignColumnBasedOnLastCharacter | Ported strict ASCII last-character odd/even A/B heuristic, invalid/short fallback A | Does not validate Bitcoin, Litecoin, or Dogecoin addresses |
| tieBreakerByBackChar | Ported suffix scan and default-to-first winner | Caller-selected source addresses, not owner consent |
| predictColumnForAddress | Ported resolved-existing and computed source column selection | Crowded helper's winnerColumn equals original computed column; not a signer proof |
| assignColumnBasedOnAddress | Ported empty/previous/open-slot/crowded branches for source-like two-party records | Calls recordParticipantChange in JS; Rust only appends unverified changes |
| bumpColumnAssignment | Ported true/one-sided swap of *participants and token A/B maps*, or direct owner overwrite; delta reports | Source DOES NOT swap clearLists, payEnabled, pubkey arrays, existing commit records. A crowded third party can displace an existing owner and inherit a column's observed tokens with no co-signed revision |
| recordCommitToChannel | Ported narrow data effects: append commit, exact 1e-8 token atom accumulation, last used column/height, nonempty incoming clearLists replace, payEnabled=true sticks across later false, repeat sender preserves column | No on-chain funding/UTXO/actor proof; cannot infer handleChannelPubkey success; source BigNumber/JS floating-point outside exact bounded atom subset unproven |
| areBothPartiesCommitted | Ported observed false result for source-created plain object participants | Historical helper checks participants.size===2, which is undefined for {A,B}. processChannelCommits also calls undefined assignColumns and updateChannelWithColumnAssignments references this.channels instead of channelsRegistry. See issue #107 |
| getChannelBalancesForAddress | Partial: case-insensitive match of current A/B or last committer fallback, positivity/property filtering and payEnabled history metadata | No actual NeDB query, persisted balances, multisig metadata, floating-point UI sort; a displaced prior committer can see a successor's column |
| savePendingWithdrawalToDB | Ported key format withdrawal-height-sender and same-key last-write-wins across different channels/properties | Full withdrawal/queue recovery and payout rules remain unimplemented |
| addToWithdrawalQueue, processWithdrawals | Only source 7-block threshold / key-collision diagnostic | Actual queue over an array + DB and source bug around block vs blockHeight not promoted into ledger |
| setChannel, addToRegistry, loadChannelsRegistry, saveChannelsRegistry, removeFromRegistry | No production port in this slice | Historical NeDB snapshots, stale cache, full replay and reorg need independent exact-source contract |
| recordChannelDelta, recordTokenTrade, recordContractTrade, saveTrade | Only participant-change observation metadata | No UUID/timestamp trusted record, trade-history DB or economic match proof |
| handleChannelPubkey, getCommitAddresses | Stage25J–O already have Bitcoin research scripts, digest and relative Merkle evidence; this slice has only explicit ClaimedMatch/ClaimedMismatch input | Signed 2-of-2 ECDSA spend is not a signed channel-state update; no authenticated first-input owner, actual multisig or best-chain proof |
| debitInitMarginFromChannel, adjustChannelBalances, processWithdrawal/All, processTransfer | NOT integrated | The source's B-to-A/B transfer branches update destination channel objects rather than indexed token balances. Withdrawal reserve/PNL and historical country/clearlist context are insufficient for trusted settlement |
| isChannelEmpty, removeEmptyChannels, removeChannelFromDB | Not ported | Source removeEmptyChannels invokes removeChannelFromDB without channelAddress; requires compatibility specification |
| updateChannelWithColumnAssignments | Not ported | Source looks up this.channels rather than channelsRegistry and the guard is dead for source-created participant object |
| Canonical signed-channel revision | **Absent in inspected pinned source**, not invented by this PR | Two-signer state bytes, monotonically numbered state, authenticated both-party agreement, funding outpoint binding, revocation, commitment invalidation, expiry, dispute and rollback rules require ratification |

## Research API, tests, limitations

UnverifiedLegacyChannel is private-field, chain-agnostic research state. apply_claimed_commit takes raw claimed sender, channel, atoms, properties, clearLists, pay flag, claimed block height and TXID and a claimed pubkey preflight result. All input claims remain **unauthenticated**. An allegedly matching pubkey preflight does not imply the historical first-input or channel script is valid. Atomic clone-before-commit refuses invalid input or balance overflow without modifying the observation. Exact integer atoms rather than Number/BigNumber rounding are an intentional narrower compatibility subset.

The methods require_authenticated_spend, require_canonical_signed_revision and require_trusted_settlement **always return errors**. UnverifiedWithdrawalKeys likewise always denies verified payout. No TrustedProtocolAdapter/ledger/channel/clearing/insurance modifications are made.

The independent dependency-free Node source-expression script scripts/audit_legacy_channels_source.cjs exercises A/B first two committers, duplicate commits, sticky clearLists/payEnabled, full-column crowding and displacement, A/B swap carrying token balances but not policy, the broken participants.size guard and colliding persisted withdrawal IDs. The Rust test suite tests those same observations and null authorization, plus invalid sender, amount, multisig-preflight claim, counterparty, source fallback balances and 7-block maturity. Source tests are deterministic mock behavior, not an actual historical multi-signer transaction.

This slice stops deliberately before token debits/credits. A historically faithful buggy owner overwrite is useful for audit but **must not** be adopted as a future consensus rule. Distinguish a source discrepancy from a repaired protocol amendment, and do not put either into production until a chain-specific activated, ratified compatibility/version boundary has been established.

## Remaining requirements for a complete channels.js migration

- Exact active-chain authenticated funding/spend with Bitcoin/Litecoin/Dogecoin-specific script and UTXO rules, whitelist Type4/19 administrative state and accepted-sender provenance, full channel cache/persistence reorg rollback.
- A canonical co-signed offchain revision envelope and monotone state version, tx/balance commitment, both key signatures, supersession/invalidated transaction proof, timeout/refund, dispute and checkpoint commitment. A 2-of-2 on-chain funding or spend alone does not authenticate the TradeLayer state inside the channel.
- Audited future economic rules for partial closes, forced liquidation, AMM-first matching, oracle/VWAP mark, margin collateral, insurance and ADL; absent those, no settlement permission.
- Historical JS permission and rounding semantics pinned to transaction-specific versions, plus safe replacement for crowded-column overwrites, broken helper, B-transfer destination-object arithmetic and the withdrawal key collision.
- Unit-parity, adversarial, node/reorg/signet test vectors with authenticated branch evidence and no genesis fund inflation.

### Offline verification

    node scripts/audit_legacy_channels_source.cjs
    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_channel_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

Module 3/49 is deterministic-source-partial until these obligations are fulfilled. The next source file to address is clearing.js.
