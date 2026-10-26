import Std
import Lean.Elab.Tactic.Omega

/-!
Exact five-byte OrderedVarInt byte layout from bitcoin/bitcoin#36629,
src/index/ordered_varint.h at cf9b981fcc7ae52c7864beae35df5d60b5c322b4.

Each list entry is one byte as a Nat in [0,255]. This reference encoder
uses the same prefix and seven-bit range-subtraction cutovers as C++.
-/

set_option autoImplicit false

namespace BitcoinCore36629.Encoding

def max5 : Nat := 34630287487

/-- C++ OrderedVarIntFormatter::Ser, expanded for five width classes. -/
def encode (n : Nat) : List Nat :=
  if n < 128 then [n]
  else if n < 16512 then
    let r := n - 128
    [128 + r / 256, r % 256]
  else if n < 2113664 then
    let r := n - 16512
    [192 + r / 65536, (r / 256) % 256, r % 256]
  else if n < 270549120 then
    let r := n - 2113664
    [224 + r / 16777216, (r / 65536) % 256, (r / 256) % 256, r % 256]
  else
    let r := n - 270549120
    [240 + r / 4294967296, (r / 16777216) % 256,
      (r / 65536) % 256, (r / 256) % 256, r % 256]

/-- C++ OrderedVarIntFormatter::Unser, restricted to exact-length encodings. -/
def decode : List Nat → Option Nat
  | [a] =>
    if a < 128 then some a else none
  | [a, b] =>
    if 128 ≤ a ∧ a < 192 ∧ b < 256 then
      some (128 + (a - 128) * 256 + b)
    else none
  | [a, b, c] =>
    if 192 ≤ a ∧ a < 224 ∧ b < 256 ∧ c < 256 then
      some (16512 + (a - 192) * 65536 + b * 256 + c)
    else none
  | [a, b, c, d] =>
    if 224 ≤ a ∧ a < 240 ∧ b < 256 ∧ c < 256 ∧ d < 256 then
      some (2113664 + (a - 224) * 16777216 + b * 65536 + c * 256 + d)
    else none
  | [a, b, c, d, e] =>
    if 240 ≤ a ∧ a < 248 ∧ b < 256 ∧ c < 256 ∧ d < 256 ∧ e < 256 then
      some (270549120 + (a - 240) * 4294967296 +
        b * 16777216 + c * 65536 + d * 256 + e)
    else none
  | _ => none

/-- Lexicographic byte comparison, exactly as needed by LevelDB range seeks. -/
def byteLex : List Nat → List Nat → Bool
  | [], [] => false
  | [], _ :: _ => true
  | _ :: _, [] => false
  | a :: as, b :: bs =>
    if a < b then true
    else if b < a then false
    else byteLex as bs

/-! Executable kernel-checked vectors matching ordered_varint_tests.cpp. -/
theorem zero_encoding : encode 0 = [0] := by decide
theorem one_byte_max : encode 127 = [127] := by decide
theorem two_byte_min : encode 128 = [128, 0] := by decide
theorem two_byte_max : encode 16511 = [191, 255] := by decide
theorem three_byte_min : encode 16512 = [192, 0, 0] := by decide
theorem three_byte_max : encode 2113663 = [223, 255, 255] := by decide
theorem four_byte_min : encode 2113664 = [224, 0, 0, 0] := by decide
theorem four_byte_max : encode 270549119 = [239, 255, 255, 255] := by decide
theorem five_byte_min : encode 270549120 = [240, 0, 0, 0, 0] := by decide
theorem uint32_max : encode 4294967295 = [240, 239, 223, 191, 127] := by decide
theorem five_byte_max : encode max5 = [247, 255, 255, 255, 255] := by decide
theorem four_billion : encode 4000000000 = [240, 222, 74, 231, 128] := by decide

theorem cutover_1_2 : byteLex (encode 127) (encode 128) = true := by decide
theorem cutover_2_3 : byteLex (encode 16511) (encode 16512) = true := by decide
theorem cutover_3_4 : byteLex (encode 2113663) (encode 2113664) = true := by decide
theorem cutover_4_5 : byteLex (encode 270549119) (encode 270549120) = true := by decide

theorem max_uint32_roundtrip : decode (encode 4294967295) = some 4294967295 := by decide
theorem max_supported_roundtrip : decode (encode max5) = some max5 := by decide
theorem rejects_six_byte_prefix : decode [248, 0, 0, 0, 0] = none := by decide
theorem rejects_truncation : decode [192, 0] = none := by decide

end BitcoinCore36629.Encoding
