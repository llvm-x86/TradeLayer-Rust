# Stage 8D5: raw funding-transaction evidence before dependency-wave probes

**Status: offline consistency only; no Bitcoin consensus or UTXO proof.**

This increment builds directly on [Stage 8D4](STAGE8D4_PARENT_UTXO_WITNESS.md)
and [Stage 8D3](STAGE8D3_READONLY_WAVE_PRECHECK.md). Its code lives in
[`src/parent_funding_evidence.rs`](../src/parent_funding_evidence.rs)
and adds
[`select_unverified_transaction_output`](../src/read_only.rs)
to the existing strict Bitcoin raw-transaction reader. It neither
allocates TradeLayer ledger state nor grants a trusted protocol adapter.

## The concrete gap closed

Stage 8D4 accepts a **caller-provided** table of previous Bitcoin outputs,
plus a fingerprint that the same caller can recompute for arbitrary value/
script fields. This stage additionally requires the **actual serialized raw
funding transaction bytes** for each *distinct* external funding TXID.
The parser computes each transaction's canonical stripped SHA256d TXID,
extracts the referenced outputs by index and compares exact satoshi values
and scriptPubKey bytes with every row in the supplied candidate snapshot.

This is strictly stronger *internal byte consistency*: a claimant cannot
change just an amount or script in the candidate table and still match the
same funding TXID preimage. A liar can, however, invent a different funding
transaction, update all its dependent transaction references and synthesize
consistent data. The result is still not a Bitcoin UTXO proof.

## Input and denial rules

1. Stage 8D4's complete parent-candidate snapshot validation runs **first**,
   including exact current-block preindex replay, fork identity, prevout
   coverage, double-spend and dependency validation.
2. The funding witness set has one **whole raw transaction** per distinct
   external prevout TXID, in increasing order by its computed *raw-wire*
   SHA256d TXID. Multiple referenced outputs from the same raw funding
   transaction share that one witness; duplicate and out-of-order sources
   fail.
3. Funding raw transactions must be valid canonical transaction encodings
   according to the existing strict legacy/SegWit syntax reader. Every
   output's amount is bounded by Bitcoin MoneyRange, including the
   transaction's total value. The parser returns a *borrowed script slice*,
   rather than allocating a second copy of each funding script.
4. For each external input, match the exact funding output index, value
   and script. Missing, extra, malformed, duplicate, unsorted, wrong value,
   wrong script and out-of-range output witnesses fail. Combined raw
   evidence is bounded to 16 MiB, with each transaction bounded by the
   existing one-megabyte read-only parser limit.
5. Same-block parent outputs are **deferred**, not represented in the
   external snapshot. Stage 8D3 wave barriers remain unverified scheduling
   hints. A failed raw-funding audit cancels the whole composed result
   before **any** wave-probe callback is invoked.

An exact successful match carries status
`raw_funding_prevout_bytes_consistent_unverified`.
Typed results permanently refuse funding inclusion, authenticated current
UTXO state, Bitcoin signature/script validity, and Bitcoin/TradeLayer
economic application.

## SegWit subtlety

The funding *TXID* intentionally omits SegWit witness bytes. A funding
transaction with altered witness data can have the **same** TXID, amount
and scriptPubKey. The test suite checks this case. The spending block's
own preindex still checks wtxid equality, but verifying **funding** witness
commitments and historical inclusion requires further authenticated
Bitcoin block evidence.

A Bitcoin header contains neither a canonical, universal UTXO-set root
nor a proof that an arbitrary outpoint remains unspent. Consequently the
stronger raw-funding byte correspondence alone cannot establish ownership,
maturity, chain membership, signatures, Script success or funding origin.

The output's `is_coinbase` and `created_height`
fields are likewise mere snapshot claims; they have not been checked
against original funding-block transactions and chain history. Even an
exact raw funding transaction can be falsely labeled coinbase by a
claimant. The test suite explicitly demonstrates that the audit remains
unverified in such a case.

## Test evidence

The targeted Rust test matrix includes:

- Three claimed external outputs matched against a **single** serialized
  funding transaction, across worker counts 1/2/3/4/8/16.
- Two distinct funding transactions, exact increasing TXID order,
  duplicate funding transaction denial and missing/extra funding evidence.
- A funding transaction's precise 105-byte canonical serialization,
  cross-checked against an independent Python SHA256d digest:
  `01adfa763d8cb8bdf850869cd34797535ecd02ff76ff6ee7d28e97d35e85ffd8`
  (digest in *raw hashing* byte order, not explorer order).
- Mutated claimed amount/script with recomputed snapshot checksum,
  changed funding bytes, malformed/truncated raw TX, nonexistent vout,
  parent fork mismatch, and callback suppression on incomplete snapshots.
- A SegWit funding witness change that alters wtxid but not TXID.
- A pure callback rejection that still cancels the composed probe,
  and permanent denial of all trusted consensus and economic operations.

~~~bash
cargo test --frozen --test stage8d_raw_funding_evidence
python3 scripts/audit_stage8d5_raw_funding.py
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
~~~

No Bitcoin Core RPC, private keys, third-party Rust crates, network service,
VPS computation or actual zk proving is required for this development slice.

## What must come next

Real consensus admission requires independently authenticated Bitcoin
funding-block inclusion and active-chain ancestry, a parent UTXO accumulator
or replay proof, spentness and maturity, BIP141 witness commitments,
correct per-input sighash flags and BIP143/BIP341 message construction,
valid signatures and actual Script execution. Same-block outputs must
only become available *after* their originating transaction passes all
checks. Finally, an ordered state applier must atomically mutate UTXO and
TradeLayer state with complete rollback on a stronger-chain reorg.

None of those obligations is established by these tests or the
`*_unverified` status.
