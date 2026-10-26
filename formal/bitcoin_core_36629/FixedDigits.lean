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

end BitcoinCore36629.Encoding
