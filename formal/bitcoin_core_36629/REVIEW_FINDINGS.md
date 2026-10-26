# Source-grounded concurrency finding: getscripthistory and index reorgs

Review target: bitcoin/bitcoin#36629
Pinned commit: cf9b981fcc7ae52c7864beae35df5d60b5c322b4
Research branch: patrickdugan/TradeLayer-Rust draft PR #143
Status: formally exhibited *conditional* interleaving; full-node reachability pending.

## Relevant C++ source facts

At the pinned commit:

1. BaseIndex::BlockUntilSyncedToCurrentChain() checks whether the index has
   caught up or drains pending validation callbacks through
   SyncWithValidationInterfaceQueue(). It is called before the RPC's
   multi-script lookup. The method returns a Boolean readiness result, and
   does not hand out a query-wide stable index/chain snapshot.

2. getscripthistory captures a tip pointer under cs_main and then calls
   g_txoscriptindex->Find separately for each expanded script hash.

3. Each TxoScriptIndex::Find constructs its own LevelDB iterator, reads an
   index key and offset list, obtains a CBlockIndex for that height under
   cs_main, and reads candidate transactions before checking full script hash.

4. The RPC's std::map<std::pair<int32_t,uint32_t>,ScriptHistoryTransaction>
   groups by (height, transaction byte offset). try_emplace preserves the
   first transaction body; a second script hash hit at the same position
   appends output indexes without checking transaction identity.

5. JSON verbosity >= 1 annotates the retained decoded_outputs.at(index).
   An incompatible later output index can throw std::out_of_range. This is
   an error-path/correctness issue, not evidence of unsafe memory access.

6. The final guard compares ActiveChain().Tip() with the starting pointer.
   A chain-tip path A -> B -> A passes this endpoint equality test.

Pinned source Git blobs, independently fetched and checked by CI:
- src/index/base.cpp: 160c5fa8cd676d205106768fe4cc07c317e0829b
- src/rpc/blockchain.cpp: 3e045b87d6dcfd417994f5967c5be2442f85f492
- src/index/txoscriptindex.cpp: b7d26cd7206a6e3e4290ec4594e914b8ad2f32f5

## Lean evidence

ReorgSchedule.lean models fully synchronized index states A, B, then A,
with an RPC reading two script hashes in the first two states.

The kernel verifies that both individual reads satisfy their then-current
ready-chain and transaction-output constraints, the endpoint tip guard
accepts, and merging the two matching-height/offset hits can produce an
invalid output index.

The proof assumes callbacks and rewinds can complete between the two
independently called Find operations. It does not establish the exact
scheduling against bitcoind's threaded runtime.

MergeSnapshot.lean proves first-transaction-wins merging is safe when all
inputs come from ONE immutable (height, offset) transaction view.

ForkAwareMerge.lean proves that adding block_hash to the map's identity
prevents distinct-fork hits from being merged onto one retained
transaction body, provided each block hash and transaction offset
determines one immutable transaction and output count.

ActiveHashGuard.lean proves that a final active-chain block-hash check
rejects the stale B result in A -> B -> A and that a success certifies
each confirmed hit's block is active at check time. The conjunction of
fork-aware merging and active hash membership is proved structurally
safe for confirmed matches under their stated assumptions.

## Narrow C++ regression evidence

cpp_reorg_oracle.cpp compiles a small standalone reproduction using the
source-shaped std::map::try_emplace, output-index append, and vector::at
logic. It checks:
- source-shaped grouping produces one entry containing an invalid index;
- the verbose vector::at call throws out_of_range;
- grouping by (block_hash,height,offset) yields two individually valid hits;
- final tip equality accepts an A -> B -> A path;
- a hypothetical monotone epoch guard rejects that path.

This is an extraction test. It does not execute Bitcoin Core's complete
RPC or reproduce the real callback timing.

## Review/mitigation direction

Add a deterministic regtest execution test with a pause between successive
script-hash Find calls. Change the active chain from A to B, wait for
txoscriptindex to finish its validated callbacks, perform the second Find,
restore A and wait for callbacks before allowing the RPC to finish.

Construct the A and B transactions so both occupy the same height and
serialized transaction offset; their output counts should differ and the
second query should match an output index absent from the first
transaction. Require the RPC to return a coherent confirmed history or a
clear retry error. It must never append second-fork output indexes to
the first-fork transaction.

Candidate defensive changes, each with a separate verification scope:

- Include block_hash in the confirmed aggregation identity, so different
  forks cannot donate indexes to a retained transaction with a different
  body. This addresses the specific index-poisoning issue.
- Check that every confirmed result's (height,blockhash) belongs to the
  active chain before returning, under one final cs_main critical section.
  This excludes stale-fork confirmed results even when the endpoint tip
  returns to its original value.
- If full query consistency across reorgs and changes in the UTXO view is
  required, use an explicit monotonic chain revision or an immutable
  snapshot token. Initial/final tip-pointer equality cannot certify this.
- Treat mempool consistency as a separate contract; the chain tip guard
  does not track mempool revisions.

The full implementation still needs a source-level regression and
security/resource review. The 50 MiB accounted-work limit also excludes
some full block and undo reads during verbose RPC serialization, as
independently observed in upstream PR review.

## Reproduce

    cd formal/bitcoin_core_36629
    lake build --wfail
    python3 scripts/check_cpp_parity.py
    python3 scripts/check_reorg_oracle.py
    python3 scripts/check_source_contracts.py

No Bitcoin Core C++ source or TradeLayer Rust runtime logic is changed.
