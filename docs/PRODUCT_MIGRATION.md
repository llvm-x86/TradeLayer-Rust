# Product migration plan: TradeLayer services on the Rust node

**Status:** plan, no code. Nothing here is activated; the README's "cannot process real funds" still holds.

## Why this document exists

The wallet, relayer, collator, orderbook, extension and explorer do not link against any protocol library.
They talk to a node over JSON-RPC (`tl-rpc`, `tl-relayer`, `tl-collator`). Moving the product to Rust therefore means
**a Rust node that serves the same RPC surface**, not porting each service. The inventory below is a grep of the
`tl_*` identifiers in the contributor-hardened service repos (143 distinct strings, unfiltered; a few are not RPC names).

## Gaps found in this crate (verified against `main` at `41007c5`)

| Need | State |
|---|---|
| Enumerate balances by address | `Ledger::available_atoms(id, account)` only; no iteration by account or by property |
| Property listing | `Ledger::property(id)` only; no enumeration |
| Live block feed | `raw_block_ingest` and `bitcoin_core_rpc_stream.py` are offline/evidence tooling |
| Persistence | checkpoint bytes (`TLJP`) only; no database |
| Network RPC server | none; the supply-chain policy admits no crates, so a server must be `std::net` |
| Tx building (`tl_createpayload_*`, `tl_createrawtx_*`) | `wire_v1` has type-2 only |
| Channels, derivatives, margin, AMM, options, insurance | unported (`JS_RUST_PORT_AUDIT.md`) |

## Phases, each shippable and each gated by tests against the JS reference

1. **Read accessors.** Deterministic iterators on `Ledger` and `Registry` (sorted, no hash-order leaks). Unit tests plus a snapshot round-trip.
2. **`tradelayer-rpcd` read-only.** `std::net` JSON-RPC 2.0 over a restored `TLJP` checkpoint, loopback by default, request-size and connection caps, no write methods.
   First methods: `tl_getinfo`, `tl_getproperty`, `tl_listproperties`, `tl_getbalance`, `tl_getallbalancesforaddress`, `tl_getsyncstatus`.
   Gate: a conformance fixture per method generated from the JS node, diffed byte-for-byte on the JSON.
3. **Live follower.** Block feed from a Litecoin Core node into `JointChainState`, reorg handling through the existing rollback history. Gate: replay of a pinned mainnet range reaches the JS node's state root.
4. **Payload builders** (`tl_createpayload_simplesend`, then attestation, then trade payloads). Gate: byte-identical to `tradelayer.js` for fixed inputs.
5. **Channels** (commit, withdraw, `tl_getchannel*`), then clearing and derivatives. These are the product's differentiator and the largest unported block.
6. **Cutover.** Run both nodes in shadow mode behind the collator, compare responses, switch when diffs are zero for a release window.

## Rules for every phase

* Integer atomic units on the wire; no floats in consensus or in RPC output that a client will parse as money.
* Untrusted RPC input is parsed with bounded sizes before any allocation scales with it.
* `scripts/ci-preflight.sh --full` before every push; no weakened CI.
* No dependency without the policy revision in `security/RUST_SUPPLY_CHAIN.md`.

## Raw inventory: `tl_*` identifiers by service repo (unfiltered)

