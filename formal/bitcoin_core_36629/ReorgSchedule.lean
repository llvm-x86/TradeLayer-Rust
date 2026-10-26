import MergeSnapshot
import RPCGuard
import Std
import Lean.Elab.Tactic.Omega

/-!
Controlled reorg/snapshot interleaving for bitcoin/bitcoin#36629 at
cf9b981fcc7ae52c7864beae35df5d60b5c322b4.

Source-oriented obligations:
  BaseIndex::BlockUntilSyncedToCurrentChain() provides a readiness check,
    not a lease on the active chain, index records, or transaction bodies.
  getscripthistory calls Find separately for each queried script hash and
    groups results by (height, transaction byte offset).
  A final ActiveChain().Tip() != tip rejects unequal *endpoint* tips.
  An A -> B -> A tip excursion can have the same endpoint tip pointer.

The transitions below represent fully synchronized points AFTER the relevant
BlockConnected/Rewind/CustomAppend callbacks. The model does not establish
that a particular execution reaches them in deployed C++ without an external
scheduler test; it proves what the guards logically imply if it does.

An epoch/revision guard is a proposed stronger protocol, NOT a field that
already exists on Bitcoin Core's active chain.
-/

set_option autoImplicit false

namespace BitcoinCore36629.ReorgSchedule
open BitcoinCore36629.MergeHistory

structure IndexedView where
  activeTip : Nat
  indexedTip : Nat
  chainEpoch : Nat
  atPosition : Snapshot

def Ready (s : IndexedView) : Prop := s.activeTip = s.indexedTip

/-- Successful synchronized observations keep the same position/tx invariant
    inside one coherent view. -/
def Readable (s : IndexedView) (hit : Hit) : Prop :=
  Ready s ∧ ValidHit s.atPosition hit

/-- One validated reorg plus a completed index callback.
    This represents the state AFTER callbacks have been processed. -/
def indexedTransition (s : IndexedView) (newTip : Nat)
    (newView : Snapshot) : IndexedView :=
  { activeTip := newTip
    indexedTip := newTip
    chainEpoch := s.chainEpoch + 1
    atPosition := newView }

/-- Repeated reorg transitions are indexed in their declared order. -/
def replay (s : IndexedView) : List (Nat × Snapshot) → IndexedView
  | [] => s
  | (newTip, newView) :: rest =>
      replay (indexedTransition s newTip newView) rest

theorem transition_ready (s : IndexedView) (newTip : Nat)
    (newView : Snapshot) :
    Ready (indexedTransition s newTip newView) := rfl

theorem transition_increments_epoch (s : IndexedView) (newTip : Nat)
    (newView : Snapshot) :
    (indexedTransition s newTip newView).chainEpoch =
      s.chainEpoch + 1 := rfl

theorem replay_epoch (s : IndexedView)
    (changes : List (Nat × Snapshot)) :
    (replay s changes).chainEpoch = s.chainEpoch + changes.length := by
  induction changes generalizing s with
  | nil => simp [replay]
  | cons change remaining ih =>
      rcases change with ⟨newTip, newView⟩
      simp only [replay, List.length_cons]
      rw [ih]
      simp only [indexedTransition]
      omega

/-- Endpoint chain-tip comparison (the source RPC's final check). -/
def sameEndTip (before after : IndexedView) : Bool :=
  decide (before.activeTip = after.activeTip)

/-- Suggested monotone-epoch comparison catches ABA transitions. -/
def sameEpoch (before after : IndexedView) : Bool :=
  decide (before.chainEpoch = after.chainEpoch)

theorem unchanged_epoch_means_no_indexed_transitions (s : IndexedView)
    (changes : List (Nat × Snapshot))
    (equalEpoch : (replay s changes).chainEpoch = s.chainEpoch) :
    changes = [] := by
  have hc := replay_epoch s changes
  have hlen : changes.length = 0 := by omega
  exact List.length_eq_zero_iff.mp hlen

theorem epoch_gate_sufficient_for_stable_view (s : IndexedView)
    (changes : List (Nat × Snapshot))
    (equalEpoch : (replay s changes).chainEpoch = s.chainEpoch) :
    (replay s changes).atPosition = s.atPosition := by
  have empty := unchanged_epoch_means_no_indexed_transitions s changes equalEpoch
  subst changes
  rfl

def stateA : IndexedView :=
  { activeTip := 101, indexedTip := 101
    chainEpoch := 7, atPosition := oldView }

def stateB : IndexedView := indexedTransition stateA 202 differentView

def stateBack : IndexedView := indexedTransition stateB 101 oldView

/-- Both individual Find invocations may observe an index fully synchronized
    to the *then-current* chain, while returning different block bodies at the
    same (height,tx offset). -/
theorem independent_queries_each_synchronized :
    Readable stateA oldHit ∧
    Readable stateB newHit ∧
    Ready stateBack := by
  constructor
  · simp [Readable, Ready, ValidHit, stateA, oldView,
      oldHit, outputIndexesInBounds]
  constructor
  · simp [Readable, Ready, ValidHit, stateB, indexedTransition,
      differentView, newHit, outputIndexesInBounds]
  · rfl

/-- The PR's endpoint tip guard accepts the A -> B -> A trace. -/
theorem end_tip_check_accepts_aba :
    sameEndTip stateA stateBack = true := by decide

/-- The proposed revision guard detects the same trace. -/
theorem revision_check_rejects_aba :
    sameEpoch stateA stateBack = false := by decide

/-- The two individually valid reads, when inserted into the source's
    first-transaction-wins map, yield an invalid output index. -/
theorem conflicting_snapshot_merge_failure :
    (aggregate [oldHit, newHit] oldHit.pos).map bounded =
      some false := by decide

/-- In this model a caller can pass the source's final tip guard and retain
    a map with an index beyond the retained transaction's vout count. -/
theorem aba_accepted_with_invalid_merged_output :
    Readable stateA oldHit ∧
    Readable stateB newHit ∧
    sameEndTip stateA stateBack = true ∧
    (aggregate [oldHit, newHit] oldHit.pos).map bounded = some false := by
  exact ⟨independent_queries_each_synchronized.1,
         independent_queries_each_synchronized.2.1,
         end_tip_check_accepts_aba,
         conflicting_snapshot_merge_failure⟩

/-- With the stronger epoch equality, the common Snapshot premise required
    by MergeSnapshot.aggregate_sound follows for any sequence of completed
    index transitions. -/
theorem stable_query_view_under_equal_epoch (s : IndexedView)
    (changes : List (Nat × Snapshot))
    (equalEpoch : (replay s changes).chainEpoch = s.chainEpoch)
    (hits : List Hit)
    (valid : ∀ hit ∈ hits, ValidHit s.atPosition hit) :
    StableGroups s.atPosition (aggregate hits) := by
  have _ := epoch_gate_sufficient_for_stable_view s changes equalEpoch
  exact aggregate_sound s.atPosition hits valid

end BitcoinCore36629.ReorgSchedule
