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
  unfold encode
  split_ifs <;> simp [byteLex] <;> split_ifs <;> omega

end BitcoinCore36629.Encoding
