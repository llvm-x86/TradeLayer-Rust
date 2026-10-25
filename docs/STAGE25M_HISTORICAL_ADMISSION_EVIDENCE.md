# Stage 25M: historical type-4/type-19 admission characterization

**Research-only source characterization, on caller-supplied synthetic direct-attestation histories.** This stage does not authenticate a blockchain event, historical database, address, country, multisig channel, revocable state, mark, executed fill or settlement.

Baseline: [TradeLayer-Rust main 36256462](https://github.com/patrickdugan/TradeLayer-Rust/commit/3625646263fd00cb56d53cc1a2a7e11e9791fb41), after Stage25L #97 (legacy P2SH two-signer ECDSA fixture) and #98 (relative Bitcoin P2WSH TXID-Merkle evidence). Pinned JavaScript: [tradelayer.js dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

## What the pinned source says

| Predicate | Pin | Source behavior and important caveat |
|---|---|---|
| Type-4 property whitelist | src/validity.js:473–653, especially 552–563, 610–654 | The **committing sender** must occur in at least one of the property's whitelist IDs, except the source's scalar whitelistId == 0 bypass. Uses **direct** ClearList.isAddressInClearlist. Historical property registry/whitelist state remains unauthenticated. |
| Type-4 optional commitment clearlist | src/validity.js:642–652 | An optional **positive** commitClearlistId checks the same sender via isAddressInClearlistOrDerived. Null and zero impose no extra requirement. Stage25M requires a *direct* positive witness for this subset; missing direct membership is **unproven**, not a verdict on derived membership. |
| Type-4 additional checks | src/validity.js:478–641 | Claimed channel output/ref, address, positive amount, sufficient TallyMap balance, activation, bans/country and accreditation, vesting token 2/3 prohibition, restricted procedural token, pubkeys/channel-address consistency and list type validation. None is proven by list membership alone. |
| Type-19 channel clearlist | src/validity.js:1804–1930 | If channel.clearLists.A or .B declares lists, concatenate **both** columns; direct or xpub-derived membership of **either** committer suffices. The check uses getCommitAddresses(sender) from the claimed registry. An empty union bypasses only this particular rule. |
| Type-19 additional checks | src/validity.js:1804–2030 | Contract, amount/price, reference/leverage, activation, expiry, sender/channel identity, ban/country, margin and collateral. This slice characterizes direct list membership and asserted identity/expiry only. |
| Direct and derived attestations | src/clearlist.js:147–189,220–275,415–449,561–570 | Direct membership checks active status and list ID. Derived path searches active xpub/merkle-root records and expands bounded HD addresses. Historical xpub derivation/roots remain unverified. |
| Activation ambiguity | src/validity.js:83–94 and src/activation.js | Pinned Validity.isActivated() **returns true unconditionally** (previous block check commented out). activationInstance.isTxTypeActive(4/19) remains a separate predicate. Neither is proven by a direct attestation. |
| Historical DB collision risk | src/clearlist.js:147–170,175–189 | addAttestation and addAttestationWithXpub upsert by _id = address even though list ID lives inside the row. Different list attestations on one address may overwrite the same DB record. Database tip is not a lossless verified history. |
| Channel data vs signed revision | src/channels.js:365–400,754–912 | Channel records have participants, columns, pubkeys, commits, block heights, balances and deltas. handleChannelPubkey extracts the first funding input's key and recomputes multisig addresses in both orders. The inspected routines supply no canonical two-signature off-chain update transcript, atomic supersession proof, revocation-key path or signed state sequence. |

## Bounded read-only audit API

The Rust module src/channel_admission_audit.rs accepts a *claimed* network, exact (height, transaction index, block ID) cutoff and a sorted sequence of claimed **direct** attestation or revocation events with network, block ID, transaction index and synthetic TXID. It refuses:

- Events from the future or the same transaction as the query; memberships later in the same block cannot be used early.
- Conflicting claimed block IDs at one height, duplicate transaction positions, unordered histories, malformed fields and foreign network claims.
- Directly revoked sender membership, failed optional type-4 commitment list membership, and type-19 lists with neither participant directly witnessed.
- Inconsistent type-19 sender/channel claims, duplicate participants, expired claims, and type-4 whitelist arrays containing zero. JavaScript loose equality for arrays containing 0 is **not silently promoted to a consensus rule**.

For **type 4**, a property whitelist is explicitly scalar zero or one or more positive IDs. An optional positive commit clearlist is independent. For **type 19**, the source A+B list union and either-party OR are preserved for direct members. A derived-only address reports NoDirectChannelWitness, a missing *direct evidence* condition rather than an assertion that the historic source rejected the transaction.

A matching source-rule observation produces UnverifiedHistoricalAdmissionSlice with a claimed timestamp and optional direct matched list/side. Calling require_authenticated_channel_admission() **always returns IndependentAdmissionProofUnavailable**. Calling require_canonical_signed_revision() **always returns CanonicalChannelRevisionUnspecified**. This holds with both sides in a list, no declared list, or apparently matching signatures. There is no connection to Ledger, clearing, TLCL/TLCC, UTXO/chain trust, type3/23/31 or liquidation.

The reported direct replay treats synthetic event history as complete for the supplied direct subset. A historical absence proof cannot be inferred from an empty fixture, database tip or purported header. NoDirect* errors mean supplied evidence is insufficient.

## Remaining independently verified gates

1. Obtain a bounded trusted block/transaction source with consensus ancestry and activation, correct TXID/index, chronological events, authenticated property metadata and source-authorized clearlist creation/attest/revoke. A supplied block hash is a label until independently proven.
2. Reconstruct historical country/ban and direct+xpub-derived memberships from independently replayable inputs. Prove any merkle/xpub roots and derived address family; investigate DB address-key collision before relying on snapshots.
3. Authenticate original type-4 sender/output/reference, signed channel funding and UTXO existence/unspent status, two pubkeys and correct script, alongside type-19 sender/participants, chronological commits, contract/ref price, expiry and collateral.
4. Ratify a **signed-state revision and commitment invalidation protocol**. Pinned JS does not provide a canonical signed revision/sighash/revocation transcript in the inspected code. Specify domain separation, state bytes, channel funding outpoint, signer order, revision order, supersession/revocation and dispute rules with reorg effects before considering issuance of a state capability. Synthetic ECDSA signatures from prior stages cover a particular Bitcoin-style **transaction digest**, not an off-chain TradeLayer update.
5. Independently prove current oracle/native mark, AMM-first safe matched liquidity and ratified exact rounding/bankruptcy, insurance/ADL economics before economic replay.

## Reproduce

    python3 scripts/check_rust_supply_chain.py
    cargo test --frozen --test channel_admission_audit
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen
    cargo build --all-targets --release --frozen

No new external crates, keys, wallets, nodes, testnet, paid services or liquidation activation. Stage23 and formal draft branches are untouched.
