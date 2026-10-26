import Std
import Lean.Elab.Tactic.Omega

/-!
A finite, functional model of the relevant parts of Bitcoin Core PR #36629.
Pinned source: andrewtoth/bitcoin@cf9b981fcc7ae52c7864beae35df5d60b5c322b4

This models the index's semantic operations, not the C++ runtime or LevelDB.
The hash-to-prefix function is arbitrary, so collisions are explicitly allowed.
-/

set_option autoImplicit false

namespace BitcoinCore36629

abbrev ScriptHash := Nat
abbrev Prefix := Nat
abbrev PrefixFn := ScriptHash → Prefix

structure Tx where
  offset : Nat
  outputHashes : List ScriptHash
  deriving Repr, DecidableEq

structure Block where
  height : Nat
  txs : List Tx
  deriving Repr, DecidableEq

structure Key where
  hashPrefix : Prefix
  height : Nat
  offset : Nat
  deriving Repr, DecidableEq

/-- An exact script-hash match in the output set of one transaction. -/
def exactHit (query : ScriptHash) (tx : Tx) : Prop :=
  query ∈ tx.outputHashes

/-- A candidate identified by the five-byte prefix. Allows hash collisions. -/
def prefixHit (f : PrefixFn) (query : ScriptHash) (tx : Tx) : Prop :=
  ∃ sh, sh ∈ tx.outputHashes ∧ f sh = f query

theorem exact_implies_prefix_hit (f : PrefixFn) (query : ScriptHash) (tx : Tx)
    (h : exactHit query tx) : prefixHit f query tx := by
  exact ⟨query, h, rfl⟩

theorem full_hash_filter_exact (f : PrefixFn) (query : ScriptHash) (tx : Tx) :
    (prefixHit f query tx ∧ exactHit query tx) ↔ exactHit query tx := by
  constructor
  · exact And.right
  · intro h
    exact ⟨exact_implies_prefix_hit f query tx h, h⟩

/-- The actual query performs a prefix lookup, followed by an exact filter. -/
noncomputable def prefixCandidates (f : PrefixFn) (query : ScriptHash) (b : Block) : List Tx := by
  classical
  exact b.txs.filter (fun tx => decide (prefixHit f query tx))

noncomputable def confirmedMatches (f : PrefixFn) (query : ScriptHash) (b : Block) : List Tx := by
  classical
  exact (prefixCandidates f query b).filter (fun tx => decide (exactHit query tx))

/-- Retrieved, non-pruned transactions are exact and complete despite prefix collisions. -/
theorem confirmed_matches_iff (f : PrefixFn) (query : ScriptHash)
    (b : Block) (tx : Tx) :
    tx ∈ confirmedMatches f query b ↔ tx ∈ b.txs ∧ exactHit query tx := by
  classical
  simp only [confirmedMatches, prefixCandidates, List.mem_filter]
  constructor
  · rintro ⟨⟨hmem, _⟩, hexact⟩
    have h : exactHit query tx := by simpa using hexact
    exact ⟨hmem, h⟩
  · rintro ⟨hmem, hexact⟩
    have hp : decide (prefixHit f query tx) = true := by
      simpa using exact_implies_prefix_hit f query tx hexact
    have he : decide (exactHit query tx) = true := by simpa using hexact
    exact ⟨⟨hmem, hp⟩, he⟩

/-- A DB row is keyed by prefix, height, and transaction offset.
    The actual LevelDB value groups all offsets with the same prefix and height. -/
def hasRow (f : PrefixFn) (b : Block) (key : Key) : Prop :=
  key.height = b.height ∧
  ∃ tx, tx ∈ b.txs ∧ tx.offset = key.offset ∧
  ∃ sh, sh ∈ tx.outputHashes ∧ f sh = key.hashPrefix

/-- Every matching output generates the correct row before offset deduplication. -/
theorem matching_output_is_indexed (f : PrefixFn) (b : Block) (tx : Tx)
    (query : ScriptHash) (htx : tx ∈ b.txs)
    (hquery : exactHit query tx) :
    hasRow f b ⟨f query, b.height, tx.offset⟩ := by
  exact ⟨rfl, tx, htx, rfl, query, hquery, rfl⟩

