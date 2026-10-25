# Module 4/49: validity.js conditional source admission predicates

Pinned historical source: [tradelayer.js@dca0888b src/validity.js](https://github.com/patrickdugan/tradelayer.js/blob/dca0888b827c5bc28fa9189bc9e1d507f5c5670c/src/validity.js), 3,796 lines / 170,301 bytes at the inspected revision. Rust baseline: [TradeLayer-Rust main 23e2406e](https://github.com/patrickdugan/TradeLayer-Rust/commit/23e2406e9df230f4d93f2643e43e1bcf527c9f7c).

**Status: bounded deterministic source predicate subset, NOT a full validity.js port, historical acceptance verifier, or transaction execution.** No existing ledger, registry, chain adapter, channel, oracle, AMM, options, insurance/ADL, liquidation or settlement policy is modified. All positive results are conditioned on untrusted supplied dependency values, and every authorization method always returns a denial.

## Source-verified compatibility findings

The source has TWO distinct activation paths. Validity.isActivated(block, txid, txType) at lines 83–94 has a commented-out temporal test and ends with an **unconditional return true**. The upstream activationInstance.isTxTypeActive(7/8/9) is a separate registry-active check and can still make those validators return valid=false. A true result from the unconditional stub cannot authenticate a block, prove historical type activation, or bypass the registry predicate.

Type 0 validateActivateTradeLayer at lines 113–162 maps array elements using JavaScript Number and filters values with !isNaN, then checks that values lie in 0..35. It **does not require integers** despite its error message. Fractional 1.5 can pass this validator, while the later Activation.activate() invokes parseInt and truncates. The new Rust observation models the already Number-coerced numeric-array subset and flags fractional source acceptance. Scalar/object/string coercion is not claimed and type-0 native token genesis remains unimplemented.

Type 7 validateCreateWhitelist at lines 883–920 requires the separate active flag; backupAddress must be a truthy string, but name need only be of JS string type (empty is allowed). If the library validateAddress rejects, the wrapper fallback accepts either camelcase **isValid** or lowercase **isvalid**. These wrapper results are caller claims in Rust, not a real address checksum.

Type 8 validateUpdateAdmin at lines 921–1000 requires active flag and *exactly one* of whitelist/oracle/token; the newAddress need only be of string type. Unlike Type 7, its wrapper fallback checks ONLY lowercase **isvalid**, ignoring a true camelcase isValid. The whitelist branch calls ClearList.getList(id), which reads **attestations** under numeric ID and checks adminAddress/backupAddress. Ordinary Type 7 creation instead stores **clearlists**, with admin/backup, and Type 8 updateAdmin writes to a stringified ID in clearlists. This new source predicate distinguishes Missing/Present claimed numeric attestation rows from canonical list registry records and does NOT assert that an arbitrary positive row exists in the historic DB. Oracle/token branches are explicitly UnmodeledSourceContext, not accepted or rejected by guesswork.

Type 9 validateIssueOrRevokeAttestation at lines 1003–1099 separately requires type-active flag, tries to replace the target from a specified output ref, and rejects an unresolved ref. For **positive list IDs**, it reads the current clearlist via getClearlistById (clearlists collection), checks only the primary adminAddress or admin field, and requires a current active direct attestation for revocation. The backup alone cannot issue or revoke. For **list zero**, a sender may self-certify a matching own target with an ASCII two-letter country code; a BANLIST tag (trimmed/case-insensitive target) requires the claimed protocol administrator and a nonempty list of ASCII two-letter country codes parsed from commas, whitespace, semicolons or vertical bars. A missing active attestation also rejects an attempted revoke on list zero. All list, protocol-admin, current-attestation and output-resolution facts are caller-supplied and unverified.

## Implemented API and trust boundaries

The new src/legacy_validity_audit.rs exposes functions:
- source_validity_is_activated_stub: literal source unconditional Boolean true, never a trusted chain query;
- source_validate_type0_array_unverified: narrow numeric array, NaN filtering, admin equality, 0..35, fractional truncation warning;
- source_validate_type7_unverified: registry active flag, backup and name types, Type 7 camel/lower address-wrapper behavior;
- source_validate_type8_unverified: registry active flag, exactly one selected admin target, the ATTESTATIONS getList field names and source Type 8 lowercase-only wrapper; oracle/token target checks unresolved;
- source_validate_type9_unverified: source separate primary, list-zero country and BANLIST, claimed ref-to-output override, current direct revocation condition.

The returned private-field UnverifiedValidityObservation provides one of three explicit dispositions: RejectedBySourcePredicate, MatchedUnderUnverifiedClaims or UnmodeledSourceContext. It exposes narrow diagnostics, source caveats, claimed ref target, parsed BANLIST country codes and the sanitized Type-0 numeric array. The "matched" state is NOT a valid transaction or admission certificate. External claims are NOT authenticated; the source validator is not a consensus definition. Methods require_authenticated_transaction(), require_accepted_historical_administration() and require_authorized_channel_settlement() ALWAYS fail with HistoricalInputAndStateUnverified, HistoricalType8AdminStateUnverified or CanonicalSignedRevisionUnavailable respectively.

This stage never supplies a proof token or mutates existing Registry/Execution/Ledger/Clearing. In particular, source whitelist matching does not grant a channel participant permission or resolve the Type-8 database collision from Module 2/49.

**Deliberate subset limitations:** typed string-valued target/meta and optional claimed ref resolution; Type0 inputs must already be JavaScript Number-coerced array members; extra country/BANLIST whitespace behavior is restricted to Rust-supported space classification; object country metadata, actual Bitcoin/Litecoin/Dogecoin addresses, xpub derivation and complete source exceptions need independent suites. Source-isActivated returning true does not mean a real transaction has passed the JS call stack.

## Coverage matrix for the remaining validators

| Source validators / methods | Current status |
|---|---|
| isActivated and validateActivateTradeLayer / Type 0 | **Source predicate characterized**, no native token genesis or actor proof |
| validateCreateWhitelist / Type 7 | **Source predicate characterized**, no real address or full list-ID allocation |
| validateUpdateAdmin / Type 8 whitelist | **Source predicate characterized**, historical getList mismatch unresolved |
| validateUpdateAdmin / Type 8 oracle/token | **Explicitly unmodeled**; dependent OracleList/PropertyList/managed-token checks remain |
| validateIssueOrRevokeAttestation / Type 9 | **Source predicate characterized** for positive direct lists, list zero country/BANLIST and claimed ref |
| isValidNumber and isRestrictedProceduralToken | Unported to this module; existing Rust arithmetic and procedural policy are not source-complete |
| validateTokenIssue / Type 1 | Unported full eligibility, ticker, issuer, backup, amount, procedural/vesting constraints |
| validateSend / Type 2 | Unported source multisend, balances, restricted properties and reference validation |
| validateTradeTokenForUTXO / Type 3 | Unported; source sometimes resets valid=true after earlier failure and has index/payment fallback quirks |
| validateCommit / Type 4 | **Partial Stage25M/N/O source rule diagnostics**; complete source validity not ported |
| validateOnChainTokenForToken / Type 5, validateCancelOrder / Type 6 | Rust standalone spot primitives exist, but full historical validator source parity remains |
| validateAMMPool / Type 10 | Source-parity safety and pool-state proof incomplete |
| validateGrantManagedToken / Type 11, validateRedeemManagedToken / Type 12 | Manage/issuer/vesting/amount validation unported |
| validateCreateOracle / Type 13, validatePublishOracleData / Type 14, validateCloseOracle / Type 15 | Partial registry prototype, not complete JS acceptance |
| validateCreateContractSeries / Type 16, validateExerciseDerivative / Type 17, validateTradeContractOnchain / Type 18 | Partial contract/clearing research, no complete historical acceptance |
| validateTradeContractChannel / Type 19, validateTradeTokensChannel / Type 20 | Stage25M claimed list and channel evidence only; authentic co-signed revision missing |
| validateWithdrawal / Type 21, validateTransfer / Type 22 | Rust ledger primitives and source audit, not full validator/authorization |
| validateSettleChannelPNL / Type 23 | Rust clearing research only, no authenticated PNL acceptance |
| validateMintSynthetic / Type 24, validateRedeemSynthetic / Type 25 | Unported full CDP collateral/mark/synthetic validity |
| validatePayToTokens / Type 26, validateOptionTrade / Type 27 | Unported full economics and options lifecycle |
| validateTradeBaiUrbun / Type 28, validateTradeMurabaha / Type 29 | Unported fiqh-labelled financial contract source |
| validateStakeFraudProof / Type 30 | Unported accepted oracle/fraud proof behavior |
| validateKingSettle / Type 31 | Source contains multiple declarations/aliases around lines 3509–3652; parse/runtime binding must be pinned, not guessed |
| validateBatchMoveZkRollup / Type 32 | ZK research claims only, not a deployed proof verifier |
| Types 33–35 | No source-complete validator paths established in this audit |
| Full catch/error propagation, native adapter, snapshot/reorg recovery | Unported; actual signed scripts, source version/serialization, network and state provenance required |

## Verification and next steps

Independent Node script scripts/audit_legacy_validity_0789.cjs characterizes literal JavaScript source expressions without importing the real DB/RPC/bitcore stack. It exhaustively enumerates eight combinations of the source address library and camel/lower wrapper booleans, as well as Type0 fractional acceptance, missing Type8 lookup and primary/backup fields, Type9 positive direct rows, missing ref, country self-cert and BANLIST branches. Rust tests in tests/legacy_validity_audit.rs assert the corresponding predicate observations, source quirks, explicit unknowns, all three authority denials and adversarial missing-context combinations.

    node scripts/audit_legacy_validity_0789.cjs
    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_validity_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

No new Rust dependencies, actual wallets, keys, nodes, real funds, testnet, production state changes or activation. **File 4/49 remains incomplete for production** until all ~3,800 source lines, transaction types and source-bound historical input/state authorization have independent evidence. The next file in the P0 migration sequence is types.js, followed by the decoder and encoder layers.
