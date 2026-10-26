import OrderedEncoding
import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option maxHeartbeats 1200000

namespace BitcoinCore36629.Encoding

/-- Adding the same prefix to the leading byte preserves list lex order. -/
theorem head_shift (base a b : Nat) (left right : List Nat) :
    byteLex ((base + a) :: left) ((base + b) :: right) =
      byteLex (a :: left) (b :: right) := by
  simp [byteLex]

/-- A strictly smaller first byte wins regardless of subsequent bytes. -/
theorem head_smaller (a b : Nat) (left right : List Nat) (h : a < b) :
    byteLex (a :: left) (b :: right) = true := by
  simp [byteLex, h]

theorem fixed_two_order (a b : Nat) (h : a < b) :
    byteLex [a / 256, a % 256] [b / 256, b % 256] = true := by
  by_cases high : a / 256 < b / 256
  · simp [byteLex, high]
  · have equal : a / 256 = b / 256 := by omega
    have low : a % 256 < b % 256 := by omega
    simp [byteLex, equal, low]

theorem fixed_three_order (a b : Nat) (h : a < b) :
    byteLex [a / 65536, a / 256 % 256, a % 256]
      [b / 65536, b / 256 % 256, b % 256] = true := by
  by_cases high : a / 65536 < b / 65536
  · simp [byteLex, high]
  · have equal : a / 65536 = b / 65536 := by omega
    have lower : a % 65536 < b % 65536 := by omega
    have ha : (a % 65536) / 256 = a / 256 % 256 := by omega
    have hb : (b % 65536) / 256 = b / 256 % 256 := by omega
    have hal : (a % 65536) % 256 = a % 256 := by omega
    have hbl : (b % 65536) % 256 = b % 256 := by omega
    have tail := fixed_two_order (a % 65536) (b % 65536) lower
    have tail' : byteLex [a / 256 % 256, a % 256]
        [b / 256 % 256, b % 256] = true := by
      simpa [ha, hb, hal, hbl] using tail
    simpa [byteLex, equal] using tail'

theorem fixed_four_order (a b : Nat) (h : a < b) :
    byteLex [a / 16777216, a / 65536 % 256, a / 256 % 256, a % 256]
      [b / 16777216, b / 65536 % 256, b / 256 % 256, b % 256] = true := by
  by_cases high : a / 16777216 < b / 16777216
  · simp [byteLex, high]
  · have equal : a / 16777216 = b / 16777216 := by omega
    have lower : a % 16777216 < b % 16777216 := by omega
    have ha : (a % 16777216) / 65536 = a / 65536 % 256 := by omega
    have hb : (b % 16777216) / 65536 = b / 65536 % 256 := by omega
    have ham : (a % 16777216) / 256 % 256 = a / 256 % 256 := by omega
    have hbm : (b % 16777216) / 256 % 256 = b / 256 % 256 := by omega
    have hal : (a % 16777216) % 256 = a % 256 := by omega
    have hbl : (b % 16777216) % 256 = b % 256 := by omega
    have tail := fixed_three_order (a % 16777216) (b % 16777216) lower
    have tail' : byteLex [a / 65536 % 256, a / 256 % 256, a % 256]
        [b / 65536 % 256, b / 256 % 256, b % 256] = true := by
      simpa [ha, hb, ham, hbm, hal, hbl] using tail
    simpa [byteLex, equal] using tail'

theorem fixed_five_order (a b : Nat) (h : a < b) :
    byteLex [a / 4294967296, a / 16777216 % 256, a / 65536 % 256,
      a / 256 % 256, a % 256]
      [b / 4294967296, b / 16777216 % 256, b / 65536 % 256,
        b / 256 % 256, b % 256] = true := by
  by_cases high : a / 4294967296 < b / 4294967296
  · simp [byteLex, high]
  · have equal : a / 4294967296 = b / 4294967296 := by omega
    have lower : a % 4294967296 < b % 4294967296 := by omega
    have ha : (a % 4294967296) / 16777216 = a / 16777216 % 256 := by omega
    have hb : (b % 4294967296) / 16777216 = b / 16777216 % 256 := by omega
    have ham : (a % 4294967296) / 65536 % 256 = a / 65536 % 256 := by omega
    have hbm : (b % 4294967296) / 65536 % 256 = b / 65536 % 256 := by omega
    have han : (a % 4294967296) / 256 % 256 = a / 256 % 256 := by omega
    have hbn : (b % 4294967296) / 256 % 256 = b / 256 % 256 := by omega
    have hal : (a % 4294967296) % 256 = a % 256 := by omega
    have hbl : (b % 4294967296) % 256 = b % 256 := by omega
    have tail := fixed_four_order (a % 4294967296) (b % 4294967296) lower
    have tail' : byteLex [a / 16777216 % 256, a / 65536 % 256,
        a / 256 % 256, a % 256]
        [b / 16777216 % 256, b / 65536 % 256, b / 256 % 256, b % 256] =
        true := by
      simpa [ha, hb, ham, hbm, han, hbn, hal, hbl] using tail
    simpa [byteLex, equal] using tail'


