import Std

/-!
The last step of getscripthistory in bitcoin/bitcoin#36629 checks:
  chainman.ActiveChain().Tip() != tip
and raises an RPC error if the tip pointer changed while the query ran.

This executable model formalizes the precise *scope* of the tip equality guard.
It does not assert that the C++ RPC promises an atomic mempool snapshot.
Pinned upstream: cf9b981fcc7ae52c7864beae35df5d60b5c322b4.
-/
set_option autoImplicit false

namespace BitcoinCore36629.RPCGuard

structure View where
  chainTipPointer : Nat
  mempoolRevision : Nat
  deriving Repr, DecidableEq

def accepts (before after : View) : Bool :=
  decide (before.chainTipPointer = after.chainTipPointer)

theorem rejects_changed_tip (before after : View)
    (changed : before.chainTipPointer ≠ after.chainTipPointer) :
    accepts before after = false := by
  simp [accepts, changed]

theorem accepts_equal_tip (before after : View)
    (same : before.chainTipPointer = after.chainTipPointer) :
    accepts before after = true := by
  simp [accepts, same]

/-- The actual end guard checks tip identity but does not compare mempool
    epochs. This is a specification boundary, not a demonstrated exploit. -/
def earlier : View := ⟨101, 5⟩
def later : View := ⟨101, 6⟩

theorem distinct_mempool_view_still_accepted :
    earlier ≠ later ∧ accepts earlier later = true := by decide

/-- A path that changes tip and then returns to its original pointer passes
    the endpoint-only check. C++ can observe different chain snapshots during
    such a path, so proving coherent per-step reads needs additional facts. -/
def intermediate : View := ⟨102, 5⟩

theorem transient_tip_excursion_not_detected :
    earlier.chainTipPointer ≠ intermediate.chainTipPointer ∧
    accepts earlier later = true := by decide

end BitcoinCore36629.RPCGuard