| identifier | used in |
|---|---|
| `tl_api_service_file` | tl-explorer |
| `tl_autoroll_out` | UTXO-Ref |
| `tl_balance_source` | UTXO-Ref |
| `tl_base_url` | UTXO-Ref |
| `tl_beta_gate_md_out` | UTXO-Ref |
| `tl_beta_gate_out` | UTXO-Ref |
| `tl_bitvmstatus` | tl-collator |
| `tl_block_12345` | UTXO-Ref |
| `tl_block_12350` | UTXO-Ref |
| `tl_block_` | UTXO-Ref |
| `tl_broadcast_chain` | UTXO-Ref |
| `tl_broadcast` | UTXO-Ref |
| `tl_chain` | UTXO-Ref |
| `tl_challenger_custody` | UTXO-Ref |
| `tl_challenger_owner` | UTXO-Ref |
| `tl_challenger_pubkey` | UTXO-Ref |
| `tl_channelbalanceforcommiter` | tl-relayer, tl-collator |
| `tl_check_commits` | tradelayer-wallet |
| `tl_check_kyc` | tradelayer-wallet |
| `tl_collateral_sats` | UTXO-Ref |
| `tl_collator_cors_origins` | tl-collator |
| `tl_collator_enable_legacy_tl_proxy` | tl-collator |
| `tl_collator_rpc_advert_ttl_ms` | tl-collator |
| `tl_collator_rpc_network` | tl-relayer, tl-collator |
| `tl_collator_rpc_service` | tl-relayer, tl-collator |
| `tl_collator_url` | tl-relayer |
| `tl_contractposition` | tl-relayer, tl-collator |
| `tl_contracttradehistoryforaddress` | tl-relayer, tl-collator |
| `tl_creategrantmanagedtokentransaction` | UTXO-Ref |
| `tl_createpayload_attestation` | tl-relayer, tl-collator, tradelayer-wallet |
| `tl_createpayload_commit_tochannel` | tl-relayer, tl-collator, tradelayer-wallet |
| `tl_createpayload_contract_instant_trade` | tradelayer-wallet |
| `tl_createpayload_instant_ltc_trade` | tradelayer-wallet |
| `tl_createpayload_instant_trade` | tradelayer-wallet |
| `tl_createpayload_sendactivation` | tradelayer-wallet |
| `tl_createpayload_sendvesting` | tradelayer-wallet |
| `tl_createpayload_simplesend` | tradelayer-wallet |
| `tl_createpayload_` | tradelayer-wallet |
| `tl_createpayload_withdrawal_fromchannel` | tradelayer-wallet |
| `tl_createrawtx_opreturn` | tl-relayer, tradelayer-wallet |
| `tl_createrawtx_reference` | tl-relayer, tradelayer-wallet |
| `tl_db_profile` | UTXO-Ref |
| `tl_decodetransaction` | tl-relayer, tradelayer-wallet |
| `tl_decodetx` | tl-relayer |
| `tl_deposit_tx_001` | UTXO-Ref |
| `tl_dlc_contract_id` | UTXO-Ref |
| `tl_dlc_hash` | UTXO-Ref |
| `tl_dlc_next_contract_id` | UTXO-Ref |
| `tl_dlc_settlement_state` | UTXO-Ref |
| `tl_dlc_template_id` | UTXO-Ref |
| `tl_duration_seconds` | UTXO-Ref |
| `tl_emergency_recovery_custody` | UTXO-Ref |
| `tl_emergency_recovery_owner` | UTXO-Ref |
| `tl_emergency_recovery_pubkey` | UTXO-Ref |
| `tl_external_reviewer` | UTXO-Ref |
| `tl_external_review_owner` | UTXO-Ref |
| `tl_external_review_scope_defined` | UTXO-Ref |
| `tl_force_expired` | UTXO-Ref |
| `tl_force_test` | UTXO-Ref |
| `tl_full_suite_passed` | UTXO-Ref |
| `tl_funding_address` | UTXO-Ref |
| `tl_funding_txid` | UTXO-Ref |
| `tl_funding_vout` | UTXO-Ref |
| `tl_getallbalancesforaddress` | tl-relayer, tl-collator, tradelayer-wallet, tl-explorer, UTXO-Ref |
| `tl_getattestations` | tl-relayer, tl-collator |
| `tl_getbalance` | tl-relayer, tl-collator |
| `tl_getchaininfo` | tl-explorer |
| `tl_getchannelcolumn` | tl-relayer, tl-collator |
| `tl_getchannel` | tl-relayer, tl-collator |
| `tl_getclearlistbyid` | tl-relayer, tl-collator |
| `tl_getcontractinfo` | tl-relayer, tl-collator |
| `tl_getfullposition` | tradelayer-wallet |
| `tl_getinfo` | tl-relayer, tl-collator, tradelayer-wallet |
| `tl_getinitmargin` | tl-relayer, tl-collator |
| `tl_getmaxsynth` | tl-relayer, tl-collator |
| `tl_getproperty` | tl-relayer, tl-collator, tradelayer-wallet, tl-explorer |
| `tl_getstatesnapshot` | tl-relayer, tl-collator |
| `tl_getsyncstatus` | tl-relayer, tl-collator |
| `tl_gettop10blocks` | tl-explorer |
| `tl_gettransactionsforaddress` | tl-explorer |
| `tl_gettransactionsforblock` | tl-explorer |
| `tl_gettransaction` | tl-relayer, tl-collator, tl-explorer |
| `tl_isaddresswinner` | tradelayer-wallet |
| `tl_listcontractseries` | tl-relayer, tl-collator |
| `tl_listener_url` | tl-relayer |
| `tl_listnodereward_addresses` | tradelayer-wallet |
| `tl_listoracles` | tl-explorer |
| `tl_listproperties` | tl-relayer, tl-collator, tl-explorer |
| `tl_max_per_contract_sats` | UTXO-Ref |
| `tl_max_total_loss_sats` | UTXO-Ref |
| `tl_modules` | tradelayer-wallet |
| `tl_observed_at_unix` | UTXO-Ref |
| `tl_oracle_admin_address` | UTXO-Ref |
| `tl_oracle_id` | UTXO-Ref |
| `tl_oracle_privkey_hex` | UTXO-Ref |
| `tl_oracle_pubkey_hex` | UTXO-Ref |
| `tl_pause_policy` | UTXO-Ref |
| `tl_payload_hash` | UTXO-Ref |
| `tl_previous_dlc_contract_id` | UTXO-Ref |
| `tl_procedural_admin_address` | UTXO-Ref |
| `tl_propertyfeecache` | tl-explorer |
| `tl_rbtc_property_id` | UTXO-Ref |
| `tl_redeem_tx_001` | UTXO-Ref |
| `tl_relay_compat_upstream_url` | tl-collator |
| `tl_relay_db` | UTXO-Ref |
| `tl_relayer_rpc_route_source` | tl-collator |
| `tl_relayer_url` | tl-relayer |
| `tl_relay_replica_root` | UTXO-Ref |
| `tl_relay_type` | UTXO-Ref |
| `tl_require_relayer` | tl-collator |
| `tl_reserve_artifact` | UTXO-Ref |
| `tl_reserve_operator_custody` | UTXO-Ref |
| `tl_reserve_operator_owner` | UTXO-Ref |
| `tl_reserve_operator_pubkey` | UTXO-Ref |
| `tl_skip_rpc_boot` | UTXO-Ref |
| `tl_smoke_address` | tl-collator |
| `tl_starting_rbtc_balance` | UTXO-Ref |
| `tl_starting_rbtc_units` | UTXO-Ref |
| `tl_starts_at_unix` | UTXO-Ref |
| `tl_state_oracle_custody` | UTXO-Ref |
| `tl_state_oracle_owner` | UTXO-Ref |
| `tl_state_oracle_pubkey` | UTXO-Ref |
| `tl_suite_count` | UTXO-Ref |
| `tl_test_address_a` | UTXO-Ref |
| `tl_test_address_b` | UTXO-Ref |
| `tl_tlbtc_property_id` | UTXO-Ref |
| `tl_tokentradehistoryforaddress` | tl-relayer, tl-collator |
| `tl_totaltradehistoryforaddress` | tl-relayer, tl-collator |
| `tl_tx_001` | UTXO-Ref |
| `tl_tx30_anchor_artifact` | UTXO-Ref |
| `tl_tx30_anchor_broadcast` | UTXO-Ref |
| `tl_tx30_anchor_out` | UTXO-Ref |
| `tl_vault_address` | UTXO-Ref |
| `tl_vault_artifact` | UTXO-Ref |
| `tl_wallet_listener_url` | tl-collator |
| `tl_wallet_unspent` | UTXO-Ref |
| `tl_watchonly_bootstrap_url` | tl-relayer |
| `tl_watchonly_register` | tl-relayer, tl-collator |
| `tl_watchonly_registry_seed_url` | tl-relayer |
| `tl_watchonly_utxos` | tl-relayer, tl-collator |
| `tl_watchtower_guardian_custody` | UTXO-Ref |
| `tl_watchtower_guardian_owner` | UTXO-Ref |
| `tl_watchtower_guardian_pubkey` | UTXO-Ref |
