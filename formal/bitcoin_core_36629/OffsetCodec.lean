import Std
import Lean.Elab.Tactic.Omega

/-!
Execution-side model of the delta-offset list stored in the LevelDB value of
Bitcoin Core bitcoin/bitcoin#36629 (pinned upstream SHA:
cf9b981fcc7ae52c7864beae35df5d60b5c322b4).

Source: src/index/txoscriptindex_key.h, DBValue::Serialize/Unserialize.
This module models the integer *deltas* between transaction offsets, not the
underlying C++ VARINT byte layout. The C++ decoder rejects empty values, zero
deltas, out-of-block offsets and excessive transaction counts.
-/
set_option autoImplicit false

namespace BitcoinCore36629.OffsetCodec

/-- A sequence of strictly increasing nonzero offsets within the block. -/
def WellFormed (maxSize prev : Nat) : List Nat → Prop
  | [] => True
  | off :: rest =>
      prev < off ∧ off < maxSize ∧ WellFormed maxSize off rest

/-- Encoder's delta recurrence: first previous offset is zero. -/
def deltas (prev : Nat) : List Nat → List Nat
  | [] => []
  | off :: rest => (off - prev) :: deltas off rest

/-- Decoder's cumulative-sum recurrence. -/
def undeltas (prev : Nat) : List Nat → List Nat
  | [] => []
  | delta :: rest =>
      (prev + delta) :: undeltas (prev + delta) rest

theorem delta_roundtrip (maxSize prev : Nat) (offsets : List Nat)
    (valid : WellFormed maxSize prev offsets) :
    undeltas prev (deltas prev offsets) = offsets := by
  induction offsets generalizing prev with
  | nil => rfl
  | cons off rest ih =>
      have increasing : prev ≤ off := Nat.le_of_lt valid.1
      have hsum : prev + (off - prev) = off := by omega
      simp only [deltas, undeltas, hsum, List.cons.injEq, true_and]
      exact ih off valid.2.2

/-- The C++ decoder performs this offset and cardinality validation. -/
def readDeltas (maxSize maxCount prev count : Nat) : List Nat → Option (List Nat)
  | [] => some []
  | delta :: rest =>
      if delta > 0 ∧ delta < maxSize - prev ∧ count < maxCount then
        let off := prev + delta
        match readDeltas maxSize maxCount off (count + 1) rest with
        | some offsets => some (off :: offsets)
        | none => none
      else none

/-- The empty byte value is rejected by C++ even though a recursive tail is empty. -/
def deserializeValue (maxSize maxCount : Nat) : List Nat → Option (List Nat)
  | [] => none
  | d :: rest => readDeltas maxSize maxCount 0 0 (d :: rest)

def wellFormedCheck (maxSize prev : Nat) : List Nat → Bool
  | [] => true
  | off :: rest => decide (prev < off) && decide (off < maxSize) &&
      wellFormedCheck maxSize off rest

theorem wellFormed_check_iff (maxSize prev : Nat) (offsets : List Nat) :
    wellFormedCheck maxSize prev offsets = true ↔ WellFormed maxSize prev offsets := by
  induction offsets generalizing prev with
  | nil => simp [wellFormedCheck, WellFormed]
  | cons off rest ih =>
      simp [wellFormedCheck, WellFormed, ih, and_assoc]

def serializeValue (maxSize : Nat) (offsets : List Nat) : Option (List Nat) :=
  if offsets.isEmpty then none
  else if wellFormedCheck maxSize 0 offsets then some (deltas 0 offsets)
  else none

theorem positive_deltas (maxSize prev : Nat) (offsets : List Nat)
    (valid : WellFormed maxSize prev offsets) :
    ∀ delta ∈ deltas prev offsets, 0 < delta := by
  induction offsets generalizing prev with
  | nil => simp [deltas]
  | cons off rest ih =>
      simp only [deltas, List.mem_cons]
      intro delta hdelta
      rcases hdelta with head | tail
      · subst delta
        have hp := valid.1
        omega
      · exact ih off valid.2.2 delta tail

/-- A valid decoded value is strictly increasing and stays in the block. -/
theorem readDeltas_sound (maxSize maxCount prev count : Nat)
    (encoded offsets : List Nat)
    (h : readDeltas maxSize maxCount prev count encoded = some offsets) :
    WellFormed maxSize prev offsets := by
  induction encoded generalizing prev count offsets with
  | nil =>
      simp [readDeltas] at h
      cases h
      trivial
  | cons delta rest ih =>
      simp only [readDeltas] at h
      split at h
      · rename_i good
        split at h
        · rename_i decoded hrecursive
          cases h
          have hrest := ih (prev + delta) (count + 1) decoded hrecursive
          dsimp [WellFormed]
          constructor
          · exact Nat.lt_add_of_pos_right good.1
          constructor
          · omega
          · exact hrest
        · contradiction
      · contradiction

/-- Every nonempty, bounded value that C++ accepts represents increasing offsets. -/
theorem deserialize_sound (maxSize maxCount : Nat) (ds offsets : List Nat)
    (h : deserializeValue maxSize maxCount ds = some offsets) :
    WellFormed maxSize 0 offsets := by
  cases ds with
  | nil => simp [deserializeValue] at h
  | cons d rest =>
      exact readDeltas_sound maxSize maxCount 0 0 (d :: rest) offsets h

/-- A duplicate/zero delta fails. -/
theorem reject_zero_delta :
    deserializeValue 4000000 66666 [81, 0] = none := by decide

/-- A delta crossing the allowed block boundary fails. -/
theorem reject_out_of_block :
    deserializeValue 4000000 66666 [81, 3999919] = none := by decide

/-- A one-entry value that ends inside the block decodes. -/
theorem single_offset :
    deserializeValue 4000000 66666 [81] = some [81] := by decide

/-- A valid three-transaction value reproduces original offsets. -/
theorem three_offsets :
    deserializeValue 4000000 66666 [81, 125, 290] =
      some [81, 206, 496] := by decide

/-- C++ cardinality guard is checked before each new offset. -/
theorem max_count_rejected :
    deserializeValue 4000000 2 [81, 1, 1] = none := by decide

end BitcoinCore36629.OffsetCodec
