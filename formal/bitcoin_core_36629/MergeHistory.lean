import Std

/-!
Multi-script confirmed result aggregation in getscripthistory, step 5, pinned
upstream: bitcoin/bitcoin#36629 at
cf9b981fcc7ae52c7864beae35df5d60b5c322b4.

C++ uses std::map<(height,tx_offset), ScriptHistoryTransaction>,
try_emplace and then appends vout indexes from each script hash lookup.
This module models the map and its first-transaction-wins behavior.

Correct association of appended output indexes relies on a stable mapping
from (height,tx_offset) to the transaction body across all Find calls.
-/
set_option autoImplicit false

namespace BitcoinCore36629.MergeHistory

structure Position where
  height : Nat
  txOffset : Nat
  deriving Repr, DecidableEq

structure Hit where
  pos : Position
  txid : Nat
  voutCount : Nat
  outputIndexes : List Nat
  deriving Repr, DecidableEq

abbrev GroupMap := Position → Option Hit

def emptyMap : GroupMap := fun _ => none

/-- The C++ try_emplace stores the first transaction for a given position.
    Subsequent hits at the same position append matching output indexes. -/
def insert (groups : GroupMap) (hit : Hit) : GroupMap :=
  fun position =>
    if position = hit.pos then
      match groups position with
      | none => some hit
      | some original =>
          some { original with
            outputIndexes := original.outputIndexes ++ hit.outputIndexes }
    else groups position

theorem other_position_unchanged (groups : GroupMap)
    (hit : Hit) (p : Position) (ne : p ≠ hit.pos) :
    insert groups hit p = groups p := by
  simp [insert, ne]

theorem new_position_creates_hit (groups : GroupMap) (hit : Hit)
    (h : groups hit.pos = none) :
    insert groups hit hit.pos = some hit := by
  simp [insert, h]

theorem repeated_position_appends (groups : GroupMap)
    (first second : Hit) (same : second.pos = first.pos)
    (present : groups first.pos = some first) :
    insert groups second first.pos =
      some {first with outputIndexes := first.outputIndexes ++ second.outputIndexes} := by
  simp [insert, same, present]

/-- The C++ aggregation retains the transaction body from the first hit. -/
theorem repeated_position_keeps_first_txid (groups : GroupMap)
    (first second : Hit) (same : second.pos = first.pos)
    (present : groups first.pos = some first) :
    (insert groups second first.pos).map Hit.txid = some first.txid := by
  simp [insert, same, present]

def outputIndexesInBounds (hit : Hit) : Prop :=
  ∀ i ∈ hit.outputIndexes, i < hit.voutCount

/-- With stable transaction identity and output vector length, combining
    independently validated script-hit lists preserves index bounds. -/
theorem safe_same_transaction_union (first second : Hit)
    (sameSize : first.voutCount = second.voutCount)
    (hFirst : outputIndexesInBounds first)
    (hSecond : outputIndexesInBounds second) :
    outputIndexesInBounds
      {first with outputIndexes := first.outputIndexes ++ second.outputIndexes} := by
  intro i hmem
  rcases List.mem_append.mp hmem with hleft | hright
  · exact hFirst i hleft
  · rw [sameSize]
    exact hSecond i hright

/-- A concrete ABA/reorg or inconsistent-record witness: hits from two
    different transactions at the same (height,offset) are merged onto
    the first transaction, potentially appending an out-of-range vout. -/
def oldHit : Hit := ⟨⟨500, 81⟩, 1001, 1, [0]⟩
def newHit : Hit := ⟨⟨500, 81⟩, 2002, 8, [7]⟩

def bounded (hit : Hit) : Bool :=
  hit.outputIndexes.all (fun i => decide (i < hit.voutCount))

theorem conflicting_block_hit_breaks_output_bounds :
    (insert (insert emptyMap oldHit) newHit oldHit.pos).map bounded =
      some false := by decide

end BitcoinCore36629.MergeHistory
