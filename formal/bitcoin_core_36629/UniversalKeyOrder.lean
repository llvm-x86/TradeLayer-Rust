import FixedDigits
import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option maxHeartbeats 1600000

namespace BitcoinCore36629.Encoding

theorem numeric_classification (n : Nat) :
    n < 128 ∨ (128 ≤ n ∧ n < 16512) ∨
    (16512 ≤ n ∧ n < 2113664) ∨
    (2113664 ≤ n ∧ n < 270549120) ∨
    270549120 ≤ n := by omega

theorem encode_one (x : Nat) (h : x < 128) :
    encode x = [x] := by simp [encode, h]

theorem encode_two (x : Nat) (h : 128 ≤ x ∧ x < 16512) :
    encode x = [128 + (x-128)/256, (x-128)%256] := by
  simp [encode, show ¬x < 128 by omega, h.2]

theorem encode_three (x : Nat) (h : 16512 ≤ x ∧ x < 2113664) :
    encode x =
    [192 + (x-16512)/65536, (x-16512)/256%256, (x-16512)%256] := by
  simp [encode, show ¬x < 128 by omega,
    show ¬x < 16512 by omega, h.2]

theorem encode_four (x : Nat) (h : 2113664 ≤ x ∧ x < 270549120) :
    encode x =
    [224 + (x-2113664)/16777216, (x-2113664)/65536%256,
      (x-2113664)/256%256, (x-2113664)%256] := by
  simp [encode, show ¬x < 128 by omega,
    show ¬x < 16512 by omega, show ¬x < 2113664 by omega, h.2]

theorem encode_five (x : Nat) (h : 270549120 ≤ x) :
    encode x =
    [240 + (x-270549120)/4294967296, (x-270549120)/16777216%256,
      (x-270549120)/65536%256, (x-270549120)/256%256,
      (x-270549120)%256] := by
  simp [encode, show ¬x < 128 by omega,
    show ¬x < 16512 by omega, show ¬x < 2113664 by omega,
    show ¬x < 270549120 by omega]

/-- Strict bytewise ordering for every supported OrderedVarInt value pair. -/
theorem strict_order (x y : Nat) (hxy : x < y) (_hmax : y ≤ max5) :
    byteLex (encode x) (encode y) = true := by
  rcases numeric_classification x with hx1 | hx2 | hx3 | hx4 | hx5
  · rcases numeric_classification y with hy1 | hy2 | hy3 | hy4 | hy5
    · rw [encode_one x hx1, encode_one y hy1]
      simp [byteLex, hxy]
    · rw [encode_one x hx1, encode_two y hy2]
      apply head_smaller
      omega
    · rw [encode_one x hx1, encode_three y hy3]
      apply head_smaller
      omega
    · rw [encode_one x hx1, encode_four y hy4]
      apply head_smaller
      omega
    · rw [encode_one x hx1, encode_five y hy5]
      apply head_smaller
      omega
  · rcases numeric_classification y with hy1 | hy2 | hy3 | hy4 | hy5
    · omega
    · exact within_two x y hx2 hy2 hxy
    · rw [encode_two x hx2, encode_three y hy3]
      apply head_smaller
      omega
    · rw [encode_two x hx2, encode_four y hy4]
      apply head_smaller
      omega
    · rw [encode_two x hx2, encode_five y hy5]
      apply head_smaller
      omega
  · rcases numeric_classification y with hy1 | hy2 | hy3 | hy4 | hy5
    · omega
    · omega
    · exact within_three x y hx3 hy3 hxy
    · rw [encode_three x hx3, encode_four y hy4]
      apply head_smaller
      omega
    · rw [encode_three x hx3, encode_five y hy5]
      apply head_smaller
      omega
  · rcases numeric_classification y with hy1 | hy2 | hy3 | hy4 | hy5
    · omega
    · omega
    · omega
    · exact within_four x y hx4 hy4 hxy
    · rw [encode_four x hx4, encode_five y hy5]
      apply head_smaller
      omega
  · rcases numeric_classification y with hy1 | hy2 | hy3 | hy4 | hy5
    · omega
    · omega
    · omega
    · omega
    · exact within_five x y hx5 hy5 hxy

end BitcoinCore36629.Encoding