/-- Within the two-byte class the actual OrderedVarInt encoding is monotone. -/
theorem within_two (x y : Nat) (hx : 128 ≤ x ∧ x < 16512)
    (hy : 128 ≤ y ∧ y < 16512) (h : x < y) :
    byteLex (encode x) (encode y) = true := by
  have ex : encode x = [128 + (x-128)/256, (x-128)%256] := by
    simp [encode, show ¬x < 128 by omega, hx.2]
  have ey : encode y = [128 + (y-128)/256, (y-128)%256] := by
    simp [encode, show ¬y < 128 by omega, hy.2]
  rw [ex,ey,head_shift]
  exact fixed_two_order (x-128) (y-128) (by omega)

/-- Within the three-byte class the actual encoding is monotone. -/
theorem within_three (x y : Nat) (hx : 16512 ≤ x ∧ x < 2113664)
    (hy : 16512 ≤ y ∧ y < 2113664) (h : x < y) :
    byteLex (encode x) (encode y) = true := by
  have ex : encode x =
      [192 + (x-16512)/65536, (x-16512)/256%256, (x-16512)%256] := by
    simp [encode, show ¬x < 128 by omega,
      show ¬x < 16512 by omega, hx.2]
  have ey : encode y =
      [192 + (y-16512)/65536, (y-16512)/256%256, (y-16512)%256] := by
    simp [encode, show ¬y < 128 by omega,
      show ¬y < 16512 by omega, hy.2]
  rw [ex,ey,head_shift]
  exact fixed_three_order (x-16512) (y-16512) (by omega)

theorem within_four (x y : Nat) (hx : 2113664 ≤ x ∧ x < 270549120)
    (hy : 2113664 ≤ y ∧ y < 270549120) (h : x < y) :
    byteLex (encode x) (encode y) = true := by
  have ex : encode x =
      [224 + (x-2113664)/16777216, (x-2113664)/65536%256,
        (x-2113664)/256%256, (x-2113664)%256] := by
    simp [encode, show ¬x < 128 by omega,
      show ¬x < 16512 by omega, show ¬x < 2113664 by omega, hx.2]
  have ey : encode y =
      [224 + (y-2113664)/16777216, (y-2113664)/65536%256,
        (y-2113664)/256%256, (y-2113664)%256] := by
    simp [encode, show ¬y < 128 by omega,
      show ¬y < 16512 by omega, show ¬y < 2113664 by omega, hy.2]
  rw [ex,ey,head_shift]
  exact fixed_four_order (x-2113664) (y-2113664) (by omega)

theorem within_five (x y : Nat) (hx : 270549120 ≤ x)
    (hy : 270549120 ≤ y) (h : x < y) :
    byteLex (encode x) (encode y) = true := by
  have ex : encode x =
      [240 + (x-270549120)/4294967296, (x-270549120)/16777216%256,
        (x-270549120)/65536%256, (x-270549120)/256%256,
        (x-270549120)%256] := by
    simp [encode, show ¬x < 128 by omega,
      show ¬x < 16512 by omega,
      show ¬x < 2113664 by omega, show ¬x < 270549120 by omega]
  have ey : encode y =
      [240 + (y-270549120)/4294967296, (y-270549120)/16777216%256,
        (y-270549120)/65536%256, (y-270549120)/256%256,
        (y-270549120)%256] := by
    simp [encode, show ¬y < 128 by omega,
      show ¬y < 16512 by omega,
      show ¬y < 2113664 by omega, show ¬y < 270549120 by omega]
  rw [ex,ey,head_shift]
  exact fixed_five_order (x-270549120) (y-270549120) (by omega)

end BitcoinCore36629.Encoding
