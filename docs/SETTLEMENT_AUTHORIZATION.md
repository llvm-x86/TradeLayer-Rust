# Stage 13: Default-deny economic authorization for channel settlement

The Stage 9 unified adapter is responsible for authenticating network
headers, transaction inclusion and order, txid, script/UTXO spend authority,
and the decoding of TradeLayer payloads. Stage 12 introduced economic
transitions which additionally require independent evidence of the
underlying trade, clearing rights, price, and authorized payout.

## Authenticated block replay

The TrustedProtocolAdapter trait now has a separate
authenticate_settlement_economics method. It is invoked for every
ChannelNetSettle (type 23 NET_SETTLE) or ChannelKingSettle (type 31)
returned by authenticate_and_decode, before that transaction can
enter the staged block replay. The default rejects these operations.

SettlementEvidenceContext passes the exact decoded operation,
claimed txid and channel actor, raw source transaction bytes, chain
namespace and block header. A production verifier must bind every
field to authenticated trade and settlement evidence.

- Ordinary L1 validation alone cannot authorize these two transitions
  in UnifiedChainState.
- Implementations explicitly returning success assume responsibility
  for the extra economic proof verification.
- Rejection is returned as UnifiedChainError::SettlementAuthorization,
  separate from ordinary blockchain authentication failures.
- Rejected blocks preserve tip, replay markers, registry, balances,
  pending withdrawals and settlement journals, including any earlier
  operations in the same block.
- Direct ProtocolState::apply_unverified remains simulation-only. It
  bypasses both chain authentication and economic authorization.

## A production verifier must establish

For type 23, independently verify the original trade, participants,
property, exact PnL, payout direction, amount, expiration, signed
trade status and neutralization rules.

For type 31, independently establish the authorized settlement
range, genuine references, the net obligation, settlement-root
semantics, and the absence of previously settled claims.

The existing SHA-256 settlement root establishes internal consistency
over claimed data. Correct hashes do not prove the original economics.
If Cairo/STWO is used, an adapter needs an actual verifier for proof
bytes with fully bound public inputs and independently checked roots.
The present zkTL bridge verifies metadata consistency only.

## Regression coverage

A labelled synthetic adapter checks a fixed allowlist of expected
network, raw fixture, block height and ID, and entire decoded
operation. It demonstrates the new gate and is not economic
verification. Tests cover default denial, operation/amount/body/block
substitution, explicit test approval, and atomic failure of a mixed
block after earlier valid transactions.

See tests/channel_settlement.rs and docs/CHANNEL_SETTLEMENT.md.
Production integration remains blocked pending external verification.
