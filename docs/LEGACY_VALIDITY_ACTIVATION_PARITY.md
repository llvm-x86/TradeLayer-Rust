# Pinned `Validity.isActivated` versus `activation.js`: source parity

Historical baseline:
[`tradelayer.js@dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

**Finding:** `src/validity.js:83–94` contains an activation-by-height
branch entirely inside a JavaScript block comment. The live function
returns `true` unconditionally. Calls to **that helper** do not enforce
the activation registry on this pinned source revision.

This differs from `src/activation.js:245–255`
(`isTxTypeActive` reads the current `active` flag, without a height
constraint), and from the independent stricter Rust
`src/registry.rs:280–286` (a transaction must occur **after**, not
in the same block as, its recorded activation). It also differs from
`src/activation.js:257–263` height storage queries.

The Node source-characterization regression
`scripts/audit_legacy_validity_activation.cjs` checks this
three-way difference on inactive, active, unknown and height-zero
transaction types, before/same/after activation heights, and reactivation
height overwrite. It is intentionally *not* a Rust end-to-end test; the
Rust registry's stricter rule is already exercised by its separate
unit tests. No full JS runtime or historical chain data is imported.

**Why isolated:** Source files may represent different deployed eras.
Other validation paths can use direct activation flags or distinct
checks. Unconditional `Validity.isActivated` cannot establish that
every transaction was actually valid, and a stricter Rust activation
rule cannot be described as equivalent to the pinned helper. Genuine
migration needs authenticated historical events, code/version
activation, native Type-0 atomic genesis with insurance, DB persistence
and rollback, and independent as-of sender/admin evidence.

**Absolutely no authority is added.** The existing
`legacy_activation_audit.rs` and Type4/19/Type9 research reports remain
non-authorizing; `registry.rs` continues to apply its independent
stricter rules. No code path is connected to Ledger, clearing, channels,
live wallet, node or settlement. This is an independently executable
source-parity counterexample, not a consensus policy change.

Offline:
```sh
node scripts/audit_legacy_validity_activation.cjs
```
