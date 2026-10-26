import UniversalKeyOrder
import Lean.Elab.Tactic.Omega

set_option autoImplicit false
set_option maxHeartbeats 1200000

namespace BitcoinCore36629.Encoding

/-- Five-byte, big-endian encoding of the truncated salted hash prefix.
    The value is a 40-bit natural number in the C++ DBKey. -/
def hashPrefixBytes (p : Nat) : List Nat :=
  [p / 4294967296, p / 16777216 % 256, p / 65536 % 256,
   p / 256 % 256, p % 256]

/-- DBKey::Serialize writes 's', five hash-prefix bytes, then OrderedVarInt height.
    This byte layout is manually extracted from txoscriptindex_key.h. -/
def rowKey (p h : Nat) : List Nat :=
  ([115] ++ hashPrefixBytes p) ++ encode h

theorem common_leading_bytes (p a b : List Nat) :
    byteLex (p ++ a) (p ++ b) = byteLex a b := by
  induction p with
  | nil => rfl
  | cons x xs ih => simp [byteLex, ih]

/-- An OrderedVarInt height range seek sorts correctly with an unchanged
    script-hash prefix. Universal, including all supported width boundaries. -/
theorem same_prefix_height_order (p x y : Nat) (hxy : x < y)
    (hy : y ≤ max5) :
    byteLex (rowKey p x) (rowKey p y) = true := by
  unfold rowKey
  rw [common_leading_bytes]
  exact strict_order x y hxy hy

theorem prefix_order (p q : Nat) (hpq : p < q) :
    byteLex (hashPrefixBytes p) (hashPrefixBytes q) = true := by
  exact fixed_five_order p q hpq

/-- With equal-length prefixes, changing the trailing height cannot reverse
    the ordering already decided by the first differing prefix byte. -/
theorem append_preserves_strict_lex (p q : List Nat) (a b : List Nat)
    (hlen : p.length = q.length) (hlt : byteLex p q = true) :
    byteLex (p ++ a) (q ++ b) = true := by
  induction p generalizing q with
  | nil =>
      cases q with
      | nil => simp [byteLex] at hlt
      | cons q qs => simp at hlen
  | cons x xs ih =>
      cases q with
      | nil => simp at hlen
      | cons y ys =>
          by_cases smaller : x < y
          · simp [byteLex, smaller]
          · by_cases greater : y < x
            · simp [byteLex, smaller, greater] at hlt
            · have same : x = y := by omega
              subst y
              have shorter : xs.length = ys.length := by
                simpa using hlen
              have tail : byteLex xs ys = true := by
                simpa [byteLex] using hlt
              have done := ih ys shorter tail
              simpa [byteLex] using done

/-- Height bytes are irrelevant when hash-prefix bytes already differ. -/
theorem different_prefix_order (p q x y : Nat) (hpq : p < q) :
    byteLex (rowKey p x) (rowKey q y) = true := by
  have hp : byteLex (hashPrefixBytes p) (hashPrefixBytes q) = true :=
    prefix_order p q hpq
  have hh : (hashPrefixBytes p).length = (hashPrefixBytes q).length := by
    simp [hashPrefixBytes]
  have hs := append_preserves_strict_lex (hashPrefixBytes p)
    (hashPrefixBytes q) (encode x) (encode y) hh hp
  simp only [rowKey, List.append_assoc, common_leading_bytes]
  exact hs

/-- DB iterator seeks cannot skip an earlier height within a fixed prefix,
    assuming LevelDB uses unsigned-byte lexicographic key comparison. -/
theorem seek_boundary_order (p start older : Nat) (h : older < start)
    (hs : start ≤ max5) :
    byteLex (rowKey p older) (rowKey p start) = true :=
  same_prefix_height_order p older start h hs

end BitcoinCore36629.Encoding
