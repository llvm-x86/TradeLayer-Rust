import MergeHistory
import Std

set_option autoImplicit false

namespace BitcoinCore36629.MergeHistory

/-- Transaction identity and vout count from one coherent index/chain view. -/
abbrev Snapshot := Position → Option (Nat × Nat)

def ValidHit (view : Snapshot) (hit : Hit) : Prop :=
  view hit.pos = some (hit.txid, hit.voutCount) ∧ outputIndexesInBounds hit

def StableGroups (view : Snapshot) (groups : GroupMap) : Prop :=
  ∀ pos hit, groups pos = some hit →
    hit.pos = pos ∧ ValidHit view hit

theorem empty_stable (view : Snapshot) :
    StableGroups view emptyMap := by
  intro pos hit h
  simp [emptyMap] at h

/-- An incoming script-query result can be merged safely if it belongs to
    the same immutable height/offset to transaction mapping as the map. -/
theorem insert_stable (view : Snapshot) (groups : GroupMap)
    (hit : Hit) (hOld : StableGroups view groups)
    (hHit : ValidHit view hit) :
    StableGroups view (insert groups hit) := by
  intro pos merged hMerged
  by_cases hp : pos = hit.pos
  · subst pos
    cases he : groups hit.pos with
    | none =>
        have hm : merged = hit := by
          simpa [insert, he] using hMerged.symm
        subst merged
        exact ⟨rfl, hHit⟩
    | some previous =>
        have hPrevious := hOld hit.pos previous he
        have hpPrev : previous.pos = hit.pos := hPrevious.1
        have hPrevSnap := hPrevious.2.1
        have countEq : previous.voutCount = hit.voutCount := by
          have hpair : (previous.txid, previous.voutCount) =
              (hit.txid, hit.voutCount) := by
            have hEq := hPrevSnap.symm.trans hHit.1
            exact Option.some.inj hEq
          exact congrArg Prod.snd hpair
        have hNewBound := safe_same_transaction_union previous hit
          countEq hPrevious.2.2 hHit.2
        have hm : merged =
            {previous with outputIndexes :=
              previous.outputIndexes ++ hit.outputIndexes} := by
          simpa [insert, he] using hMerged.symm
        subst merged
        refine ⟨hpPrev, ?_, hNewBound⟩
        exact hPrevSnap
  · have hEq := other_position_unchanged groups hit pos hp
    rw [hEq] at hMerged
    exact hOld pos merged hMerged

def insertMany (initial : GroupMap) : List Hit → GroupMap
  | [] => initial
  | hit :: remaining => insertMany (insert initial hit) remaining

/-- Every finite sequence of script queries preserves safe vout indexing
    provided all rows refer to one stable transaction snapshot. -/
theorem insert_many_stable (view : Snapshot) (initial : GroupMap)
    (hits : List Hit) (hInitial : StableGroups view initial)
    (hHits : ∀ hit ∈ hits, ValidHit view hit) :
    StableGroups view (insertMany initial hits) := by
  induction hits generalizing initial with
  | nil => exact hInitial
  | cons first rest ih =>
      apply ih (insert initial first)
      · exact insert_stable view initial first hInitial
          (hHits first (by simp))
      · intro hit hMem
        exact hHits hit (by simp [hMem])

def aggregate (hits : List Hit) : GroupMap :=
  insertMany emptyMap hits

theorem aggregate_sound (view : Snapshot) (hits : List Hit)
    (h : ∀ hit ∈ hits, ValidHit view hit) :
    StableGroups view (aggregate hits) := by
  exact insert_many_stable view emptyMap hits (empty_stable view) h

/-- Conflicting transaction identities can occupy the same height/offset
    when modeled reads belong to incompatible snapshots. -/
def oldView : Snapshot := fun p =>
  if p = oldHit.pos then some (oldHit.txid, oldHit.voutCount) else none

def differentView : Snapshot := fun p =>
  if p = newHit.pos then some (newHit.txid, newHit.voutCount) else none

theorem opposing_snapshots_same_key :
    oldHit.pos = newHit.pos ∧
    oldView oldHit.pos ≠ differentView newHit.pos := by decide

end BitcoinCore36629.MergeHistory
