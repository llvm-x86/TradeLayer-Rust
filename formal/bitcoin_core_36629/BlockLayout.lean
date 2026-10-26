import OffsetCodec
import Lean.Elab.Tactic.Omega

/-!
Index write-path model: src/index/txoscriptindex.cpp::DB::WriteEntries.
The C++ writer starts at block-header length (80) plus CompactSize(vtx
count), and adds ComputeTotalSize() for each serialized transaction.

This is the arithmetic layout of all transaction offsets, under an assumed
accurate serialized transaction size and block-size ceiling. C++ integer
overflow, transaction deserialization, and data-file consistency are not
formally translated into Lean here.
-/
set_option autoImplicit false

namespace BitcoinCore36629.BlockLayout
open BitcoinCore36629.OffsetCodec

def initialOffset (compactSizeLength : Nat) : Nat := 80 + compactSizeLength

/-- One position per transaction; subsequent positions advance by full
    serialized transaction byte size, including witness. -/
def positions (start : Nat) : List Nat → List Nat
  | [] => []
  | size :: rest => start :: positions (start + size) rest

theorem initial_offset_positive (compactSizeLength : Nat) :
    0 < initialOffset compactSizeLength := by
  simp [initialOffset]
  omega

theorem positions_length (start : Nat) (sizes : List Nat) :
    (positions start sizes).length = sizes.length := by
  induction sizes generalizing start with
  | nil => rfl
  | cons size rest ih =>
      simp [positions, ih]

theorem positions_strict (cap prev start : Nat) (sizes : List Nat)
    (hprev : prev < start)
    (hpos : ∀ s ∈ sizes, 0 < s)
    (hcap : start + sizes.sum < cap) :
    WellFormed cap prev (positions start sizes) := by
  induction sizes generalizing prev start with
  | nil => trivial
  | cons size rest ih =>
      have hsize : 0 < size := hpos size (by simp)
      have htail : ∀ s ∈ rest, 0 < s := by
        intro s hs
        exact hpos s (by simp [hs])
      have hcapRest : (start + size) + rest.sum < cap := by
        simp only [List.sum_cons] at hcap
        omega
      dsimp [positions, WellFormed]
      refine ⟨hprev, ?_, ?_⟩
      · omega
      · exact ih start (start + size) (by omega) htail hcapRest

/-- The exact writer offsets roundtrip through the DBValue delta representation. -/
theorem offsets_delta_roundtrip (cap compactSizeLength : Nat)
    (sizes : List Nat)
    (hpositive : ∀ s ∈ sizes, 0 < s)
    (hbounded : initialOffset compactSizeLength + sizes.sum < cap) :
    undeltas 0 (deltas 0 (positions (initialOffset compactSizeLength) sizes)) =
      positions (initialOffset compactSizeLength) sizes := by
  exact delta_roundtrip cap 0 _
    (positions_strict cap 0 (initialOffset compactSizeLength) sizes
      (initial_offset_positive compactSizeLength) hpositive hbounded)

/-- A synthetic mixed SegWit block has offsets 81, 181, 431 and 731. -/
theorem mixed_size_layout :
    positions 81 [100, 250, 300, 220] = [81, 181, 431, 731] := by decide

/-- The same four positions are stored as positive deltas. -/
theorem mixed_size_deltas :
    deltas 0 (positions 81 [100, 250, 300, 220]) = [81, 100, 250, 300] := by decide

end BitcoinCore36629.BlockLayout
