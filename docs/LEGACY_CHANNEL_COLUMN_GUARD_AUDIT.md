# Source-only historical channel column-assignment helper audit

This is a bounded regression for pinned
[`tradelayer.js@dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c)
and [Issue #107](https://github.com/patrickdugan/TradeLayer-Rust/issues/107).
It cannot authenticate any channel state or change consensus behavior.

## Source evidence

| Pinned channel source | Consequence |
| --- | --- |
| `channels.js:809–821` initializes `participants: {'A':'','B':''}` | `participants` is a plain object |
| `channels.js:899` writes `participants[channelColumn]=senderAddress` | A and B remain normal object properties |
| `channels.js:913–917` tests `participants.size === 2` | `size` is `undefined`, even with two nonempty addresses |
| `channels.js:742–749` gates `assignColumns` behind that predicate | This helper's assignment branch is bypassed for the source-built object |
| `channels.js:1169–1176` looks up `this.channels.get` | The persisted registry elsewhere is `this.channelsRegistry` |
| `channels.js:844–846` calls `assignColumnBasedOnAddress`/`resolveColumn` | The direct record-commit path remains separate |

GitHub repository code search on the pinned revision found
`processChannelCommits`, `areBothPartiesCommitted`,
`updateChannelWithColumnAssignments` and `assignColumns` only in
`src/channels.js`. The latter appeared as a call site. Those findings
apply to **this revision**, with deployed source variants still unknown.

## Executable characterization

`scripts/audit_legacy_channel_column_guard.cjs` uses Node's built-in
`assert` to run the literal small plain-object representation and
`participants.size === 2` expression. It shows the two-party predicate
returns false after both A and B have populated addresses, while a
different `Map` representation returns true. Counting object keys alone
is also insufficient because the source initializes both keys empty.
The source helper's mismatched `this.channels` lookup is an independent
reason a one-line repair would be unsafe.

This script deliberately refrains from importing the historical JS
database/channel manager or from replaying any real blocks. It does not
claim actual users failed to commit: `recordCommitToChannel` has
independent column-resolution logic. The test captures a **parity trap**
for future Rust migration, and its result is diagnostic only.

Any repaired consensus behavior requires historical source versions,
actual on-chain/DB records, a complete direct channel-replay trace, and
an explicit activation/version policy. A valid 2-of-2 script or matching
public keys still does not prove current co-signed state, revision
monotonicity, revocation, UTXO inclusion or settlement authority.

### Offline test

```sh
node scripts/audit_legacy_channel_column_guard.cjs
```

Rust zero-third-party Cargo policy and all prior channel/insurance
verifiers remain unchanged. No wallet, node, network, funds or script
authentication is involved.
