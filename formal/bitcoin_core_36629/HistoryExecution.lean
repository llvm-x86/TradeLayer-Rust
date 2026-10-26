import IndexModel
import OffsetCodec
import Lean.Elab.Tactic.Omega

/-!
A small-step executable model of src/index/txoscriptindex.cpp::Find in
bitcoin/bitcoin#36629 at cf9b981fcc7ae52c7864beae35df5d60b5c322b4.

LevelDB Seek is modeled by supplying the suffix of rows beginning at the
searched key. The row prefixes, heights and transaction lists are trusted
inputs corresponding to successfully decoded database records. This model
charges the key *before* the range stop, then the value, then every candidate
transaction including a hash-wanted false positive. Disk/undo IO is distinct.
-/
set_option autoImplicit false

namespace BitcoinCore36629.HistoryExecution
open BitcoinCore36629

structure CandidateTx where
  tx : Tx
  serializedBytes : Nat
  deriving Repr, DecidableEq

structure Row where
  hashPrefix : Prefix
  height : Nat
  blockHash : Nat
  keyBytes : Nat
  valueBytes : Nat
  activeChain : Bool
  dataAvailable : Bool
  candidates : List CandidateTx
  deriving Repr, DecidableEq

def totalCandidateBytes (cs : List CandidateTx) : Nat :=
  (cs.map CandidateTx.serializedBytes).sum

/-- One charge, using the semantics of Find's shared uint64_t budget. -/
def charge (budget cost : Nat) : Option Nat :=
  if cost ≤ budget then some (budget - cost) else none

/-- Candidate bodies are read and charged even if they prove to be false hits. -/
def readCandidates (budget : Nat) : List CandidateTx → Option Nat
  | [] => some budget
  | c :: rest =>
      match charge budget c.serializedBytes with
      | none => none
      | some after => readCandidates after rest

theorem readCandidates_conserves (cs : List CandidateTx)
    (initial remaining : Nat)
    (h : readCandidates initial cs = some remaining) :
    remaining + totalCandidateBytes cs = initial := by
  induction cs generalizing initial remaining with
  | nil =>
      simp only [readCandidates, Option.some.injEq] at h
      simp [totalCandidateBytes, ← h]
  | cons c rest ih =>
      by_cases affordable : c.serializedBytes ≤ initial
      · simp [readCandidates, charge, affordable] at h
        have recur := ih (initial - c.serializedBytes) remaining h
        simp only [totalCandidateBytes, List.map_cons, List.sum_cons] at recur ⊢
        omega
      · simp [readCandidates, charge, affordable] at h

/-- Independent exact-script filter, applied only to transactions read from disk. -/
def exactResults (query : ScriptHash) (cs : List CandidateTx) : List Tx :=
  (cs.filter (fun c => decide (query ∈ c.tx.outputHashes))).map CandidateTx.tx

theorem exactResults_sound (query : ScriptHash) (cs : List CandidateTx)
    (tx : Tx) (h : tx ∈ exactResults query cs) : exactHit query tx := by
  unfold exactResults at h
  rcases List.mem_map.mp h with ⟨c, hc, rfl⟩
  have hmem := (List.mem_filter.mp hc).2
  simpa [exactHit] using hmem

/-- A row may cross the range boundary after its key has been charged. -/
inductive RowStep where
  | exhausted
  | stop (remaining : Nat)
  | next (remaining : Nat) (hits : List Tx) (pruned : List Nat)
  deriving Repr, DecidableEq

def step (query : ScriptHash) (wantedPrefix endHeight budget : Nat)
    (row : Row) : RowStep :=
  if row.keyBytes > budget then .exhausted
  else
    let afterKey := budget - row.keyBytes
    if row.hashPrefix != wantedPrefix || row.height > endHeight then .stop afterKey
    else if row.valueBytes > afterKey then .exhausted
    else
      let afterValue := afterKey - row.valueBytes
      if !row.activeChain then .next afterValue [] []
      else if !row.dataAvailable then .next afterValue [] [row.blockHash]
      else
        match readCandidates afterValue row.candidates with
        | none => .exhausted
        | some afterTxs => .next afterTxs (exactResults query row.candidates) []

