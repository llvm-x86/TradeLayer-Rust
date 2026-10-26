import MergeSnapshot
import Std

/-!
Fork-aware alternative to the source's map keyed only by
(height, transaction byte offset).

Pinned upstream bitcoin/bitcoin#36629:
cf9b981fcc7ae52c7864beae35df5d60b5c322b4
src/rpc/blockchain.cpp, getscripthistory step 5.

The source already supplies block_hash for every confirmed hit. Adding
block_hash to the aggregation identity separates same-position hits from
different forks. This model proves a *conditional* safety property:
one immutable block hash and byte offset determine one transaction body
and output count. This is a recommendation, not an upstream code change.
-/

set_option autoImplicit false

namespace BitcoinCore36629.ForkAwareMerge
open BitcoinCore36629.MergeHistory

structure ForkKey where
  blockHash : Nat
  position : Position
  deriving DecidableEq, Repr

structure ForkHit where
  blockHash : Nat
  tx : Hit
  deriving DecidableEq, Repr

def key (h : ForkHit) : ForkKey := ⟨h.blockHash, h.tx.pos⟩

/-- Each immutable block identity determines the transaction at an offset. -/
abbrev ForkImage := ForkKey → Option (Nat × Nat)

def ValidForkHit (image : ForkImage) (h : ForkHit) : Prop :=
  image (key h) = some (h.tx.txid, h.tx.voutCount) ∧ outputIndexesInBounds h.tx

abbrev ForkGroups := ForkKey → Option ForkHit

def empty : ForkGroups := fun _ => none

def insert (groups : ForkGroups) (hit : ForkHit) : ForkGroups :=
  fun k =>
    if k = key hit then
      match groups k with
      | none => some hit
      | some prev => some {prev with tx := {
          prev.tx with
          outputIndexes := prev.tx.outputIndexes ++ hit.tx.outputIndexes}}
    else groups k

def Stable (image : ForkImage) (groups : ForkGroups) : Prop :=
  ∀ k hit, groups k = some hit → key hit = k ∧ ValidForkHit image hit

theorem empty_stable (image : ForkImage) : Stable image empty := by
  intro k h hh
  simp [empty] at hh

theorem other_key_unchanged (groups : ForkGroups) (hit : ForkHit)
    (k : ForkKey) (ne : k ≠ key hit) :
    insert groups hit k = groups k := by
  simp [insert, ne]

theorem insert_stable (image : ForkImage) (groups : ForkGroups)
    (hit : ForkHit) (old : Stable image groups)
    (good : ValidForkHit image hit) :
    Stable image (insert groups hit) := by
  intro k result hResult
  by_cases hk : k = key hit
  · subst k
    cases existing : groups (key hit) with
    | none =>
        have same : result = hit := by
          simpa [insert, existing] using hResult.symm
        subst result
        exact ⟨rfl, good⟩
    | some previous =>
        have previousGood := old (key hit) previous existing
        have keyPrevious : key previous = key hit := previousGood.1
        have imagePrevious := previousGood.2.1
        have imageCurrent := good.1
        have pairEqual : (previous.tx.txid, previous.tx.voutCount) =
            (hit.tx.txid, hit.tx.voutCount) := by
          have sameImage : image (key hit) =
              some (previous.tx.txid, previous.tx.voutCount) := by
            simpa [keyPrevious] using imagePrevious
          exact Option.some.inj (sameImage.symm.trans imageCurrent)
        have sameCount : previous.tx.voutCount = hit.tx.voutCount :=
          congrArg Prod.snd pairEqual
        have validBounds := safe_same_transaction_union
          previous.tx hit.tx sameCount previousGood.2.2 good.2
        have sameResult : result = {previous with tx := {previous.tx with
            outputIndexes := previous.tx.outputIndexes ++ hit.tx.outputIndexes}} := by
          simpa [insert, existing] using hResult.symm
        subst result
        constructor
        · exact keyPrevious
        · exact ⟨imagePrevious, validBounds⟩
  · rw [other_key_unchanged groups hit k hk] at hResult
    exact old k result hResult

def insertMany (initial : ForkGroups) : List ForkHit → ForkGroups
  | [] => initial
  | x :: xs => insertMany (insert initial x) xs

theorem insert_many_stable (image : ForkImage) (initial : ForkGroups)
    (hs : List ForkHit) (start : Stable image initial)
    (valid : ∀ h ∈ hs, ValidForkHit image h) :
    Stable image (insertMany initial hs) := by
  induction hs generalizing initial with
  | nil => exact start
  | cons h rest ih =>
      apply ih (insert initial h)
      · exact insert_stable image initial h start (valid h (by simp))
      · intro other ho
        exact valid other (by simp [ho])

def aggregate (hs : List ForkHit) : ForkGroups := insertMany empty hs

theorem aggregate_sound (image : ForkImage) (hs : List ForkHit)
    (valid : ∀ h ∈ hs, ValidForkHit image h) :
    Stable image (aggregate hs) :=
  insert_many_stable image empty hs (empty_stable image) valid

/-- Old and new blocks have identical heights and transaction byte offsets,
    but distinct block hashes. Both produce locally valid hits. -/
def oldFork : ForkHit := ⟨101, oldHit⟩
def newFork : ForkHit := ⟨202, newHit⟩

def exampleImage : ForkImage := fun k =>
  if k = key oldFork then some (oldHit.txid, oldHit.voutCount)
  else if k = key newFork then some (newHit.txid, newHit.voutCount)
  else none

theorem same_height_offset_different_block_hash :
    oldFork.tx.pos = newFork.tx.pos ∧ key oldFork ≠ key newFork := by decide

theorem source_height_only_merge_has_invalid_vout :
    (BitcoinCore36629.MergeHistory.aggregate [oldHit, newHit] oldHit.pos).map
        bounded = some false := by decide

theorem fork_aware_merging_preserves_both_transactions :
    (aggregate [oldFork, newFork] (key oldFork)).map
        (fun h : ForkHit => (h.tx.txid, bounded h.tx)) = some (oldHit.txid, true) ∧
    (aggregate [oldFork, newFork] (key newFork)).map
        (fun h : ForkHit => (h.tx.txid, bounded h.tx)) = some (newHit.txid, true) := by
  decide

/-- On the synthetic fork pair, each lookup satisfies the immutable-body map. -/
theorem example_image_is_safe :
    ValidForkHit exampleImage oldFork ∧ ValidForkHit exampleImage newFork := by
  decide

end BitcoinCore36629.ForkAwareMerge
