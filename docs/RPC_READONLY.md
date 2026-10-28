# Read-only JSON-RPC server (`tradelayer-rpcd`): DRAFT

**Status: DRAFT.** Method names come from the product migration inventory.
Request and response shapes here are this crate's own choices. They are
not diffed against the JavaScript or C++ node, which this repository does
not contain, so **no parity claim is made**. Before any client relies on
them, capture node fixtures and diff the JSON byte-for-byte.

The server serves a restored `TLJP` checkpoint. It has no live block feed
(that is phase 3) and cannot write state. Nothing here authorizes a
transaction or proves the checkpoint's contents.

## Run

```
tradelayer-rpcd --checkpoint <file.tljp> \
  --network <bitcoin|litecoin|dogecoin> \
  --admin <account> \
  --permitted <id[,id...]> \          # may be empty: --permitted ""
  [--bind 127.0.0.1:7360] \           # placeholder default port, not a parity claim
  [--allow-nonloopback]
```

- Network, admin and permitted properties are deployment policy. They are
  checked against the checkpoint and are never read from it.
- A non-loopback `--bind` (any address other than `127.0.0.0/8` or `::1`)
  is refused with exit code 2 unless `--allow-nonloopback` is passed.
- Configuration errors exit 2. Checkpoint or bind failures exit 1.

## Transport

- One request per TCP connection, then `Connection: close`.
- `POST /` only. Other methods get 405; other paths get 404.
- HTTP/1.0 and HTTP/1.1 request lines. Headers end with CRLF CRLF; bare LF is refused.
- `Content-Length` is required (411 if missing). It must be a single
  decimal value of at most 20 digits. Anything else is 400.
- `Transfer-Encoding` of any kind is refused with 501. Chunked bodies are not read.
- The declared length is checked against the 64 KiB cap before any body byte is read or any buffer is sized from it. Over the cap: 413.
- Header block over 8 KiB: 431.
- Whole-request deadline of 5 s, covering head and body. Slower requests get 408.
- At most 16 handler threads. The 17th concurrent connection gets 503 immediately.
- Responses: `200 OK` with `Content-Type: application/json` for every JSON-RPC
  reply, including JSON-RPC errors. HTTP-level failures carry an empty body.

## JSON parsing limits

| Limit | Value |
| --- | --- |
| Request body | 64 KiB (`MAX_REQUEST_BODY_BYTES`) |
| Container nesting depth, objects and arrays together | 8 |
| Total JSON values per request | 512 |
| Decoded string length | 1024 bytes |
| Number token length | 32 bytes |

Duplicate object keys are rejected. Strings must be valid UTF-8, and lone surrogates are rejected.

## JSON-RPC envelope

- Only `"jsonrpc": "2.0"` is accepted.
- `id` is required and must be a string, an integer, or `null`.
- Batch arrays are **rejected** with -32600. They are not supported.
- Notifications (requests without `id`) are **rejected** with -32600. They are not silently ignored.
- `params` must be a positional array. Object params get -32602.
- Unknown top-level members are ignored.
- Responses have a fixed key order: `jsonrpc`, then `result` or `error`, then `id`.
  The `id` is echoed verbatim, so responses are byte-reproducible.

| Code | Meaning |
| --- | --- |
| -32700 | Parse error (malformed JSON) |
| -32600 | Invalid request (envelope, batch, notification, limits, duplicate keys) |
| -32601 | Method not found (includes every write-shaped name) |
| -32602 | Invalid params (wrong arity, wrong type, invalid address, zero or out-of-range property id) |
| -32001 | Property not found (server-defined code, outside the reserved JSON-RPC ranges) |

## Value encoding

- Money and balances are JSON strings of unsigned integer atoms, in decimal. Never a float.
- Property ids, heights and counts are JSON numbers.
- `finalized_height` is a number, or `null` when the checkpoint carries no finalized height.
- Address parameters must be non-empty printable ASCII of at most 90 bytes (`AccountId`). This is a format check only, not an address validity check.

## Methods

Params are positional. Every method rejects a wrong argument count with -32602.

### `tl_getinfo`

Params: `[]`

```json
{"jsonrpc":"2.0","result":{"network":"litecoin","finalized_height":42,"property_count":2,"processed_count":0},"id":1}
```

### `tl_getsyncstatus`

Params: `[]`. Reports what the checkpoint carries. There is no live feed.

```json
{"jsonrpc":"2.0","result":{"source":"checkpoint","finalized_height":42,"processed_count":0},"id":5}
```

### `tl_listproperties`

Params: `[]`. Ascending property id.

```json
{"jsonrpc":"2.0","result":[{"propertyid":7,"issuer":"alice","kind":"fixed","total_supply_atoms":"1000"},{"propertyid":9,"issuer":"bob","kind":"managed","total_supply_atoms":"500"}],"id":2}
```

### `tl_getproperty`

Params: `[propertyid]`, a positive integer that fits in u32. Unknown id: -32001.

```json
{"jsonrpc":"2.0","result":{"propertyid":7,"issuer":"alice","kind":"fixed","total_supply_atoms":"1000"},"id":"req-1"}
```

`kind` is `"fixed"` or `"managed"`.

### `tl_getbalance`

Params: `[address, propertyid]`. Unknown property: -32001. Unknown account: balance `"0"`.

```json
{"jsonrpc":"2.0","result":{"address":"carol","propertyid":7,"balance":"250"},"id":3}
```

The balance is the stored **available** balance, as `Ledger::available_atoms` reports it. Channel and settlement liabilities are not included.

### `tl_getallbalancesforaddress`

Params: `[address]`. Ascending property id, available balances only.

```json
{"jsonrpc":"2.0","result":[{"propertyid":7,"balance":"250"},{"propertyid":9,"balance":"5"}],"id":4}
```

## Not covered

- Parity with the JavaScript or C++ node (see the status note above).
- Live Litecoin, Bitcoin or Dogecoin block ingestion or reorg handling (phase 3).
- Transaction building or broadcast, and any write method. Those are out of scope by design.
- Authentication, TLS, rate limiting beyond the connection cap, and logging.
  The server assumes a trusted local operator. Loopback is the only exposure
  it is designed for.

## Tests

`tests/rpc_server.rs` covers exact response bytes for every method, the
malformed-JSON, envelope, param and batch cases, each HTTP cap, a loopback
socket round trip, the 503 connection cap, the 408 read timeout, the bind
policy, and the binary running on loopback. Each security cap was also
checked by removing or weakening it in a scratch copy and confirming the
targeted test fails.