/-- C++ charges an out-of-range key before terminating the iterator. -/
theorem boundary_key_charged (query wanted endHeight budget : Nat) (row : Row)
    (hBudget : row.keyBytes ≤ budget)
    (hOut : row.hashPrefix ≠ wanted ∨ row.height > endHeight) :
    step query wanted endHeight budget row =
      .stop (budget - row.keyBytes) := by
  simp [step, Nat.not_lt.mpr hBudget, hOut]

/-- An over-budget boundary key fails the RPC even though the row is excluded. -/
theorem boundary_key_can_exhaust (query wanted endHeight budget : Nat) (row : Row)
    (hBudget : budget < row.keyBytes) :
    step query wanted endHeight budget row = .exhausted := by
  simp [step, hBudget]


/-- Semantic invariant of any successfully returned row. -/
def SoundStep (query : ScriptHash) : RowStep → Prop
  | .next _ hits _ => ∀ tx ∈ hits, exactHit query tx
  | _ => True

/-- Regardless of collisions, pruning and a budget failure, executing
    any row can only emit exact script-hash matches. -/
theorem step_sound (query wanted endHeight budget : Nat) (row : Row) :
    SoundStep query (step query wanted endHeight budget row) := by
  have hExact : ∀ tx ∈ exactResults query row.candidates,
      exactHit query tx := by
    intro tx htx
    exact exactResults_sound query row.candidates tx htx
  by_cases hKey : budget < row.keyBytes
  · simp [step, SoundStep, hKey]
  · by_cases hOut : row.hashPrefix ≠ wanted ∨ endHeight < row.height
    · simp [step, SoundStep, hKey, hOut]
    · by_cases hValue : budget - row.keyBytes < row.valueBytes
      · simp [step, SoundStep, hKey, hOut, hValue]
      · by_cases hChain : row.activeChain = false
        · simp [step, SoundStep, hKey, hOut, hValue, hChain]
        · by_cases hAvailable : row.dataAvailable = false
          · simp [step, SoundStep, hKey, hOut, hValue, hChain, hAvailable]
          · cases hRead : readCandidates (budget - row.keyBytes - row.valueBytes)
                row.candidates with
            | none =>
                simp [step, SoundStep, hKey, hOut, hValue,
                  hChain, hAvailable, hRead]
            | some remaining =>
                simpa [step, SoundStep, hKey, hOut, hValue,
                  hChain, hAvailable, hRead] using hExact

/-- Complete row-scan outcome. A failure returns no partial response. -/
inductive ScanResult where
  | exhausted
  | complete (remaining : Nat) (hits : List Tx) (pruned : List Nat)
  deriving Repr, DecidableEq

/-- Caller supplies the iterator after Seek; once a boundary is reached we stop. -/
def scan (query : ScriptHash) (wanted endHeight : Nat) :
    Nat → List Row → ScanResult
  | budget, [] => .complete budget [] []
  | budget, row :: rest =>
      match step query wanted endHeight budget row with
      | .exhausted => .exhausted
      | .stop remaining => .complete remaining [] []
      | .next remaining found pruned =>
          match scan query wanted endHeight remaining rest with
          | .exhausted => .exhausted
          | .complete after moreMatches morePruned =>
              .complete after (found ++ moreMatches) (pruned ++ morePruned)

/-- A wanted collision costs bytes but cannot fabricate a verified match. -/
def collidingRow : Row :=
  { hashPrefix := 0, height := 8, blockHash := 123,
    keyBytes := 7, valueBytes := 2, activeChain := true,
    dataAvailable := true, candidates := [⟨foreignTx, 100⟩] }

theorem collision_filtered_and_charged :
    step 10 0 10 200 collidingRow = .next 91 [] [] := by decide

/-- A pruned row emits only a potential block hash, without a transaction. -/
def prunedRow : Row := { collidingRow with dataAvailable := false }

theorem pruned_block_placeholder :
    step 10 0 10 200 prunedRow = .next 191 [] [123] := by decide

/-- Key past the end height is charged, while value and transaction are skipped. -/
theorem range_stop_example :
    step 10 0 7 200 collidingRow = .stop 193 := by decide

/-- The boundary can itself trigger a budget error. -/
theorem boundary_error_example :
    step 10 0 7 6 collidingRow = .exhausted := by decide

end BitcoinCore36629.HistoryExecution