/-- Every indexed offset really belongs to a transaction with the matching prefix. -/
theorem indexed_row_sound (f : PrefixFn) (b : Block) (key : Key)
    (h : hasRow f b key) :
    key.height = b.height ∧
    ∃ tx, tx ∈ b.txs ∧ tx.offset = key.offset ∧
      ∃ sh, sh ∈ tx.outputHashes ∧ f sh = key.hashPrefix := by
  exact h

def inRange (startHeight endHeight height : Nat) : Prop :=
  startHeight ≤ height ∧ height ≤ endHeight

/-- A complete index and available block data yield every in-range transaction. -/
theorem in_range_match_is_retrieved (f : PrefixFn) (query : ScriptHash)
    (b : Block) (tx : Tx) (startHeight endHeight : Nat)
    (hRange : inRange startHeight endHeight b.height)
    (hTx : tx ∈ b.txs) (hHit : exactHit query tx) :
    inRange startHeight endHeight b.height ∧
    tx ∈ confirmedMatches f query b := by
  exact ⟨hRange, (confirmed_matches_iff f query b tx).2 ⟨hTx, hHit⟩⟩

/-- An index database is represented by membership, avoiding duplicate keys. -/
abbrev Database := Key → Prop

def appendRows (db rows : Database) : Database :=
  fun key => db key ∨ rows key

def eraseRows (db rows : Database) : Database :=
  fun key => db key ∧ ¬ rows key

/-- Append followed by rollback restores the previous index when keys are fresh. -/
theorem append_erase_roundtrip (db rows : Database)
    (fresh : ∀ key, db key → ¬ rows key) :
    ∀ key, eraseRows (appendRows db rows) rows key ↔ db key := by
  intro key
  constructor
  · intro h
    rcases h.1 with old | added
    · exact old
    · exact False.elim (h.2 added)
  · intro h
    exact ⟨Or.inl h, fresh key h⟩

/-- Fresh block heights suffice for the append/remove roundtrip. -/
theorem rollback_new_height (db : Database) (f : PrefixFn) (b : Block)
    (freshHeight : ∀ key, db key → key.height ≠ b.height) :
    ∀ key, eraseRows (appendRows db (hasRow f b)) (hasRow f b) key ↔ db key := by
  apply append_erase_roundtrip
  intro key hOld hRow
  exact freshHeight key hOld hRow.1

/-- The same prefix may refer to a different script hash. -/
def collisionPrefix (_ : ScriptHash) : Prefix := 0

def foreignTx : Tx := ⟨81, [20]⟩

/-- A pruned-block candidate cannot be promoted to a certified exact match. -/
theorem pruned_prefix_false_positive :
    prefixHit collisionPrefix 10 foreignTx ∧ ¬ exactHit 10 foreignTx := by
  constructor
  · exact ⟨20, by simp [foreignTx], rfl⟩
  · simp [exactHit, foreignTx]

structure ReadCost where
  keyAndValueBytes : Nat
  candidateTxBytes : Nat
  blockAndUndoBytes : Nat
  deriving Repr

def chargedBytes (c : ReadCost) : Nat :=
  c.keyAndValueBytes + c.candidateTxBytes

def physicalBytes (c : ReadCost) : Nat :=
  chargedBytes c + c.blockAndUndoBytes

/-- Reading uncharged block/undo data exceeds the nominal index budget. -/
theorem read_budget_incomplete (c : ReadCost)
    (h : 0 < c.blockAndUndoBytes) :
    chargedBytes c < physicalBytes c := by
  simp only [physicalBytes]
  omega

def charge (remaining cost : Nat) : Option Nat :=
  if cost ≤ remaining then some (remaining - cost) else none

/-- A successful charge preserves the byte accounting equation. -/
theorem charge_conserves (remaining cost rest : Nat)
    (h : charge remaining cost = some rest) :
    cost + rest = remaining := by
  unfold charge at h
  split_ifs at h with affordable
  · cases h
    simpa [Nat.add_comm] using (Nat.sub_add_cancel affordable)
  · contradiction

end BitcoinCore36629
