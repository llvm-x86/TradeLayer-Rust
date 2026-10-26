import Std
import Lean.Elab.Tactic.Omega
import OrderedEncoding

set_option autoImplicit false
set_option maxHeartbeats 1600000

namespace BitcoinCore36629.Encoding

/-!
A stronger claim than the cutover-vector checks: the complete 1–5 byte
representation is strictly lexicographically monotone over its domain.
-/

theorem strict_byte_order (x y : Nat)
    (hx : x ≤ max5) (hy : y ≤ max5) (hxy : x < y) :
    byteLex (encode x) (encode y) = true := by
  by_cases hx1 : x < 128 <;>
  by_cases hx2 : x < 16512 <;>
  by_cases hx3 : x < 2113664 <;>
  by_cases hx4 : x < 270549120 <;>
  by_cases hy1 : y < 128 <;>
  by_cases hy2 : y < 16512 <;>
  by_cases hy3 : y < 2113664 <;>
  by_cases hy4 : y < 270549120 <;>
  simp_all [encode, byteLex] <;> omega

end BitcoinCore36629.Encoding
