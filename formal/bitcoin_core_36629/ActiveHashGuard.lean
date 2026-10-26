import ForkAwareMerge
import ReorgSchedule
import Std

/-!
An additional hypothetical RPC postcondition for bitcoin/bitcoin#36629:
check each confirmed result's block hash against the active-chain block
hash at that height immediately before returning.

At pinned commit cf9b981fcc7ae52c7864beae35df5d60b5c322b4
getscripthistory checks only ActiveChain().Tip() == captured_tip. It does
not perform per-matched-block hash validation.

This model proves that post-validation excludes stale confirmed block
references after an A -> B -> A reorg, while retaining the intended
current-chain results. It is an alternative defensive design, not an
implementation or a proof of atomic spentness/mempool state.
-/

set_option autoImplicit false

namespace BitcoinCore36629.ActiveHashGuard
open BitcoinCore36629.ForkAwareMerge
open BitcoinCore36629.MergeHistory

abbrev ActiveChainImage := Nat → Option Nat

def activeConfirmed (chain : ActiveChainImage) (hit : ForkHit) : Prop :=
  chain hit.tx.pos.height = some hit.blockHash

def validConfirmed (chain : ActiveChainImage) (hits : List ForkHit) : Prop :=
  ∀ hit ∈ hits, activeConfirmed chain hit

def activeAtReturn (chain : ActiveChainImage) (hit : ForkHit) : Bool :=
  decide (activeConfirmed chain hit)

def checkAll (chain : ActiveChainImage) (hits : List ForkHit) : Bool :=
  hits.all (activeAtReturn chain)

theorem check_all_iff (chain : ActiveChainImage) (hits : List ForkHit) :
    checkAll chain hits = true ↔ validConfirmed chain hits := by
  induction hits with
  | nil => simp [checkAll, validConfirmed]
  | cons hit rest ih =>
      simp only [checkAll, List.all_cons, Bool.and_eq_true,
        activeAtReturn, decide_eq_true_eq]
      constructor
      · rintro ⟨hhead, htail⟩
        intro h hh
        rcases List.mem_cons.mp hh with rfl | more
        · exact hhead
        · exact ih.mp htail h more
      · intro all
        constructor
        · exact all hit (by simp)
        · apply ih.mpr
          intro h hh
          exact all h (by simp [hh])

/-- A current-chain result is accepted. -/
def activeA : ActiveChainImage :=
  fun height => if height = 500 then some 101 else none

/-- The old fork is active at final response time; the B fork is stale. -/
theorem valid_old_hash_after_aba :
    checkAll activeA [oldFork] = true := by decide

theorem reject_stale_fork_after_aba :
    checkAll activeA [oldFork, newFork] = false := by decide

/-- End-tip pointer equality alone accepts the same A -> B -> A sequence. -/
theorem anchored_guard_stronger_on_aba :
    BitcoinCore36629.ReorgSchedule.sameEndTip
      BitcoinCore36629.ReorgSchedule.stateA
      BitcoinCore36629.ReorgSchedule.stateBack = true ∧
    checkAll activeA [oldFork, newFork] = false := by decide

/-- A successful post-validation gives active-chain membership at the
    instant of validation for every confirmed transaction in the result. -/
theorem accepted_results_reference_active_blocks
    (chain : ActiveChainImage) (hits : List ForkHit)
    (h : checkAll chain hits = true) :
    ∀ hit ∈ hits, chain hit.tx.pos.height = some hit.blockHash := by
  exact (check_all_iff chain hits).mp h

end BitcoinCore36629.ActiveHashGuard
