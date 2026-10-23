import Lean

/-!
# TradeLayer accounting and replay: abstract proof contract

This kernel-checked model follows the arithmetic intent of
src/ledger.rs, src/ledger_settlement_impl.rs and src/network_profile.rs.
It does not prove that compiled Rust refines this model, authenticate a
transaction, or verify any Bitcoin/Litecoin/Dogecoin consensus input.

Amounts use Nat plus explicit u128 guards. Every accounting step names
its authorization/funding preconditions. Real authorization is an
external proof obligation.
-/

set_option autoImplicit false

namespace TradeLayerFormal

def maxU128 : Nat := 2 ^ 128 - 1

/-- A single property's projection onto two accounts and one bilateral
channel. These are columns of the Rust ledger, not a second ledger. -/
structure Portfolio where
  alice : Nat
  bob : Nat
  channelA : Nat
  channelB : Nat
  supply : Nat
  deriving Repr, DecidableEq

def circulating (s : Portfolio) : Nat :=
  s.alice + s.bob + s.channelA + s.channelB

def Conserved (s : Portfolio) : Prop := circulating s = s.supply

def Bounded (s : Portfolio) : Prop :=
  s.alice ≤ maxU128 ∧ s.bob ≤ maxU128 ∧
  s.channelA ≤ maxU128 ∧ s.channelB ≤ maxU128 ∧
  s.supply ≤ maxU128

/-- A transition may modify a single property only when its checked
debits, additions and settlement direction have been established.

The settlement constructors represent Type 31's net debit, after Type
23 claims have been checked. They deliberately make no assertion that
the journal/commitment actually authenticates underlying trades. -/
inductive AccountingStep : Portfolio → Portfolio → Prop where
  | send (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.alice)
      (creditFits : s.bob + n ≤ maxU128) :
      AccountingStep s
        {s with alice := s.alice - n, bob := s.bob + n}
  | commitA (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.alice)
      (creditFits : s.channelA + n ≤ maxU128) :
      AccountingStep s
        {s with alice := s.alice - n, channelA := s.channelA + n}
  | commitB (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.bob)
      (creditFits : s.channelB + n ≤ maxU128) :
      AccountingStep s
        {s with bob := s.bob - n, channelB := s.channelB + n}
  | withdrawA (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.channelA)
      (creditFits : s.alice + n ≤ maxU128) :
      AccountingStep s
        {s with channelA := s.channelA - n, alice := s.alice + n}
  | withdrawB (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.channelB)
      (creditFits : s.bob + n ≤ maxU128) :
      AccountingStep s
        {s with channelB := s.channelB - n, bob := s.bob + n}
  | settleA (s : Portfolio) (aToB bToA : Nat)
      (direction : bToA < aToB)
      (funded : aToB - bToA ≤ s.channelA)
      (creditFits : s.channelB + (aToB - bToA) ≤ maxU128) :
      AccountingStep s
        {s with channelA := s.channelA - (aToB - bToA),
                channelB := s.channelB + (aToB - bToA)}
  | settleB (s : Portfolio) (aToB bToA : Nat)
      (direction : aToB < bToA)
      (funded : bToA - aToB ≤ s.channelB)
      (creditFits : s.channelA + (bToA - aToB) ≤ maxU128) :
      AccountingStep s
        {s with channelB := s.channelB - (bToA - aToB),
                channelA := s.channelA + (bToA - aToB)}
  | managedMint (s : Portfolio) (n : Nat)
      (positive : 0 < n)
      (creditFits : s.alice + n ≤ maxU128)
      (supplyFits : s.supply + n ≤ maxU128) :
      AccountingStep s
        {s with alice := s.alice + n, supply := s.supply + n}
  | managedBurn (s : Portfolio) (n : Nat)
      (positive : 0 < n) (funded : n ≤ s.alice)
      (issued : n ≤ s.supply) :
      AccountingStep s
        {s with alice := s.alice - n, supply := s.supply - n}

/-- Every admitted step accounts for all of its input units. -/
theorem accounting_step_conserves
    {s t : Portfolio} (step : AccountingStep s t)
    (good : Conserved s) : Conserved t := by
  cases step <;> dsimp [Conserved, circulating] at * <;> omega

/-- Explicit overflow guards preserve the bounds of every u128 column. -/
theorem accounting_step_stays_bounded
    {s t : Portfolio} (step : AccountingStep s t)
    (good : Bounded s) : Bounded t := by
  rcases good with ⟨ha, hb, hca, hcb, hs⟩
  cases step <;> dsimp [Bounded] <;> omega

inductive AccountingRun : Portfolio → Portfolio → Prop where
  | refl (s : Portfolio) : AccountingRun s s
  | next {s t u : Portfolio} (prior : AccountingRun s t)
      (step : AccountingStep t u) : AccountingRun s u

/-- The conservation property holds after any finite sequence of admitted
transactions, including arbitrarily interleaved mints, burns and net sweeps. -/
theorem every_reachable_state_conserves
    {s t : Portfolio} (run : AccountingRun s t)
    (good : Conserved s) : Conserved t := by
  induction run with
  | refl => exact good
  | next prior step ih => exact accounting_step_conserves step ih

theorem every_reachable_state_bounded
    {s t : Portfolio} (run : AccountingRun s t)
    (good : Bounded s) : Bounded t := by
  induction run with
  | refl => exact good
  | next prior step ih => exact accounting_step_stays_bounded step ih

/-- The real code uses different genesis-bound ChainIds and independent
state machines. This type is the abstract three-mainnet namespace. -/
inductive Mainnet where
  | bitcoin
  | litecoin
  | dogecoin
  deriving Repr, DecidableEq

abbrev Asset := Mainnet × Nat

theorem same_property_id_on_distinct_mainnets_is_distinct
    (a b : Mainnet) (property : Nat) (distinct : a ≠ b) :
    ((a, property) : Asset) ≠ (b, property) := by
  intro equal
  exact distinct (congrArg Prod.fst equal)

/-- Replay markers are scoped by both mainnet and transaction identifier.
The Nat identifier abstracts a concrete 32-byte txid; byte-level parsing
and genesis-identity checks require an independent refinement proof. -/
structure Replay where
  seen : Mainnet → Nat → Bool

def mark (r : Replay) (net : Mainnet) (txid : Nat) : Replay :=
  { seen := fun n i =>
      if n = net then
        if i = txid then true else r.seen n i
      else r.seen n i }

def admit (r : Replay) (net : Mainnet) (txid : Nat) : Option Replay :=
  if r.seen net txid then none else some (mark r net txid)

theorem marked_transaction_is_used
    (r : Replay) (net : Mainnet) (txid : Nat) :
    (mark r net txid).seen net txid = true := by
  simp [mark]

/-- A transaction that was accepted once cannot be accepted a second time
under the same chain namespace. -/
theorem accepted_tx_cannot_be_replayed
    (r after : Replay) (net : Mainnet) (txid : Nat)
    (accepted : admit r net txid = some after) :
    admit after net txid = none := by
  cases prior : r.seen net txid with
  | true =>
      simp [admit, prior] at accepted
  | false =>
      have eq : mark r net txid = after := by
        simpa [admit, prior] using accepted
      rw [← eq]
      simp [admit, marked_transaction_is_used]

theorem marking_preserves_other_chain
    (r : Replay) (net other : Mainnet) (txid query : Nat)
    (different : other ≠ net) :
    (mark r net txid).seen other query = r.seen other query := by
  simp [mark, different]

/-- Reusing the exact same txid on a different mainnet does not inherit the
first chain's replay marker, even when local asset IDs are equal. -/
theorem cross_chain_txid_remains_independent
    (r : Replay) (net other : Mainnet) (txid : Nat)
    (different : other ≠ net) (unused : r.seen other txid = false) :
    (mark r net txid).seen other txid = false := by
  rw [marking_preserves_other_chain r net other txid txid different]
  exact unused

/-- This is a single-ledger-per-network protocol projection. The next-state
witness is an admitted arithmetic step for the selected mainnet. -/
structure Engine where
  balances : Mainnet → Portfolio
  replay : Replay

def AllConserved (e : Engine) : Prop :=
  ∀ net, Conserved (e.balances net)

def AllBounded (e : Engine) : Prop :=
  ∀ net, Bounded (e.balances net)

/-- One atomic state update, with the replay marker and ledger column scoped
to the same network. The Nat txid and eligibility proof must eventually be
derived from a verified Rust transaction/chain adapter. -/
def finish (e : Engine) (net : Mainnet) (txid : Nat)
    (next : Portfolio) : Engine :=
  { balances := fun other => if other = net then next else e.balances other,
    replay := mark e.replay net txid }

/-- An abstract proof-carrying transition: the caller must supply a Lean
witness of a permitted accounting step to construct this command. The Rust
engine does not yet produce such a witness. -/
def tryCommit (e : Engine) (net : Mainnet) (txid : Nat)
    (next : Portfolio) (_admitted : AccountingStep (e.balances net) next) :
    Option Engine :=
  if e.replay.seen net txid then none else some (finish e net txid next)

theorem committed_conserves_every_network
    (e after : Engine) (net : Mainnet) (txid : Nat)
    (next : Portfolio) (step : AccountingStep (e.balances net) next)
    (good : AllConserved e)
    (accepted : tryCommit e net txid next step = some after) :
    AllConserved after := by
  cases used : e.replay.seen net txid with
  | true => simp [tryCommit, used] at accepted
  | false =>
      have h : finish e net txid next = after := by
        simpa [tryCommit, used] using accepted
      rw [← h]
      intro other
      by_cases same : other = net
      · subst other
        simpa [finish] using accounting_step_conserves step (good net)
      · simpa [finish, same] using good other

theorem committed_bounds_every_network
    (e after : Engine) (net : Mainnet) (txid : Nat)
    (next : Portfolio) (step : AccountingStep (e.balances net) next)
    (good : AllBounded e)
    (accepted : tryCommit e net txid next step = some after) :
    AllBounded after := by
  cases used : e.replay.seen net txid with
  | true => simp [tryCommit, used] at accepted
  | false =>
      have h : finish e net txid next = after := by
        simpa [tryCommit, used] using accepted
      rw [← h]
      intro other
      by_cases same : other = net
      · subst other
        simpa [finish] using accounting_step_stays_bounded step (good net)
      · simpa [finish, same] using good other

theorem committed_marks_same_chain_replay
    (e after : Engine) (net : Mainnet) (txid : Nat)
    (next : Portfolio) (step : AccountingStep (e.balances net) next)
    (accepted : tryCommit e net txid next step = some after) :
    after.replay.seen net txid = true := by
  cases used : e.replay.seen net txid with
  | true => simp [tryCommit, used] at accepted
  | false =>
      have h : finish e net txid next = after := by
        simpa [tryCommit, used] using accepted
      rw [← h]
      exact marked_transaction_is_used e.replay net txid

theorem committed_leaves_other_ledger_untouched
    (e after : Engine) (net other : Mainnet) (txid : Nat)
    (next : Portfolio) (step : AccountingStep (e.balances net) next)
    (different : other ≠ net)
    (accepted : tryCommit e net txid next step = some after) :
    after.balances other = e.balances other := by
  cases used : e.replay.seen net txid with
  | true => simp [tryCommit, used] at accepted
  | false =>
      have h : finish e net txid next = after := by
        simpa [tryCommit, used] using accepted
      rw [← h]
      simp [finish, different]

/-!
## Checked debit/credit refinement target

This defines the exact three rejection guards used by the production Rust
`checked_amounts::move_atoms` function. Kani separately checks the *actual
Rust function* over all u128 input triples. The two checkers have independent
semantics; their agreement is made reviewable through the guard sequence and
the Rust source. No compiler-correctness or cross-language linking theorem
is assumed.
-/

inductive MoveRejection where
  | zero
  | insufficient
  | overflow
  deriving Repr, DecidableEq

def checkedMove (debit credit amount : Nat) :
    Except MoveRejection (Nat × Nat) :=
  if amount = 0 then .error .zero
  else if debit < amount then .error .insufficient
  else if maxU128 < credit + amount then .error .overflow
  else .ok (debit - amount, credit + amount)

theorem checked_move_valid (debit credit amount : Nat)
    (positive : 0 < amount) (funded : amount ≤ debit)
    (creditFits : credit + amount ≤ maxU128) :
    checkedMove debit credit amount =
      .ok (debit - amount, credit + amount) := by
  have nz : amount ≠ 0 := by omega
  have enough : ¬ debit < amount := by omega
  have fits : ¬ maxU128 < credit + amount := by omega
  simp [checkedMove, nz, enough, fits]

theorem checked_move_zero (debit credit : Nat) :
    checkedMove debit credit 0 = .error .zero := by
  simp [checkedMove]

theorem checked_move_insufficient (debit credit amount : Nat)
    (positive : 0 < amount) (unfunded : debit < amount) :
    checkedMove debit credit amount = .error .insufficient := by
  have nz : amount ≠ 0 := by omega
  simp [checkedMove, nz, unfunded]

theorem checked_move_overflow (debit credit amount : Nat)
    (positive : 0 < amount) (funded : amount ≤ debit)
    (overflow : maxU128 < credit + amount) :
    checkedMove debit credit amount = .error .overflow := by
  have nz : amount ≠ 0 := by omega
  have enough : ¬ debit < amount := by omega
  simp [checkedMove, nz, enough, overflow]

theorem checked_move_success_is_exact (debit credit amount afterDebit afterCredit : Nat)
    (accepted : checkedMove debit credit amount =
      .ok (afterDebit, afterCredit)) :
    0 < amount ∧ amount ≤ debit ∧
    credit + amount ≤ maxU128 ∧
    afterDebit = debit - amount ∧ afterCredit = credit + amount := by
  unfold checkedMove at accepted
  by_cases hz : amount = 0
  · simp [hz] at accepted
  · by_cases hf : debit < amount
    · simp [hz, hf] at accepted
    · by_cases ho : maxU128 < credit + amount
      · simp [hz, hf, ho] at accepted
      · have pairs : (debit - amount, credit + amount) =
            (afterDebit, afterCredit) := by
          simpa [hz, hf, ho] using accepted
        rcases Prod.mk.inj pairs with ⟨hDebit, hCredit⟩
        constructor
        · omega
        constructor
        · omega
        constructor
        · omega
        constructor
        · omega
        · omega

theorem checked_move_preserves_sum (debit credit amount afterDebit afterCredit : Nat)
    (accepted : checkedMove debit credit amount =
      .ok (afterDebit, afterCredit)) :
    afterDebit + afterCredit = debit + credit := by
  obtain ⟨_, funded, _, debitEq, creditEq⟩ :=
    checked_move_success_is_exact debit credit amount afterDebit afterCredit accepted
  omega

theorem checked_move_preserves_u128 (debit credit amount afterDebit afterCredit : Nat)
    (boundedDebit : debit ≤ maxU128) (_boundedCredit : credit ≤ maxU128)
    (accepted : checkedMove debit credit amount =
      .ok (afterDebit, afterCredit)) :
    afterDebit ≤ maxU128 ∧ afterCredit ≤ maxU128 := by
  obtain ⟨_, funded, fits, debitEq, creditEq⟩ :=
    checked_move_success_is_exact debit credit amount afterDebit afterCredit accepted
  omega


/-!
## Atomic multi-transaction replay

Rust's `Ledger::apply_batch` validates a cloned candidate and publishes it
only after every operation and final conservation check succeed. This abstract
model stages certified transitions and projects either the successful state
or the original live state. Individual transition certification remains a
separate obligation from Rust-to-Lean refinement.
-/

/-- Each operation supplies an implementation-shaped proposal and a proof
that every successful proposal refines the accounting step relation.
Rejection may represent authorization failure, arithmetic failure or an
unavailable settlement proof. The executor still checks replay separately. -/
structure CertifiedBatchTx where
  network : Mainnet
  txid : Nat
  run : Portfolio → Option Portfolio
  sound : ∀ s t, run s = some t → AccountingStep s t

def stageTx (e : Engine) (tx : CertifiedBatchTx) : Option Engine :=
  if e.replay.seen tx.network tx.txid then none
  else
    match tx.run (e.balances tx.network) with
    | none => none
    | some updated => some (finish e tx.network tx.txid updated)

theorem stage_tx_effect (e after : Engine) (tx : CertifiedBatchTx)
    (accepted : stageTx e tx = some after) :
    ∃ updated, tx.run (e.balances tx.network) = some updated ∧
      after = finish e tx.network tx.txid updated := by
  cases used : e.replay.seen tx.network tx.txid with
  | true =>
      simp [stageTx, used] at accepted
  | false =>
      cases proposed : tx.run (e.balances tx.network) with
      | none =>
          simp [stageTx, used, proposed] at accepted
      | some updated =>
          have eq : finish e tx.network tx.txid updated = after := by
            simpa [stageTx, used, proposed] using accepted
          exact ⟨updated, rfl, eq.symm⟩

theorem stage_tx_conserves (e after : Engine) (tx : CertifiedBatchTx)
    (accepted : stageTx e tx = some after)
    (good : AllConserved e) : AllConserved after := by
  obtain ⟨updated, valid, eq⟩ := stage_tx_effect e after tx accepted
  subst after
  have transition : AccountingStep (e.balances tx.network) updated :=
    tx.sound (e.balances tx.network) updated valid
  intro other
  by_cases same : other = tx.network
  · subst other
    simpa [finish] using accounting_step_conserves transition (good tx.network)
  · simpa [finish, same] using good other

theorem stage_tx_stays_bounded (e after : Engine) (tx : CertifiedBatchTx)
    (accepted : stageTx e tx = some after)
    (good : AllBounded e) : AllBounded after := by
  obtain ⟨updated, valid, eq⟩ := stage_tx_effect e after tx accepted
  subst after
  have transition : AccountingStep (e.balances tx.network) updated :=
    tx.sound (e.balances tx.network) updated valid
  intro other
  by_cases same : other = tx.network
  · subst other
    simpa [finish] using accounting_step_stays_bounded transition (good tx.network)
  · simpa [finish, same] using good other

theorem stage_tx_marks_replay (e after : Engine) (tx : CertifiedBatchTx)
    (accepted : stageTx e tx = some after) :
    after.replay.seen tx.network tx.txid = true := by
  obtain ⟨updated, _, eq⟩ := stage_tx_effect e after tx accepted
  subst after
  simpa [finish] using marked_transaction_is_used e.replay tx.network tx.txid

/-- A staged prefix is an internal candidate; any failure returns none. -/
def stagedBatch (e : Engine) : List CertifiedBatchTx → Option Engine
  | [] => some e
  | tx :: tail =>
      match stageTx e tx with
      | none => none
      | some mid => stagedBatch mid tail

theorem staged_batch_conserves (txs : List CertifiedBatchTx) :
    ∀ e after : Engine, stagedBatch e txs = some after →
      AllConserved e → AllConserved after := by
  induction txs with
  | nil =>
      intro e after accepted good
      have eq : e = after := by simpa [stagedBatch] using accepted
      rw [← eq]
      exact good
  | cons tx tail ih =>
      intro e after accepted good
      cases step : stageTx e tx with
      | none =>
          simp [stagedBatch, step] at accepted
      | some mid =>
          have tailAccepted : stagedBatch mid tail = some after := by
            simpa [stagedBatch, step] using accepted
          exact ih mid after tailAccepted (stage_tx_conserves e mid tx step good)

theorem staged_batch_stays_bounded (txs : List CertifiedBatchTx) :
    ∀ e after : Engine, stagedBatch e txs = some after →
      AllBounded e → AllBounded after := by
  induction txs with
  | nil =>
      intro e after accepted good
      have eq : e = after := by simpa [stagedBatch] using accepted
      rw [← eq]
      exact good
  | cons tx tail ih =>
      intro e after accepted good
      cases step : stageTx e tx with
      | none =>
          simp [stagedBatch, step] at accepted
      | some mid =>
          have tailAccepted : stagedBatch mid tail = some after := by
            simpa [stagedBatch, step] using accepted
          exact ih mid after tailAccepted
            (stage_tx_stays_bounded e mid tx step good)

/-- Published state plus whether the staged batch committed. This is the
abstract observable projection of Rust's Result-based commit boundary. -/
def batchResult (e : Engine) (txs : List CertifiedBatchTx) : Engine × Bool :=
  match stagedBatch e txs with
  | none => (e, false)
  | some after => (after, true)

theorem rejected_batch_restores_full_state
    (e : Engine) (txs : List CertifiedBatchTx)
    (failed : (batchResult e txs).2 = false) :
    (batchResult e txs).1 = e := by
  cases result : stagedBatch e txs with
  | none =>
      simp [batchResult, result]
  | some after =>
      simp [batchResult, result] at failed

theorem rejected_batch_restores_replay_marker
    (e : Engine) (txs : List CertifiedBatchTx)
    (net : Mainnet) (txid : Nat)
    (failed : (batchResult e txs).2 = false) :
    (batchResult e txs).1.replay.seen net txid = e.replay.seen net txid := by
  rw [rejected_batch_restores_full_state e txs failed]

theorem committed_batch_conserves_all_networks
    (e : Engine) (txs : List CertifiedBatchTx)
    (good : AllConserved e)
    (succeeded : (batchResult e txs).2 = true) :
    AllConserved (batchResult e txs).1 := by
  cases result : stagedBatch e txs with
  | none =>
      simp [batchResult, result] at succeeded
  | some after =>
      simpa [batchResult, result] using
        (staged_batch_conserves txs e after result good)

theorem committed_batch_bounds_all_networks
    (e : Engine) (txs : List CertifiedBatchTx)
    (good : AllBounded e)
    (succeeded : (batchResult e txs).2 = true) :
    AllBounded (batchResult e txs).1 := by
  cases result : stagedBatch e txs with
  | none =>
      simp [batchResult, result] at succeeded
  | some after =>
      simpa [batchResult, result] using
        (staged_batch_stays_bounded txs e after result good)

theorem repeated_txid_fails_staging (e : Engine) (tx : CertifiedBatchTx) :
    stagedBatch e [tx, tx] = none := by
  cases first : stageTx e tx with
  | none =>
      simp [stagedBatch, first]
  | some middle =>
      have used : middle.replay.seen tx.network tx.txid = true :=
        stage_tx_marks_replay e middle tx first
      have second : stageTx middle tx = none := by
        simp [stageTx, used]
      simp [stagedBatch, first, second]

theorem repeated_txid_rolls_back_everything (e : Engine)
    (tx : CertifiedBatchTx) :
    batchResult e [tx, tx] = (e, false) := by
  simp [batchResult, repeated_txid_fails_staging]

theorem staged_batch_append (e : Engine)
    (front back : List CertifiedBatchTx) :
    stagedBatch e (front ++ back) =
      (stagedBatch e front).bind (fun mid => stagedBatch mid back) := by
  induction front generalizing e with
  | nil =>
      simp [stagedBatch]
  | cons tx tail ih =>
      cases step : stageTx e tx with
      | none =>
          simp [stagedBatch, step]
      | some middle =>
          simp [stagedBatch, step, ih]

theorem valid_prefix_then_rejection_rolls_back
    (e middle : Engine) (front : List CertifiedBatchTx)
    (bad : CertifiedBatchTx)
    (validPrefix : stagedBatch e front = some middle)
    (rejected : stageTx middle bad = none) :
    batchResult e (front ++ [bad]) = (e, false) := by
  have wholeFails : stagedBatch e (front ++ [bad]) = none := by
    rw [staged_batch_append, validPrefix]
    simp [stagedBatch, rejected]
  simp [batchResult, wholeFails]


/-- Concrete transfer candidate sourced from the same checked Nat arithmetic
specification as the Kani-verified Rust debit/credit kernel. -/
def directSendProposal (amount : Nat) (s : Portfolio) : Option Portfolio :=
  match checkedMove s.alice s.bob amount with
  | .error _ => none
  | .ok (nextA, nextB) =>
      some {s with alice := nextA, bob := nextB}

theorem direct_send_proposal_refines_accounting
    (s after : Portfolio) (amount : Nat)
    (success : directSendProposal amount s = some after) :
    AccountingStep s after := by
  cases arithmetic : checkedMove s.alice s.bob amount with
  | error why =>
      simp [directSendProposal, arithmetic] at success
  | ok columns =>
      rcases columns with ⟨nextA, nextB⟩
      have updated : {s with alice := nextA, bob := nextB} = after := by
        simpa [directSendProposal, arithmetic] using success
      obtain ⟨positive, funded, creditFits, debitEq, creditEq⟩ :=
        checked_move_success_is_exact s.alice s.bob amount nextA nextB arithmetic
      rw [← updated]
      simpa [debitEq, creditEq] using
        (AccountingStep.send s amount positive funded creditFits)

/-- The proof-carrying operation is constructed from an actual checked
transfer specification. Other protocol opcodes need separate refinements. -/
def certifiedDirectSend (net : Mainnet) (txid amount : Nat) : CertifiedBatchTx :=
  { network := net
    txid := txid
    run := directSendProposal amount
    sound := by
      intro s after accepted
      exact direct_send_proposal_refines_accounting s after amount accepted }

theorem zero_direct_send_rejected (e : Engine) (net : Mainnet) (txid : Nat) :
    stageTx e (certifiedDirectSend net txid 0) = none := by
  simp [stageTx, certifiedDirectSend, directSendProposal, checkedMove]

theorem stage_tx_retains_existing_replay (e after : Engine)
    (tx : CertifiedBatchTx) (net : Mainnet) (txid : Nat)
    (accepted : stageTx e tx = some after)
    (seen : e.replay.seen net txid = true) :
    after.replay.seen net txid = true := by
  obtain ⟨updated, _, eq⟩ := stage_tx_effect e after tx accepted
  subst after
  simp [finish, mark, seen]

theorem staged_batch_retains_existing_replay (txs : List CertifiedBatchTx) :
    ∀ (e after : Engine) (net : Mainnet) (txid : Nat),
      stagedBatch e txs = some after →
      e.replay.seen net txid = true →
      after.replay.seen net txid = true := by
  induction txs with
  | nil =>
      intro e after net txid accepted seen
      have same : e = after := by simpa [stagedBatch] using accepted
      rw [← same]
      exact seen
  | cons tx tail ih =>
      intro e after net txid accepted seen
      cases one : stageTx e tx with
      | none =>
          simp [stagedBatch, one] at accepted
      | some middle =>
          have restAccepted : stagedBatch middle tail = some after := by
            simpa [stagedBatch, one] using accepted
          exact ih middle after net txid restAccepted
            (stage_tx_retains_existing_replay e middle tx net txid one seen)

theorem previously_seen_txid_rejects_at_end
    (e : Engine) (intervening : List CertifiedBatchTx)
    (last : CertifiedBatchTx)
    (alreadySeen : e.replay.seen last.network last.txid = true) :
    stagedBatch e (intervening ++ [last]) = none := by
  rw [staged_batch_append]
  cases intermediate : stagedBatch e intervening with
  | none =>
      simp
  | some after =>
      have stillSeen : after.replay.seen last.network last.txid = true :=
        staged_batch_retains_existing_replay intervening e after
          last.network last.txid intermediate alreadySeen
      have rejected : stageTx after last = none := by
        simp [stageTx, stillSeen]
      simp [stagedBatch, rejected]

/-- Even with an arbitrarily long successful prefix of unrelated operations,
a duplicate within one TradeLayer forces the entire batch to reject. -/
theorem nonadjacent_duplicate_txid_rejects_batch
    (e : Engine) (first last : CertifiedBatchTx)
    (intervening : List CertifiedBatchTx)
    (sameNet : last.network = first.network)
    (sameTxid : last.txid = first.txid) :
    batchResult e (first :: intervening ++ [last]) = (e, false) := by
  have stagingFails :
      stagedBatch e (first :: intervening ++ [last]) = none := by
    cases attempted : stageTx e first with
    | none =>
        simp [stagedBatch, attempted]
    | some middle =>
        have marked : middle.replay.seen first.network first.txid = true :=
          stage_tx_marks_replay e middle first attempted
        have sameMarked : middle.replay.seen last.network last.txid = true := by
          simpa [sameNet, sameTxid] using marked
        have suffixFails :=
          previously_seen_txid_rejects_at_end middle intervening last sameMarked
        simpa [stagedBatch, attempted] using suffixFails
  unfold batchResult
  rw [stagingFails]


/-!
## Arbitrarily many property identities

The Rust ledger stores issued supply and available/escrow balances in ordered
maps keyed by PropertyId. The following is a functional per-property model:
all other property projections must be untouched by a local accounting step.
The external Rust BTreeMap/identity correspondence remains open.
-/

structure PropertyLedgerView where
  byId : Nat → Portfolio

def propertyUpdate (s : PropertyLedgerView) (property : Nat)
    (next : Portfolio) : PropertyLedgerView :=
  { byId := fun other => if other = property then next else s.byId other }

theorem property_update_keeps_other_assets (s : PropertyLedgerView)
    (property other : Nat) (next : Portfolio) (different : other ≠ property) :
    (propertyUpdate s property next).byId other = s.byId other := by
  simp [propertyUpdate, different]

theorem property_update_preserves_all_conservation
    (s : PropertyLedgerView) (property : Nat) (next : Portfolio)
    (valid : ∀ id, Conserved (s.byId id))
    (admitted : AccountingStep (s.byId property) next) :
    ∀ id, Conserved ((propertyUpdate s property next).byId id) := by
  intro id
  by_cases same : id = property
  · subst id
    simpa [propertyUpdate] using
      accounting_step_conserves admitted (valid property)
  · simpa [propertyUpdate, same] using valid id

theorem property_update_preserves_all_bounds
    (s : PropertyLedgerView) (property : Nat) (next : Portfolio)
    (valid : ∀ id, Bounded (s.byId id))
    (admitted : AccountingStep (s.byId property) next) :
    ∀ id, Bounded ((propertyUpdate s property next).byId id) := by
  intro id
  by_cases same : id = property
  · subst id
    simpa [propertyUpdate] using
      accounting_step_stays_bounded admitted (valid property)
  · simpa [propertyUpdate, same] using valid id

/-!
## Claim-presence map and single-use sweep

Each Nat represents a claim TxId. A successful sweep transfers membership
between disjoint pending/swept maps with exactly one local update to each.
This checks presence semantics. Trade-reference uniqueness, receipt roots,
timestamps, economic validity and real SHA-256 bindings are external.
-/

structure JournalView where
  pending : Nat → Bool
  swept : Nat → Bool

def JournalDisjoint (j : JournalView) : Prop :=
  ∀ id, j.pending id = true → j.swept id = false

def journalMove (j : JournalView) (key : Nat) : Option JournalView :=
  if j.swept key then none
  else if j.pending key then
    some
      { pending := fun query => if query = key then false else j.pending query,
        swept := fun query => if query = key then true else j.swept query }
  else none

theorem journal_move_rejects_consumed (j : JournalView) (key : Nat)
    (consumed : j.swept key = true) :
    journalMove j key = none := by
  simp [journalMove, consumed]

theorem journal_move_rejects_missing (j : JournalView) (key : Nat)
    (fresh : j.swept key = false) (missing : j.pending key = false) :
    journalMove j key = none := by
  simp [journalMove, fresh, missing]

theorem journal_move_success_shape
    (j after : JournalView) (key : Nat)
    (accepted : journalMove j key = some after) :
    j.swept key = false ∧ j.pending key = true ∧
    after =
      { pending := fun query => if query = key then false else j.pending query,
        swept := fun query => if query = key then true else j.swept query } := by
  cases swept : j.swept key with
  | true =>
      simp [journalMove, swept] at accepted
  | false =>
      cases pending : j.pending key with
      | false =>
          simp [journalMove, swept, pending] at accepted
      | true =>
          have shape :
              ({ pending := fun query => if query = key then false else j.pending query,
                 swept := fun query => if query = key then true else j.swept query } :
                 JournalView) = after := by
              simpa [journalMove, swept, pending] using accepted
          exact ⟨rfl, rfl, shape.symm⟩

theorem journal_move_marks_swept (j after : JournalView) (key : Nat)
    (accepted : journalMove j key = some after) :
    after.pending key = false ∧ after.swept key = true := by
  obtain ⟨_, _, shape⟩ :=
    journal_move_success_shape j after key accepted
  rw [shape]
  simp

theorem journal_move_other_keys_unchanged
    (j after : JournalView) (key other : Nat)
    (accepted : journalMove j key = some after) (different : other ≠ key) :
    after.pending other = j.pending other ∧
    after.swept other = j.swept other := by
  obtain ⟨_, _, shape⟩ :=
    journal_move_success_shape j after key accepted
  rw [shape]
  simp [different]

theorem journal_move_preserves_disjoint
    (j after : JournalView) (key : Nat)
    (valid : JournalDisjoint j)
    (accepted : journalMove j key = some after) :
    JournalDisjoint after := by
  obtain ⟨_, _, shape⟩ :=
    journal_move_success_shape j after key accepted
  rw [shape]
  intro other pending
  by_cases same : other = key
  · simp [same] at pending
  · have previous : j.pending other = true := by
      simpa [same] using pending
    simpa [same] using valid other previous

theorem journal_move_cannot_repeat (j after : JournalView) (key : Nat)
    (accepted : journalMove j key = some after) :
    journalMove after key = none := by
  have swept := (journal_move_marks_swept j after key accepted).2
  exact journal_move_rejects_consumed after key swept


/-!
## Arbitrary finite accounting columns

An individual property can have arbitrarily many wallet owners, escrow
columns and reserved withdrawal entries. Here every type of storage column
receives a distinct abstract Nat identifier and the support list is finite,
complete and duplicate-free. This representation is independent from the
Rust BTreeMap implementation and its concrete composite-key encoding.
-/

def columnTotal : List Nat → (Nat → Nat) → Nat
  | [], _ => 0
  | key :: rest, amounts => amounts key + columnTotal rest amounts

def columnPut (amounts : Nat → Nat) (key newValue : Nat) : Nat → Nat :=
  fun query => if query = key then newValue else amounts query

theorem column_total_put_absent
    (keys : List Nat) (amounts : Nat → Nat) (key newValue : Nat)
    (absent : key ∉ keys) :
    columnTotal keys (columnPut amounts key newValue) =
      columnTotal keys amounts := by
  induction keys with
  | nil => rfl
  | cons head rest ih =>
      have unequal : head ≠ key := by
        intro same
        apply absent
        simp [same]
      have restAbsent : key ∉ rest := by
        intro member
        apply absent
        simp [member]
      simp [columnTotal, columnPut, unequal, ih restAbsent]

/-- Replacing one covered, unique balance has a precise aggregate delta.
This applies to any finite number of distinct wallet/channel/reservation
columns, and proves the arithmetic fact behind BTreeMap aggregation. -/
theorem column_total_put
    (keys : List Nat) (amounts : Nat → Nat) (key newValue : Nat)
    (unique : keys.Nodup) (present : key ∈ keys) :
    columnTotal keys (columnPut amounts key newValue) + amounts key =
      columnTotal keys amounts + newValue := by
  induction keys with
  | nil => simp at present
  | cons head rest ih =>
      have noTail : head ∉ rest := (List.nodup_cons.mp unique).1
      have uniqueTail : rest.Nodup := (List.nodup_cons.mp unique).2
      by_cases same : head = key
      · subst head
        have absentTail := column_total_put_absent rest amounts key newValue noTail
        simp [columnTotal, columnPut, absentTail]
        omega
      · have inTail : key ∈ rest := by
          have presentOrTail := List.mem_cons.mp present
          exact presentOrTail.resolve_left (by simpa [eq_comm] using same)
        have tailEquation := ih uniqueTail inTail
        simp only [columnTotal]
        simp [columnPut, same]
        omega

def columnMove (amounts : Nat → Nat) (debitKey creditKey amount : Nat) :
    Nat → Nat :=
  columnPut
    (columnPut amounts debitKey (amounts debitKey - amount))
    creditKey (amounts creditKey + amount)

theorem column_move_preserves_total
    (keys : List Nat) (amounts : Nat → Nat)
    (debitKey creditKey amount : Nat)
    (unique : keys.Nodup)
    (debitPresent : debitKey ∈ keys)
    (creditPresent : creditKey ∈ keys)
    (distinct : debitKey ≠ creditKey)
    (funded : amount ≤ amounts debitKey) :
    columnTotal keys (columnMove amounts debitKey creditKey amount) =
      columnTotal keys amounts := by
  let intermediate := columnPut amounts debitKey (amounts debitKey - amount)
  have first := column_total_put keys amounts debitKey
    (amounts debitKey - amount) unique debitPresent
  have second := column_total_put keys intermediate creditKey
    (amounts creditKey + amount) unique creditPresent
  have creditUnchanged : intermediate creditKey = amounts creditKey := by
    simp [intermediate, columnPut, Ne.symm distinct]
  change columnTotal keys intermediate + amounts debitKey =
    columnTotal keys amounts + (amounts debitKey - amount) at first
  change columnTotal keys (columnMove amounts debitKey creditKey amount) +
    intermediate creditKey =
    columnTotal keys intermediate + (amounts creditKey + amount) at second
  rw [creditUnchanged] at second
  omega

theorem column_move_frame
    (amounts : Nat → Nat) (debitKey creditKey amount other : Nat)
    (notDebit : other ≠ debitKey) (notCredit : other ≠ creditKey) :
    columnMove amounts debitKey creditKey amount other = amounts other := by
  simp [columnMove, columnPut, notDebit, notCredit]

theorem column_move_debit
    (amounts : Nat → Nat) (debitKey creditKey amount : Nat)
    (distinct : debitKey ≠ creditKey) :
    columnMove amounts debitKey creditKey amount debitKey =
      amounts debitKey - amount := by
  simp [columnMove, columnPut, distinct]

theorem column_move_credit
    (amounts : Nat → Nat) (debitKey creditKey amount : Nat)
    (_distinct : debitKey ≠ creditKey) :
    columnMove amounts debitKey creditKey amount creditKey =
      amounts creditKey + amount := by
  simp [columnMove, columnPut]

/-- The support of the actual ledger includes positive-value available
accounts, channel escrow entries and pending withdrawal TxIds. The support
list here may also contain zero-valued slots to permit a move without
rebuilding the support. It is an abstract set of candidate columns. -/
structure FiniteSupplyView where
  slots : List Nat
  atoms : Nat → Nat
  supply : Nat

def FiniteSupplyValid (s : FiniteSupplyView) : Prop :=
  s.slots.Nodup ∧
  (∀ key, key ∉ s.slots → s.atoms key = 0) ∧
  columnTotal s.slots s.atoms = s.supply ∧
  (∀ key, key ∈ s.slots → s.atoms key ≤ maxU128) ∧
  s.supply ≤ maxU128

def finiteMove (s : FiniteSupplyView)
    (debitKey creditKey amount : Nat) : FiniteSupplyView :=
  { s with atoms := columnMove s.atoms debitKey creditKey amount }

theorem finite_move_preserves_validity
    (s : FiniteSupplyView) (debitKey creditKey amount : Nat)
    (good : FiniteSupplyValid s)
    (debitPresent : debitKey ∈ s.slots)
    (creditPresent : creditKey ∈ s.slots)
    (distinct : debitKey ≠ creditKey)
    (positive : 0 < amount)
    (funded : amount ≤ s.atoms debitKey)
    (creditFits : s.atoms creditKey + amount ≤ maxU128) :
    FiniteSupplyValid (finiteMove s debitKey creditKey amount) := by
  rcases good with ⟨unique, covered, conserved, bounded, supplyFits⟩
  refine ⟨unique, ?_, ?_, ?_, supplyFits⟩
  · intro key absent
    have outside : key ∉ s.slots := by simpa [finiteMove] using absent
    have otherDebit : key ≠ debitKey := by
      intro same
      apply outside
      simpa [same] using debitPresent
    have otherCredit : key ≠ creditKey := by
      intro same
      apply outside
      simpa [same] using creditPresent
    change columnMove s.atoms debitKey creditKey amount key = 0
    rw [column_move_frame s.atoms debitKey creditKey amount
      key otherDebit otherCredit]
    exact covered key outside
  · simpa [finiteMove] using
      (column_move_preserves_total s.slots s.atoms
        debitKey creditKey amount unique debitPresent
        creditPresent distinct funded).trans conserved
  · intro key present
    have inSlots : key ∈ s.slots := by simpa [finiteMove] using present
    change columnMove s.atoms debitKey creditKey amount key ≤ maxU128
    by_cases isDebit : key = debitKey
    · subst key
      rw [column_move_debit s.atoms debitKey creditKey amount distinct]
      have debitBound := bounded debitKey debitPresent
      omega
    · by_cases isCredit : key = creditKey
      · subst key
        rw [column_move_credit s.atoms debitKey creditKey amount distinct]
        exact creditFits
      · rw [column_move_frame s.atoms debitKey creditKey amount
          key isDebit isCredit]
        exact bounded key inSlots

def finiteMint (s : FiniteSupplyView) (key amount : Nat) : FiniteSupplyView :=
  { s with atoms := columnPut s.atoms key (s.atoms key + amount),
           supply := s.supply + amount }

def finiteBurn (s : FiniteSupplyView) (key amount : Nat) : FiniteSupplyView :=
  { s with atoms := columnPut s.atoms key (s.atoms key - amount),
           supply := s.supply - amount }

theorem finite_mint_preserves_total
    (s : FiniteSupplyView) (key amount : Nat)
    (conserved : columnTotal s.slots s.atoms = s.supply)
    (unique : s.slots.Nodup) (present : key ∈ s.slots) :
    columnTotal (finiteMint s key amount).slots (finiteMint s key amount).atoms =
      (finiteMint s key amount).supply := by
  have updated := column_total_put s.slots s.atoms key
    (s.atoms key + amount) unique present
  dsimp [finiteMint]
  omega

theorem finite_burn_preserves_total
    (s : FiniteSupplyView) (key amount : Nat)
    (conserved : columnTotal s.slots s.atoms = s.supply)
    (unique : s.slots.Nodup) (present : key ∈ s.slots)
    (funded : amount ≤ s.atoms key)
    (supplyEnough : amount ≤ s.supply) :
    columnTotal (finiteBurn s key amount).slots (finiteBurn s key amount).atoms =
      (finiteBurn s key amount).supply := by
  have updated := column_total_put s.slots s.atoms key
    (s.atoms key - amount) unique present
  dsimp [finiteBurn]
  omega


/-!
## Collision-free column namespaces

The abstract finite-support proof above treats every accounting column as
a distinct Nat. A namespace digit in {0,1,2,3} distinguishes available,
channel-A, channel-B, and reserved-withdrawal columns. Each concrete
identifier (owner, channel or TxId) still requires an injective
representation in Nat for an executable Rust-to-Lean refinement.
-/

def taggedColumn (id tag : Nat) : Nat := 4 * id + tag

theorem tagged_columns_are_disjoint
    (idA idB tagA tagB : Nat)
    (validA : tagA < 4) (validB : tagB < 4)
    (differentTag : tagA ≠ tagB) :
    taggedColumn idA tagA ≠ taggedColumn idB tagB := by
  unfold taggedColumn
  omega

theorem tagged_columns_preserve_identifier
    (idA idB tag : Nat) :
    taggedColumn idA tag = taggedColumn idB tag → idA = idB := by
  simp [taggedColumn]
  omega

def availableColumn (account : Nat) : Nat := taggedColumn account 0
def escrowAColumn (channel : Nat) : Nat := taggedColumn channel 1
def escrowBColumn (channel : Nat) : Nat := taggedColumn channel 2
def reservedColumn (requestTxid : Nat) : Nat := taggedColumn requestTxid 3

theorem available_does_not_alias_escrow_a (a c : Nat) :
    availableColumn a ≠ escrowAColumn c := by
  exact tagged_columns_are_disjoint a c 0 1 (by omega) (by omega) (by omega)

theorem available_does_not_alias_reserved (a tx : Nat) :
    availableColumn a ≠ reservedColumn tx := by
  exact tagged_columns_are_disjoint a tx 0 3 (by omega) (by omega) (by omega)

theorem escrow_columns_are_disjoint (a b : Nat) :
    escrowAColumn a ≠ escrowBColumn b := by
  exact tagged_columns_are_disjoint a b 1 2 (by omega) (by omega) (by omega)

theorem reserved_does_not_alias_escrow_b (tx c : Nat) :
    reservedColumn tx ≠ escrowBColumn c := by
  exact tagged_columns_are_disjoint tx c 3 2 (by omega) (by omega) (by omega)


/-!
## Newly encountered recipients

The real Rust available map omits zero-valued keys, and a transfer can create
a previously absent recipient. A finite formal support may first be extended
with a zero-valued slot without affecting conservation; the checked movement
then transfers atoms into that slot. Removal of zero entries from the
canonical Rust BTreeMap remains a separate refinement obligation.
-/

def finiteExtendZero (s : FiniteSupplyView) (key : Nat) : FiniteSupplyView :=
  { s with slots := key :: s.slots }

theorem finite_extend_zero_preserves_validity
    (s : FiniteSupplyView) (key : Nat)
    (good : FiniteSupplyValid s) (fresh : key ∉ s.slots) :
    FiniteSupplyValid (finiteExtendZero s key) := by
  rcases good with ⟨unique, covered, conserved, bounded, supplyBounded⟩
  have zero : s.atoms key = 0 := covered key fresh
  change (key :: s.slots).Nodup ∧
    (∀ other, other ∉ key :: s.slots → s.atoms other = 0) ∧
    columnTotal (key :: s.slots) s.atoms = s.supply ∧
    (∀ other, other ∈ key :: s.slots → s.atoms other ≤ maxU128) ∧
    s.supply ≤ maxU128
  refine ⟨List.nodup_cons.mpr ⟨fresh, unique⟩, ?_, ?_, ?_, supplyBounded⟩
  · intro other absent
    apply covered other
    intro member
    apply absent
    simp [member]
  · simp [columnTotal, zero, conserved]
  · intro other member
    rcases List.mem_cons.mp member with equal | oldMember
    · subst other
      simp [zero]
    · exact bounded other oldMember

theorem finite_move_to_new_recipient_preserves_validity
    (s : FiniteSupplyView) (sender recipient amount : Nat)
    (good : FiniteSupplyValid s)
    (senderPresent : sender ∈ s.slots)
    (recipientFresh : recipient ∉ s.slots)
    (positive : 0 < amount)
    (funded : amount ≤ s.atoms sender)
    (amountFits : amount ≤ maxU128) :
    FiniteSupplyValid
      (finiteMove (finiteExtendZero s recipient) sender recipient amount) := by
  have extended := finite_extend_zero_preserves_validity
    s recipient good recipientFresh
  have senderMember : sender ∈ (finiteExtendZero s recipient).slots := by
    simp [finiteExtendZero, senderPresent]
  have recipientMember : recipient ∈ (finiteExtendZero s recipient).slots := by
    simp [finiteExtendZero]
  have different : sender ≠ recipient := by
    intro same
    apply recipientFresh
    simpa [same] using senderPresent
  have zero : s.atoms recipient = 0 := good.2.1 recipient recipientFresh
  apply finite_move_preserves_validity
    (finiteExtendZero s recipient) sender recipient amount
    extended senderMember recipientMember different positive
  · exact funded
  · simpa [finiteExtendZero, zero] using amountFits


/-- Canonical snapshots omit zero-valued entries. Removing an exhausted,
zero-valued head column preserves total, coverage and u128 bounds.
A generic map erase refinement must additionally show that the removed
key was unique and zero and that all remaining keys retain their values. -/
def finiteDropHead (s : FiniteSupplyView) (remaining : List Nat) :
    FiniteSupplyView :=
  { s with slots := remaining }

theorem finite_drop_zero_head_preserves_validity
    (s : FiniteSupplyView) (key : Nat) (remaining : List Nat)
    (good : FiniteSupplyValid s)
    (support : s.slots = key :: remaining)
    (empty : s.atoms key = 0) :
    FiniteSupplyValid (finiteDropHead s remaining) := by
  rcases good with ⟨unique, covered, conserved, bounded, supplyBounded⟩
  have uniqueRest : remaining.Nodup := by
    rw [support] at unique
    exact (List.nodup_cons.mp unique).2
  change remaining.Nodup ∧
    (∀ other, other ∉ remaining → s.atoms other = 0) ∧
    columnTotal remaining s.atoms = s.supply ∧
    (∀ other, other ∈ remaining → s.atoms other ≤ maxU128) ∧
    s.supply ≤ maxU128
  refine ⟨uniqueRest, ?_, ?_, ?_, supplyBounded⟩
  · intro other missing
    by_cases same : other = key
    · subst other
      exact empty
    · apply covered other
      rw [support]
      intro inCons
      rcases List.mem_cons.mp inCons with atHead | inRest
      · exact same atHead
      · exact missing inRest
  · have total : columnTotal (key :: remaining) s.atoms = s.supply := by
      simpa [support] using conserved
    simpa [columnTotal, empty] using total
  · intro other present
    apply bounded other
    rw [support]
    simp [present]


/-!
## Arbitrary-position finite-support insertion and erasure

A Rust BTreeMap may insert or erase a zero balance at any position in its
canonical order. These theorems operate over any prefix and suffix witness
of the affected key. The amount function is unchanged while a zero-valued
slot is inserted or removed; the proven relation therefore applies to
canonical support maintenance separately from moving nonzero atoms.
-/

def finiteInsertAt (s : FiniteSupplyView) (before after : List Nat)
    (key : Nat) : FiniteSupplyView :=
  { s with slots := before ++ key :: after }

def finiteEraseAt (s : FiniteSupplyView) (before after : List Nat) :
    FiniteSupplyView :=
  { s with slots := before ++ after }

/-- A zero-valued slot makes no contribution regardless of its location. -/
theorem column_total_insert_zero_at (before after : List Nat)
    (amounts : Nat → Nat) (key : Nat) (zero : amounts key = 0) :
    columnTotal (before ++ key :: after) amounts =
      columnTotal (before ++ after) amounts := by
  induction before with
  | nil => simp [columnTotal, zero]
  | cons head tail ih =>
      simpa only [List.cons_append, columnTotal] using
        congrArg (fun n => amounts head + n) ih

/-- Removing any one element from a duplicate-free support stays
duplicate-free, independent of the original index. -/
theorem column_erase_at_nodup (before after : List Nat) (key : Nat)
    (unique : (before ++ key :: after).Nodup) :
    (before ++ after).Nodup := by
  induction before with
  | nil =>
      exact (List.nodup_cons.mp unique).2
  | cons head tail ih =>
      have old : (head :: (tail ++ key :: after)).Nodup := by
        simpa only [List.cons_append] using unique
      have absent := (List.nodup_cons.mp old).1
      have rest := (List.nodup_cons.mp old).2
      have kept : head ∉ (tail ++ after) := by
        intro member
        rcases List.mem_append.mp member with atFront | atBack
        · exact absent (List.mem_append.mpr (Or.inl atFront))
        · exact absent (List.mem_append.mpr
            (Or.inr (List.mem_cons.mpr (Or.inr atBack))))
      change (head :: (tail ++ after)).Nodup
      exact List.nodup_cons.mpr ⟨kept, ih rest⟩

/-- Inserting an absent key at any location preserves unique support. -/
theorem column_insert_at_nodup (before after : List Nat) (key : Nat)
    (unique : (before ++ after).Nodup)
    (fresh : key ∉ before ++ after) :
    (before ++ key :: after).Nodup := by
  induction before with
  | nil =>
      exact List.nodup_cons.mpr ⟨fresh, unique⟩
  | cons head tail ih =>
      have old : (head :: (tail ++ after)).Nodup := by
        simpa only [List.cons_append] using unique
      have absent := (List.nodup_cons.mp old).1
      have rest := (List.nodup_cons.mp old).2
      have freshRest : key ∉ tail ++ after := by
        intro member
        exact fresh (List.mem_cons.mpr (Or.inr member))
      have distinct : head ≠ key := by
        intro equal
        exact fresh (List.mem_cons.mpr (Or.inl equal.symm))
      have kept : head ∉ tail ++ key :: after := by
        intro member
        rcases List.mem_append.mp member with atFront | atBack
        · exact absent (List.mem_append.mpr (Or.inl atFront))
        · rcases List.mem_cons.mp atBack with equal | inBack
          · exact distinct equal
          · exact absent (List.mem_append.mpr (Or.inr inBack))
      change (head :: (tail ++ key :: after)).Nodup
      exact List.nodup_cons.mpr ⟨kept, ih rest freshRest⟩

/-- Arbitrary-position erasure of an exhausted slot preserves the entire
finite-support validity predicate. Unlike the earlier head-only lemma, the
key can occur anywhere in the recorded support. -/
theorem finite_erase_zero_anywhere_preserves_validity
    (s : FiniteSupplyView) (before after : List Nat) (key : Nat)
    (valid : FiniteSupplyValid s)
    (support : s.slots = before ++ key :: after)
    (zero : s.atoms key = 0) :
    FiniteSupplyValid (finiteEraseAt s before after) := by
  rcases valid with ⟨unique, covered, conserved, bounded, supplyBounded⟩
  have priorNodup : (before ++ key :: after).Nodup := by
    simpa [support] using unique
  have keptNodup := column_erase_at_nodup before after key priorNodup
  change (before ++ after).Nodup ∧
    (∀ other, other ∉ before ++ after → s.atoms other = 0) ∧
    columnTotal (before ++ after) s.atoms = s.supply ∧
    (∀ other, other ∈ before ++ after → s.atoms other ≤ maxU128) ∧
    s.supply ≤ maxU128
  refine ⟨keptNodup, ?_, ?_, ?_, supplyBounded⟩
  · intro other missing
    by_cases equal : other = key
    · subst other
      exact zero
    · apply covered other
      rw [support]
      intro member
      rcases List.mem_append.mp member with inFront | inTail
      · exact missing (List.mem_append.mpr (Or.inl inFront))
      · rcases List.mem_cons.mp inTail with atKey | inBack
        · exact equal atKey
        · exact missing (List.mem_append.mpr (Or.inr inBack))
  · have originalTotal :
        columnTotal (before ++ key :: after) s.atoms = s.supply := by
      simpa [support] using conserved
    rw [column_total_insert_zero_at before after s.atoms key zero] at originalTotal
    exact originalTotal
  · intro other present
    apply bounded other
    rw [support]
    rcases List.mem_append.mp present with inFront | inBack
    · exact List.mem_append.mpr (Or.inl inFront)
    · exact List.mem_append.mpr
        (Or.inr (List.mem_cons.mpr (Or.inr inBack)))

/-- A previously absent zero-valued recipient may enter the finite support
at any insertion index while retaining the original total and column bounds. -/
theorem finite_insert_zero_anywhere_preserves_validity
    (s : FiniteSupplyView) (before after : List Nat) (key : Nat)
    (valid : FiniteSupplyValid s)
    (support : s.slots = before ++ after)
    (fresh : key ∉ s.slots) :
    FiniteSupplyValid (finiteInsertAt s before after key) := by
  rcases valid with ⟨unique, covered, conserved, bounded, supplyBounded⟩
  have oldNodup : (before ++ after).Nodup := by
    simpa [support] using unique
  have freshSupport : key ∉ before ++ after := by
    simpa [support] using fresh
  have zero : s.atoms key = 0 := covered key fresh
  have newNodup := column_insert_at_nodup
    before after key oldNodup freshSupport
  change (before ++ key :: after).Nodup ∧
    (∀ other, other ∉ before ++ key :: after → s.atoms other = 0) ∧
    columnTotal (before ++ key :: after) s.atoms = s.supply ∧
    (∀ other, other ∈ before ++ key :: after → s.atoms other ≤ maxU128) ∧
    s.supply ≤ maxU128
  refine ⟨newNodup, ?_, ?_, ?_, supplyBounded⟩
  · intro other missing
    apply covered other
    rw [support]
    intro member
    rcases List.mem_append.mp member with inFront | inBack
    · exact missing (List.mem_append.mpr (Or.inl inFront))
    · exact missing (List.mem_append.mpr
        (Or.inr (List.mem_cons.mpr (Or.inr inBack))))
  · rw [column_total_insert_zero_at before after s.atoms key zero]
    simpa [support] using conserved
  · intro other present
    rcases List.mem_append.mp present with inFront | inTail
    · apply bounded other
      rw [support]
      exact List.mem_append.mpr (Or.inl inFront)
    · rcases List.mem_cons.mp inTail with atKey | inBack
      · subst other
        simp [zero]
      · apply bounded other
        rw [support]
        exact List.mem_append.mpr (Or.inr inBack)


/-!
## Support reordering is semantically inert

A canonical BTreeMap enumerates keys in sorted order, while a finite-support
model can enumerate them in any order. List.Perm is an explicit ordering
certificate. These theorems show that all aggregate conservation obligations
commute with any such finite rearrangement, under the existing unique-key
and adequate-balance premises.
-/

theorem column_total_permutation
    {keys reordered : List Nat} (amounts : Nat → Nat)
    (permutation : keys.Perm reordered) :
    columnTotal keys amounts = columnTotal reordered amounts := by
  induction permutation with
  | nil => rfl
  | cons key earlier ih =>
      simpa only [columnTotal] using
        congrArg (fun n => amounts key + n) ih
  | swap first second rest =>
      simp only [columnTotal]
      omega
  | trans first second ihFirst ihSecond =>
      exact ihFirst.trans ihSecond

theorem column_move_conserves_under_reordering
    (keys reordered : List Nat) (amounts : Nat → Nat)
    (debitKey creditKey amount : Nat)
    (permutation : keys.Perm reordered)
    (unique : keys.Nodup)
    (debitPresent : debitKey ∈ keys)
    (creditPresent : creditKey ∈ keys)
    (distinct : debitKey ≠ creditKey)
    (funded : amount ≤ amounts debitKey) :
    columnTotal reordered
      (columnMove amounts debitKey creditKey amount) =
    columnTotal reordered amounts := by
  calc
    columnTotal reordered
        (columnMove amounts debitKey creditKey amount) =
      columnTotal keys (columnMove amounts debitKey creditKey amount) :=
      (column_total_permutation _ permutation).symm
    _ = columnTotal keys amounts :=
      column_move_preserves_total keys amounts
        debitKey creditKey amount unique debitPresent
        creditPresent distinct funded
    _ = columnTotal reordered amounts :=
      column_total_permutation amounts permutation


/-!
## Stage 7: exact typed-key identity shared with Rust supply projections

Rust exports `SupplyColumnProofKey` as a FOUR-FIELD tuple containing a
declared network tag (1/2/3), nonzero local property ID, source tag
(0/1/2/3), and verbatim identifier bytes (ASCII account/channel or 32-byte
TxId). The formal key records those same data fields as Nat and List Nat.

The following left inverse proves injectivity without assuming collision
resistance or inventing a lossy digest. A separate correspondence obligation
remains for Rust's string-to-byte representation, property bounds, and the
caller-supplied network's actual chain provenance.
-/

inductive ColumnOrigin where
  | available (bytes : List Nat)
  | escrowA (bytes : List Nat)
  | escrowB (bytes : List Nat)
  | reserved (bytes : List Nat)
  deriving Repr, DecidableEq

structure TypedColumnKey where
  network : Mainnet
  property : Nat
  origin : ColumnOrigin
  deriving Repr, DecidableEq

structure EncodedColumnKey where
  networkTag : Nat
  propertyId : Nat
  sourceTag : Nat
  identityBytes : List Nat
  deriving Repr, DecidableEq

def encodeNetwork : Mainnet → Nat
  | .bitcoin => 1
  | .litecoin => 2
  | .dogecoin => 3

def decodeNetwork : Nat → Option Mainnet
  | 1 => some .bitcoin
  | 2 => some .litecoin
  | 3 => some .dogecoin
  | _ => none

def encodeOrigin : ColumnOrigin → Nat × List Nat
  | .available bytes => (0, bytes)
  | .escrowA bytes => (1, bytes)
  | .escrowB bytes => (2, bytes)
  | .reserved bytes => (3, bytes)

def decodeOrigin (tag : Nat) (bytes : List Nat) : Option ColumnOrigin :=
  match tag with
  | 0 => some (.available bytes)
  | 1 => some (.escrowA bytes)
  | 2 => some (.escrowB bytes)
  | 3 => some (.reserved bytes)
  | _ => none

def encodeTypedColumn (key : TypedColumnKey) : EncodedColumnKey :=
  let origin := encodeOrigin key.origin
  { networkTag := encodeNetwork key.network
    propertyId := key.property
    sourceTag := origin.1
    identityBytes := origin.2 }

def decodeTypedColumn (key : EncodedColumnKey) : Option TypedColumnKey := do
  let network ← decodeNetwork key.networkTag
  let origin ← decodeOrigin key.sourceTag key.identityBytes
  pure { network := network, property := key.propertyId, origin := origin }

/-- Reconstruct the exact typed network, property and source bytes. -/
theorem decode_encode_typed_column (key : TypedColumnKey) :
    decodeTypedColumn (encodeTypedColumn key) = some key := by
  cases key with
  | mk network property origin =>
    cases network <;> cases origin <;> rfl

/-- Exact structured key encoding is injective, for ALL byte sequences. -/
theorem encode_typed_column_injective :
    Function.Injective encodeTypedColumn := by
  intro a b same
  have decoded := congrArg decodeTypedColumn same
  rw [decode_encode_typed_column, decode_encode_typed_column] at decoded
  exact Option.some.inj decoded

theorem encoded_typed_columns_separate_networks
    (a b : TypedColumnKey) (different : a.network ≠ b.network) :
    encodeTypedColumn a ≠ encodeTypedColumn b := by
  intro equal
  exact different (congrArg TypedColumnKey.network
    (encode_typed_column_injective equal))

theorem encoded_typed_columns_separate_properties
    (a b : TypedColumnKey) (different : a.property ≠ b.property) :
    encodeTypedColumn a ≠ encodeTypedColumn b := by
  intro equal
  exact different (congrArg TypedColumnKey.property
    (encode_typed_column_injective equal))

theorem encoded_typed_columns_separate_sources
    (a b : TypedColumnKey) (different : a.origin ≠ b.origin) :
    encodeTypedColumn a ≠ encodeTypedColumn b := by
  intro equal
  exact different (congrArg TypedColumnKey.origin
    (encode_typed_column_injective equal))

/-- General finite-support accounting total over typed keys. -/
def typedColumnTotal {α : Type} : List α → (α → Nat) → Nat
  | [], _ => 0
  | key :: rest, amounts =>
      amounts key + typedColumnTotal rest amounts

def decodedColumnAmounts
    (amounts : TypedColumnKey → Nat) (key : EncodedColumnKey) : Nat :=
  match decodeTypedColumn key with
  | some typed => amounts typed
  | none => 0

/-- Relabeling concrete typed support by the exact encoded-key structure
cannot lose or double-count a single atom. Works for arbitrary finite lists. -/
theorem typed_column_total_preserved_by_encoding
    (support : List TypedColumnKey) (amounts : TypedColumnKey → Nat) :
    typedColumnTotal (support.map encodeTypedColumn)
      (decodedColumnAmounts amounts) =
    typedColumnTotal support amounts := by
  induction support with
  | nil => rfl
  | cons key tail ih =>
      simpa [typedColumnTotal, decodedColumnAmounts,
        decode_encode_typed_column] using
        congrArg (fun n => amounts key + n) ih

/-- A concrete example: the ASCII bytes for 'a', property 7, on
Bitcoin's local TradeLayer remain visible without hashing or truncation. -/
example :
    encodeTypedColumn
        { network := .bitcoin, property := 7,
          origin := .available [97] } =
      { networkTag := 1, propertyId := 7,
        sourceTag := 0, identityBytes := [97] } := by
  rfl

/-- All source tags remain distinct even with byte-identical identifiers. -/
theorem source_tag_noncollision
    (network : Mainnet) (property : Nat) (bytes : List Nat) :
    encodeTypedColumn
        { network := network, property := property, origin := .available bytes } ≠
      encodeTypedColumn
        { network := network, property := property, origin := .reserved bytes } := by
  intro equal
  have contradiction := congrArg EncodedColumnKey.sourceTag equal
  simp [encodeTypedColumn, encodeOrigin] at contradiction


/-!
## Typed transfer conservation under exact key encoding

This final bridge lifts the existing column-sum reasoning to the exact
typed keys represented by the Rust audit. Since the encoded key has a
machine-checked left inverse, the same conservation equation holds when
its support is re-keyed to (network tag, property, source tag, bytes).
-/

def typedColumnPut (amounts : TypedColumnKey → Nat)
    (key : TypedColumnKey) (newValue : Nat) : TypedColumnKey → Nat :=
  fun query => if query = key then newValue else amounts query

theorem typed_column_total_put_absent
    (support : List TypedColumnKey) (amounts : TypedColumnKey → Nat)
    (key : TypedColumnKey) (newValue : Nat)
    (absent : key ∉ support) :
    typedColumnTotal support (typedColumnPut amounts key newValue) =
      typedColumnTotal support amounts := by
  induction support with
  | nil => rfl
  | cons head tail ih =>
      have different : head ≠ key := by
        intro equal
        apply absent
        simp [equal]
      have absentTail : key ∉ tail := by
        intro member
        apply absent
        exact List.mem_cons.mpr (Or.inr member)
      simp [typedColumnTotal, typedColumnPut, different, ih absentTail]

theorem typed_column_total_put
    (support : List TypedColumnKey) (amounts : TypedColumnKey → Nat)
    (key : TypedColumnKey) (newValue : Nat)
    (unique : support.Nodup) (present : key ∈ support) :
    typedColumnTotal support (typedColumnPut amounts key newValue) + amounts key =
      typedColumnTotal support amounts + newValue := by
  induction support with
  | nil => simp at present
  | cons head tail ih =>
      have noTail : head ∉ tail := (List.nodup_cons.mp unique).1
      have uniqueTail : tail.Nodup := (List.nodup_cons.mp unique).2
      by_cases same : head = key
      · subst head
        have tailUnchanged :=
          typed_column_total_put_absent tail amounts key newValue noTail
        simp [typedColumnTotal, typedColumnPut, tailUnchanged]
        omega
      · have inTail : key ∈ tail := by
          have disjunction := List.mem_cons.mp present
          exact disjunction.resolve_left (by simpa [eq_comm] using same)
        have restEq := ih uniqueTail inTail
        simp only [typedColumnTotal]
        simp [typedColumnPut, same]
        omega

def typedColumnMove (amounts : TypedColumnKey → Nat)
    (sender recipient : TypedColumnKey) (amount : Nat) :
    TypedColumnKey → Nat :=
  typedColumnPut
    (typedColumnPut amounts sender (amounts sender - amount))
    recipient (amounts recipient + amount)

theorem typed_column_move_preserves_total
    (support : List TypedColumnKey) (amounts : TypedColumnKey → Nat)
    (sender recipient : TypedColumnKey) (amount : Nat)
    (unique : support.Nodup)
    (senderPresent : sender ∈ support)
    (recipientPresent : recipient ∈ support)
    (distinct : sender ≠ recipient)
    (funded : amount ≤ amounts sender) :
    typedColumnTotal support (typedColumnMove amounts sender recipient amount) =
      typedColumnTotal support amounts := by
  let intermediate := typedColumnPut amounts sender (amounts sender - amount)
  have first := typed_column_total_put support amounts sender
    (amounts sender - amount) unique senderPresent
  have second := typed_column_total_put support intermediate recipient
    (amounts recipient + amount) unique recipientPresent
  have untouched : intermediate recipient = amounts recipient := by
    simp [intermediate, typedColumnPut, Ne.symm distinct]
  change typedColumnTotal support intermediate + amounts sender =
    typedColumnTotal support amounts + (amounts sender - amount) at first
  change typedColumnTotal support (typedColumnMove amounts sender recipient amount) +
    intermediate recipient =
    typedColumnTotal support intermediate + (amounts recipient + amount) at second
  rw [untouched] at second
  omega

/-- Conservation transfers through the full four-field, network-qualified
encoding for arbitrary finite typed supports and two distinct columns. -/
theorem encoded_column_move_preserves_total
    (support : List TypedColumnKey) (amounts : TypedColumnKey → Nat)
    (sender recipient : TypedColumnKey) (amount : Nat)
    (unique : support.Nodup)
    (senderPresent : sender ∈ support)
    (recipientPresent : recipient ∈ support)
    (distinct : sender ≠ recipient)
    (funded : amount ≤ amounts sender) :
    typedColumnTotal (support.map encodeTypedColumn)
        (decodedColumnAmounts (typedColumnMove amounts sender recipient amount)) =
      typedColumnTotal (support.map encodeTypedColumn)
        (decodedColumnAmounts amounts) := by
  rw [typed_column_total_preserved_by_encoding,
      typed_column_total_preserved_by_encoding]
  exact typed_column_move_preserves_total support amounts sender recipient
    amount unique senderPresent recipientPresent distinct funded


/-!
## Stage 8: a key/value association-map refinement contract

These definitions model the *observable* pointwise behavior of a map of
network-qualified typed keys to atomic amounts. Unlike Rust's BTreeMap, the
simple list implementation is independent of tree balancing, allocation and
iteration order. The lemmas prove a left-to-right abstraction relation for
arbitrary finite lists (including malformed duplicates); they are NOT a proof
of std::BTreeMap internals or full Ledger::execute correctness.
-/

/-- Return zero for absent entries, like Ledger::available_atoms. -/
def assocColumnGet : List (EncodedColumnKey × Nat) →
    EncodedColumnKey → Nat
  | [], _ => 0
  | (stored, atoms) :: rest, key =>
      if stored = key then atoms else assocColumnGet rest key

/-- Erase ALL matching keys, including duplicates in malformed input. -/
def assocColumnErase : List (EncodedColumnKey × Nat) →
    EncodedColumnKey → List (EncodedColumnKey × Nat)
  | [], _ => []
  | (stored, atoms) :: rest, key =>
      if stored = key then assocColumnErase rest key
      else (stored, atoms) :: assocColumnErase rest key

/-- The production nonzero rule: zero removes, positive replaces or inserts.
This uses a front insertion; sorting the output is a separate map contract. -/
def assocColumnPut (rows : List (EncodedColumnKey × Nat))
    (key : EncodedColumnKey) (atoms : Nat) :
    List (EncodedColumnKey × Nat) :=
  if atoms = 0 then assocColumnErase rows key
  else (key, atoms) :: assocColumnErase rows key

theorem assoc_erase_get_self
    (rows : List (EncodedColumnKey × Nat)) (key : EncodedColumnKey) :
    assocColumnGet (assocColumnErase rows key) key = 0 := by
  induction rows with
  | nil => rfl
  | cons pair rest ih =>
      rcases pair with ⟨stored, atoms⟩
      by_cases equal : stored = key
      · simp [assocColumnErase, assocColumnGet, equal, ih]
      · simp [assocColumnErase, assocColumnGet, equal, ih]

theorem assoc_erase_get_other
    (rows : List (EncodedColumnKey × Nat))
    (erased query : EncodedColumnKey) (different : query ≠ erased) :
    assocColumnGet (assocColumnErase rows erased) query =
      assocColumnGet rows query := by
  induction rows with
  | nil => rfl
  | cons pair rest ih =>
      rcases pair with ⟨stored, atoms⟩
      by_cases equal : stored = erased
      · subst stored
        simp [assocColumnErase, assocColumnGet, Ne.symm different, ih]
      · simp [assocColumnErase, assocColumnGet, equal, ih]

theorem assoc_put_get_self
    (rows : List (EncodedColumnKey × Nat))
    (key : EncodedColumnKey) (atoms : Nat) :
    assocColumnGet (assocColumnPut rows key atoms) key = atoms := by
  by_cases zero : atoms = 0
  · subst atoms
    simp [assocColumnPut, assoc_erase_get_self]
  · simp [assocColumnPut, zero, assocColumnGet]

theorem assoc_put_get_other
    (rows : List (EncodedColumnKey × Nat))
    (key other : EncodedColumnKey) (atoms : Nat)
    (different : other ≠ key) :
    assocColumnGet (assocColumnPut rows key atoms) other =
      assocColumnGet rows other := by
  by_cases zero : atoms = 0
  · simp [assocColumnPut, zero,
      assoc_erase_get_other rows key other different]
  · simp [assocColumnPut, zero, assocColumnGet,
      Ne.symm different, assoc_erase_get_other rows key other different]

/-- A map refinement is pointwise equality of all stored amounts.
No ordering hypothesis is needed because key behavior is extensional. -/
def AssocColumnRepresents
    (rows : List (EncodedColumnKey × Nat))
    (amounts : EncodedColumnKey → Nat) : Prop :=
  ∀ key, assocColumnGet rows key = amounts key

/-- The association-map implementation refines functional assignment at
ANY typed key, for arbitrary old maps and inserted/removed amounts. -/
theorem assoc_put_refines_function
    (rows : List (EncodedColumnKey × Nat))
    (amounts : EncodedColumnKey → Nat)
    (key : EncodedColumnKey) (atoms : Nat)
    (good : AssocColumnRepresents rows amounts) :
    AssocColumnRepresents (assocColumnPut rows key atoms)
        (fun query => if query = key then atoms else amounts query) := by
  intro query
  by_cases same : query = key
  · subst query
    simpa [AssocColumnRepresents] using
      assoc_put_get_self rows key atoms
  · have other := assoc_put_get_other rows key query atoms same
    simpa [AssocColumnRepresents, same, good query] using other

/-- Transaction-style two-column update, identical to a sequence of two
canonical map writes after funding/overflow validation. -/
def assocColumnMove (rows : List (EncodedColumnKey × Nat))
    (sender receiver : EncodedColumnKey) (atoms : Nat) :
    List (EncodedColumnKey × Nat) :=
  let debit := assocColumnGet rows sender
  let credit := assocColumnGet rows receiver
  let first := assocColumnPut rows sender (debit - atoms)
  assocColumnPut first receiver (credit + atoms)

/-- Proof that the two-column update has exact source, destination and frame
semantics; note the sender and receiver must be distinct. -/
theorem assoc_move_refines_function
    (rows : List (EncodedColumnKey × Nat))
    (amounts : EncodedColumnKey → Nat)
    (sender receiver : EncodedColumnKey) (atoms : Nat)
    (good : AssocColumnRepresents rows amounts)
    (different : sender ≠ receiver) :
    AssocColumnRepresents (assocColumnMove rows sender receiver atoms)
      (fun query =>
        if query = sender then amounts sender - atoms
        else if query = receiver then amounts receiver + atoms
        else amounts query) := by
  have debit : assocColumnGet rows sender = amounts sender := good sender
  have credit : assocColumnGet rows receiver = amounts receiver := good receiver
  unfold assocColumnMove
  rw [debit, credit]
  have first := assoc_put_refines_function rows amounts sender
    (amounts sender - atoms) good
  have second := assoc_put_refines_function
    (assocColumnPut rows sender (amounts sender - atoms))
    (fun query => if query = sender then amounts sender - atoms else amounts query)
    receiver (amounts receiver + atoms) first
  intro query
  have atQuery := second query
  by_cases isSender : query = sender
  · subst query
    simpa [different, Ne.symm different] using atQuery
  · by_cases isReceiver : query = receiver
    · subst query
      simpa [isSender, Ne.symm different] using atQuery
    · simpa [isSender, isReceiver] using atQuery


/-!
## Stage 9: sorted vector lookup and canonical update semantics

The production `OrderedColumns<K>` stores strictly increasing keys in a
Vec and returns zero for missing balance columns. The following Nat-key
model gives the ordered search/update semantics independently of allocator
behavior. Rust's `Ord` comparison and library Vec/binary-search behavior
remain explicit implementation trust boundaries.
-/

def sortedVecGet : List (Nat × Nat) → Nat → Nat
  | [], _ => 0
  | (stored, atoms) :: rest, key =>
      if key < stored then 0
      else if key = stored then atoms else sortedVecGet rest key

def sortedVecPut : List (Nat × Nat) → Nat → Nat → List (Nat × Nat)
  | [], key, amount =>
      if amount = 0 then [] else [(key, amount)]
  | (stored, atoms) :: rest, key, amount =>
      if key < stored then
        if amount = 0 then (stored, atoms) :: rest
        else (key, amount) :: (stored, atoms) :: rest
      else if key = stored then
        if amount = 0 then rest
        else (stored, amount) :: rest
      else
        (stored, atoms) :: sortedVecPut rest key amount

def SortedVecKeys : List (Nat × Nat) → Prop
  | [] => True
  | (stored, _) :: rest =>
      (∀ entry ∈ rest, stored < entry.1) ∧ SortedVecKeys rest

theorem sorted_vec_read_below (rows : List (Nat × Nat))
    (key lower : Nat)
    (greater : ∀ entry ∈ rows, lower < entry.1)
    (below : key ≤ lower) :
    sortedVecGet rows key = 0 := by
  cases rows with
  | nil => rfl
  | cons pair rest =>
      rcases pair with ⟨stored, atoms⟩
      have gt : lower < stored :=
        greater (stored, atoms) (by simp)
      have lt : key < stored := by omega
      simp [sortedVecGet, lt]

/-- The canonical zero-delete / positive-insert operation places or removes
the selected key at its ordered position, even when that position is not the
head of the vector. -/
theorem sorted_vec_put_reads_written_key (rows : List (Nat × Nat))
    (key amount : Nat) (sorted : SortedVecKeys rows) :
    sortedVecGet (sortedVecPut rows key amount) key = amount := by
  induction rows with
  | nil =>
      by_cases zero : amount = 0 <;>
        simp [sortedVecPut, sortedVecGet, zero]
  | cons pair rest ih =>
      rcases pair with ⟨stored, prior⟩
      obtain ⟨greater, restSorted⟩ := sorted
      by_cases lt : key < stored
      · by_cases zero : amount = 0
        · simp [sortedVecPut, sortedVecGet, lt, zero]
        · simp [sortedVecPut, sortedVecGet, lt, zero]
      · by_cases equal : key = stored
        · subst key
          by_cases zero : amount = 0
          · have restZero :
                sortedVecGet rest stored = 0 :=
              sorted_vec_read_below rest stored stored greater (by omega)
            simp [sortedVecPut, sortedVecGet, lt, zero, restZero]
          · simp [sortedVecPut, sortedVecGet, lt, zero]
        · have restResult := ih restSorted
          simpa [sortedVecPut, sortedVecGet, lt, equal] using restResult


/-- A canonical sorted-vector write does not modify the observable balance
at any distinct key. Together with `sorted_vec_put_reads_written_key`,
this is an extensional get/put specification for the new balance backend. -/
theorem sorted_vec_put_keeps_other_keys
    (rows : List (Nat × Nat)) (key other amount : Nat)
    (sorted : SortedVecKeys rows) (different : other ≠ key) :
    sortedVecGet (sortedVecPut rows key amount) other =
      sortedVecGet rows other := by
  induction rows with
  | nil =>
      by_cases zero : amount = 0
      · simp [sortedVecPut, sortedVecGet, zero]
      · by_cases below : other < key
        · simp [sortedVecPut, sortedVecGet, zero, below]
        · simp [sortedVecPut, sortedVecGet, zero, below, different]
  | cons pair rest ih =>
      rcases pair with ⟨stored, prior⟩
      obtain ⟨greater, restSorted⟩ := sorted
      by_cases lt : key < stored
      · by_cases zero : amount = 0
        · simp [sortedVecPut, zero, lt]
        · by_cases below : other < key
          · have oldBelow : other < stored := by omega
            simp [sortedVecPut, sortedVecGet, lt, zero, below, oldBelow]
          · simp [sortedVecPut, sortedVecGet, lt, zero, below, different]
      · by_cases equal : key = stored
        · subst key
          by_cases zero : amount = 0
          · by_cases below : other < stored
            · have tailZero :=
                sorted_vec_read_below rest other stored greater (by omega)
              simp [sortedVecPut, sortedVecGet, lt, zero, below,
                different, tailZero]
            · simp [sortedVecPut, sortedVecGet, lt, zero, below, different]
          · simp [sortedVecPut, sortedVecGet, lt, zero, different]
        · have tailFrame := ih restSorted
          simp [sortedVecPut, sortedVecGet, lt, equal, tailFrame]


/-- Inserting or removing one key cannot introduce a key below a declared
lower bound when both the old support and inserted key exceed the bound. -/
theorem sorted_vec_write_preserves_lower_bound
    (rows : List (Nat × Nat)) (lower key amount : Nat)
    (greater : ∀ entry ∈ rows, lower < entry.1)
    (newAbove : lower < key) :
    ∀ entry ∈ sortedVecPut rows key amount, lower < entry.1 := by
  induction rows with
  | nil =>
      intro entry member
      by_cases zero : amount = 0
      · simp [sortedVecPut, zero] at member
      · have equal : entry = (key, amount) := by
          simpa [sortedVecPut, zero] using member
        subst entry
        exact newAbove
  | cons pair rest ih =>
      rcases pair with ⟨stored, prior⟩
      have greaterTail : ∀ entry ∈ rest, lower < entry.1 := by
        intro entry inRest
        exact greater entry (List.mem_cons.mpr (Or.inr inRest))
      have oldAbove : lower < stored :=
        greater (stored, prior) (by simp)
      intro entry member
      by_cases before : key < stored
      · by_cases zero : amount = 0
        · have oldMember : entry ∈ (stored, prior) :: rest := by
            simpa [sortedVecPut, before, zero] using member
          exact greater entry oldMember
        · have choice :
            entry = (key, amount) ∨ entry ∈ (stored, prior) :: rest := by
            simpa [sortedVecPut, before, zero] using member
          rcases choice with isNew | isOld
          · subst entry
            exact newAbove
          · exact greater entry isOld
      · by_cases equal : key = stored
        · by_cases zero : amount = 0
          · have inRest : entry ∈ rest := by
              simpa [sortedVecPut, before, equal, zero] using member
            exact greaterTail entry inRest
          · have choice :
                entry = (stored, amount) ∨ entry ∈ rest := by
              simpa [sortedVecPut, before, equal, zero] using member
            rcases choice with atHead | atRest
            · subst entry
              exact oldAbove
            · exact greaterTail entry atRest
        · have choice :
              entry = (stored, prior) ∨
                entry ∈ sortedVecPut rest key amount := by
            simpa [sortedVecPut, before, equal] using member
          rcases choice with atHead | atRest
          · subst entry
            exact oldAbove
          · exact ih greaterTail entry atRest

/-- For any number of entries, sorted insertion, replacement and zero-erasure
preserve strict ascending order. This is the ordered Vec representation
invariant, not merely a balance-conservation statement. -/
theorem sorted_vec_write_preserves_order
    (rows : List (Nat × Nat)) (key amount : Nat)
    (sorted : SortedVecKeys rows) :
    SortedVecKeys (sortedVecPut rows key amount) := by
  induction rows with
  | nil =>
      by_cases zero : amount = 0 <;>
        simp [sortedVecPut, SortedVecKeys, zero]
  | cons pair rest ih =>
      rcases pair with ⟨stored, prior⟩
      obtain ⟨greater, restSorted⟩ := sorted
      by_cases before : key < stored
      · by_cases zero : amount = 0
        · simpa [sortedVecPut, before, zero] using
            (show SortedVecKeys ((stored, prior) :: rest) from
              ⟨greater, restSorted⟩)
        · have newGreater :
              ∀ entry ∈ (stored, prior) :: rest, key < entry.1 := by
            intro entry member
            rcases List.mem_cons.mp member with isHead | inTail
            · subst entry
              exact before
            · have oldAbove := greater entry inTail
              omega
          simpa [sortedVecPut, before, zero, SortedVecKeys] using
            (show SortedVecKeys ((key, amount) :: (stored, prior) :: rest)
              from ⟨newGreater, ⟨greater, restSorted⟩⟩)
      · by_cases equal : key = stored
        · by_cases zero : amount = 0
          · simpa [sortedVecPut, before, equal, zero] using restSorted
          · simpa [sortedVecPut, before, equal, zero, SortedVecKeys] using
              (show SortedVecKeys ((stored, amount) :: rest)
                from ⟨greater, restSorted⟩)
        · have above : stored < key := by omega
          have futureGreater :
              ∀ entry ∈ sortedVecPut rest key amount,
                stored < entry.1 :=
            sorted_vec_write_preserves_lower_bound rest stored key amount
              greater above
          have tailSorted := ih restSorted
          simpa [sortedVecPut, before, equal, SortedVecKeys] using
            (show SortedVecKeys
                ((stored, prior) :: sortedVecPut rest key amount)
              from ⟨futureGreater, tailSorted⟩)


/-- Sequential debit and credit using the actual sorted-column update
semantics. This is the abstract refinement target for Rust's two calls
to the canonical wallet/escrow balance writer. -/
def sortedVecMove (rows : List (Nat × Nat))
    (sender receiver amount : Nat) : List (Nat × Nat) :=
  let debit := sortedVecGet rows sender
  let credit := sortedVecGet rows receiver
  let intermediate := sortedVecPut rows sender (debit - amount)
  sortedVecPut intermediate receiver (credit + amount)

/-- Moving amounts between distinct keys has the exact pairwise accounting
semantics, and every other balance column is untouched. This theorem is
for any finite sorted list, not only a two-slot Kani fixture. -/
theorem sorted_vec_move_refines_two_column_function
    (rows : List (Nat × Nat))
    (sender receiver amount : Nat)
    (sorted : SortedVecKeys rows)
    (distinct : sender ≠ receiver) :
    ∀ query,
      sortedVecGet (sortedVecMove rows sender receiver amount) query =
        if query = sender then sortedVecGet rows sender - amount
        else if query = receiver then sortedVecGet rows receiver + amount
        else sortedVecGet rows query := by
  let debit := sortedVecGet rows sender
  let credit := sortedVecGet rows receiver
  let mid := sortedVecPut rows sender (debit - amount)
  have midSorted : SortedVecKeys mid :=
    sorted_vec_write_preserves_order rows sender (debit - amount) sorted
  have senderAfter : sortedVecGet mid sender = debit - amount :=
    sorted_vec_put_reads_written_key rows sender (debit - amount) sorted
  have receiverAfter :
      sortedVecGet (sortedVecPut mid receiver (credit + amount)) receiver =
        credit + amount :=
    sorted_vec_put_reads_written_key mid receiver (credit + amount) midSorted
  intro query
  by_cases isSender : query = sender
  · subst query
    have framed := sorted_vec_put_keeps_other_keys
      mid receiver sender (credit + amount) midSorted distinct
    simpa [sortedVecMove, mid, debit, credit] using
      (framed.trans senderAfter)
  · by_cases isReceiver : query = receiver
    · subst query
      simpa [sortedVecMove, mid, debit, credit, isSender]
        using receiverAfter
    · have secondFrame := sorted_vec_put_keeps_other_keys
        mid receiver query (credit + amount) midSorted isReceiver
      have firstFrame := sorted_vec_put_keeps_other_keys
        rows sender query (debit - amount) sorted isSender
      simpa [sortedVecMove, mid, debit, credit, isSender, isReceiver]
        using secondFrame.trans firstFrame


/-!
## Stage 10: arbitrary-length sorted-column refinement through entire histories

The production ordered-column backend searches a strictly sorted Vec. This
model compares its early-stop lookup against an independent linear scan and
then shows that any finite sequence of inserts, replacements or zero-erases
refines a sequence of functional writes. The proof is not a verification of
Rust slice indexing, binary_search_by, the allocator or native Ord.
-/

/-- Independent linear lookup that never stops early because of ordering. -/
def linearColumnGet : List (Nat × Nat) → Nat → Nat
  | [], _ => 0
  | (stored, value) :: tail, query =>
      if stored = query then value else linearColumnGet tail query

theorem linear_column_get_below
    (rows : List (Nat × Nat)) (query lower : Nat)
    (above : ∀ entry ∈ rows, lower < entry.1)
    (below : query ≤ lower) :
    linearColumnGet rows query = 0 := by
  induction rows with
  | nil => rfl
  | cons pair tail ih =>
      rcases pair with ⟨stored, value⟩
      have headAbove : lower < stored :=
        above (stored, value) (by simp)
      have different : stored ≠ query := by omega
      have tailAbove : ∀ entry ∈ tail, lower < entry.1 := by
        intro entry member
        exact above entry (List.mem_cons.mpr (Or.inr member))
      simpa [linearColumnGet, different] using ih tailAbove

/-- A sorted early-stopping lookup is identical to an unconditional linear
lookup on every key, for an arbitrary finite strictly ordered support. -/
theorem sorted_vec_get_equals_linear
    (rows : List (Nat × Nat)) (query : Nat)
    (sorted : SortedVecKeys rows) :
    sortedVecGet rows query = linearColumnGet rows query := by
  induction rows with
  | nil => rfl
  | cons pair tail ih =>
      rcases pair with ⟨stored, value⟩
      obtain ⟨above, tailSorted⟩ := sorted
      by_cases before : query < stored
      · have different : stored ≠ query := by omega
        have tailAbove : ∀ entry ∈ tail, query < entry.1 := by
          intro entry member
          have larger := above entry member
          omega
        have emptyTail :=
          linear_column_get_below tail query query tailAbove (by omega)
        simp [sortedVecGet, linearColumnGet, before, different, emptyTail]
      · by_cases same : query = stored
        · subst query
          simp [sortedVecGet, linearColumnGet, before]
        · have different : stored ≠ query := Ne.symm same
          simpa [sortedVecGet, linearColumnGet, before, same, different]
            using ih tailSorted

/-- A pure functional balance map with an explicit sorted-vector
representation invariant. Missing keys always correspond to zero. -/
def SortedColumnRepresents (rows : List (Nat × Nat))
    (amounts : Nat → Nat) : Prop :=
  SortedVecKeys rows ∧
    ∀ query, sortedVecGet rows query = amounts query

theorem sorted_vec_write_refines_function
    (rows : List (Nat × Nat)) (amounts : Nat → Nat)
    (key value : Nat)
    (represented : SortedColumnRepresents rows amounts) :
    SortedColumnRepresents (sortedVecPut rows key value)
      (fun query => if query = key then value else amounts query) := by
  rcases represented with ⟨sorted, equal⟩
  refine ⟨sorted_vec_write_preserves_order rows key value sorted, ?_⟩
  intro query
  by_cases same : query = key
  · subst query
    simpa using sorted_vec_put_reads_written_key rows key value sorted
  · have framed := sorted_vec_put_keeps_other_keys
      rows key query value sorted same
    simpa [same, equal query] using framed

/-- Sequential canonical writes, with zero as deletion. -/
def sortedVecWriteTrace (rows : List (Nat × Nat)) :
    List (Nat × Nat) → List (Nat × Nat)
  | [] => rows
  | (key, value) :: tail =>
      sortedVecWriteTrace (sortedVecPut rows key value) tail

/-- The mathematical function updated in precisely the same order. -/
def functionalWriteTrace (amounts : Nat → Nat) :
    List (Nat × Nat) → Nat → Nat
  | [] => amounts
  | (key, value) :: tail =>
      functionalWriteTrace
        (fun query => if query = key then value else amounts query) tail

/-- Any finite sequence of canonical vector writes preserves strict key
ordering and matches pointwise assignment in the abstract map. No bound on
history length, key count or insertion/deletion position is assumed. -/
theorem sorted_vec_write_trace_refines
    (writes : List (Nat × Nat)) :
    ∀ (rows : List (Nat × Nat)) (amounts : Nat → Nat),
      SortedColumnRepresents rows amounts →
      SortedColumnRepresents
        (sortedVecWriteTrace rows writes)
        (functionalWriteTrace amounts writes) := by
  induction writes with
  | nil =>
      intro rows amounts good
      simpa [sortedVecWriteTrace, functionalWriteTrace] using good
  | cons pair tail ih =>
      rcases pair with ⟨key, value⟩
      intro rows amounts good
      change SortedColumnRepresents
        (sortedVecWriteTrace (sortedVecPut rows key value) tail)
        (functionalWriteTrace
          (fun query => if query = key then value else amounts query) tail)
      exact ih _ _ (sorted_vec_write_refines_function
        rows amounts key value good)

/-- Even an initially empty wallet book admits arbitrary sequences of
updates while remaining a correct representation of the balance function. -/
theorem sorted_vec_empty_history_refines
    (writes : List (Nat × Nat)) :
    SortedColumnRepresents (sortedVecWriteTrace [] writes)
      (functionalWriteTrace (fun _ => 0) writes) := by
  have emptyRep : SortedColumnRepresents [] (fun _ => 0) := by
    constructor
    · trivial
    · intro query
      rfl
  exact sorted_vec_write_trace_refines writes [] (fun _ => 0) emptyRep


/-!
## Stage 11: canonical sorted-column representations are unique

The production wallet/escrow writer never stores a zero balance. Combined
with strict increasing key order, this removes the final ambiguity in an
extensional finite-map representation: two canonical lists that agree at
EVERY lookup key must contain precisely the same entries in the same order.
This theorem is about Lean's sorted-list specification and does not yet
certify Rust's `binary_search_by` implementation.
-/

def PositiveColumnValues : List (Nat × Nat) → Prop
  | [] => True
  | (_, value) :: tail => 0 < value ∧ PositiveColumnValues tail

theorem sorted_vec_write_preserves_positive
    (rows : List (Nat × Nat)) (key value : Nat)
    (positive : PositiveColumnValues rows) :
    PositiveColumnValues (sortedVecPut rows key value) := by
  induction rows with
  | nil =>
      by_cases zero : value = 0
      · simp [sortedVecPut, PositiveColumnValues, zero]
      · have newPositive : 0 < value := by omega
        simp [sortedVecPut, PositiveColumnValues, zero, newPositive]
  | cons pair tail ih =>
      rcases pair with ⟨stored, atoms⟩
      obtain ⟨oldPositive, tailPositive⟩ := positive
      by_cases before : key < stored
      · by_cases zero : value = 0
        · simp [sortedVecPut, PositiveColumnValues, before, zero,
            oldPositive, tailPositive]
        · have newPositive : 0 < value := by omega
          simp [sortedVecPut, PositiveColumnValues, before, zero,
            newPositive, oldPositive, tailPositive]
      · by_cases equal : key = stored
        · by_cases zero : value = 0
          · simpa [sortedVecPut, before, equal, zero] using tailPositive
          · have newPositive : 0 < value := by omega
            simp [sortedVecPut, PositiveColumnValues, before, equal, zero,
              newPositive, tailPositive]
        · have updatedTail := ih tailPositive
          simpa [sortedVecPut, PositiveColumnValues, before, equal] using
            (show 0 < atoms ∧
                PositiveColumnValues (sortedVecPut tail key value)
              from ⟨oldPositive, updatedTail⟩)

def CanonicalSortedColumns (rows : List (Nat × Nat)) : Prop :=
  SortedVecKeys rows ∧ PositiveColumnValues rows

theorem canonical_sorted_columns_write
    (rows : List (Nat × Nat)) (key value : Nat)
    (valid : CanonicalSortedColumns rows) :
    CanonicalSortedColumns (sortedVecPut rows key value) := by
  exact ⟨sorted_vec_write_preserves_order rows key value valid.1,
    sorted_vec_write_preserves_positive rows key value valid.2⟩

theorem canonical_sorted_columns_write_history
    (writes : List (Nat × Nat)) :
    ∀ rows : List (Nat × Nat),
      CanonicalSortedColumns rows →
      CanonicalSortedColumns (sortedVecWriteTrace rows writes) := by
  induction writes with
  | nil =>
      intro rows good
      simpa [sortedVecWriteTrace] using good
  | cons pair tail ih =>
      rcases pair with ⟨key, value⟩
      intro rows good
      change CanonicalSortedColumns
        (sortedVecWriteTrace (sortedVecPut rows key value) tail)
      exact ih _ (canonical_sorted_columns_write rows key value good)

/-- No two canonical finite vectors can encode the same balance function
unless they are structurally identical. This rules out duplicate keys,
hidden zero entries and order-only ambiguity at the abstract layer. -/
theorem canonical_sorted_columns_extensional_unique :
    ∀ (left right : List (Nat × Nat)),
      CanonicalSortedColumns left →
      CanonicalSortedColumns right →
      (∀ query, sortedVecGet left query = sortedVecGet right query) →
      left = right := by
  intro left
  induction left with
  | nil =>
      intro right _ canonRight agree
      cases right with
      | nil => rfl
      | cons entry rest =>
          rcases entry with ⟨key, atoms⟩
          have nonzero : 0 < atoms := canonRight.2.1
          have same := agree key
          simp [sortedVecGet] at same
          omega
  | cons entry rest ih =>
      rcases entry with ⟨first, firstAtoms⟩
      intro right canonLeft canonRight agree
      cases right with
      | nil =>
          have nonzero : 0 < firstAtoms := canonLeft.2.1
          have same := agree first
          simp [sortedVecGet] at same
          omega
      | cons other tail =>
          rcases other with ⟨second, secondAtoms⟩
          have firstAbove : ∀ entry ∈ rest, first < entry.1 :=
            canonLeft.1.1
          have secondAbove : ∀ entry ∈ tail, second < entry.1 :=
            canonRight.1.1
          have firstPositive : 0 < firstAtoms := canonLeft.2.1
          have secondPositive : 0 < secondAtoms := canonRight.2.1
          have sameKey : first = second := by
            by_cases equal : first = second
            · exact equal
            · by_cases smaller : first < second
              · have same := agree first
                have skip : sortedVecGet ((second, secondAtoms) :: tail) first = 0 := by
                  simp [sortedVecGet, smaller]
                have found : sortedVecGet ((first, firstAtoms) :: rest) first =
                    firstAtoms := by simp [sortedVecGet]
                rw [found, skip] at same
                omega
              · have greater : second < first := by omega
                have same := agree second
                have skip : sortedVecGet ((first, firstAtoms) :: rest) second = 0 := by
                  simp [sortedVecGet, greater]
                have found : sortedVecGet ((second, secondAtoms) :: tail) second =
                    secondAtoms := by simp [sortedVecGet]
                rw [skip, found] at same
                omega
          subst second
          have sameAtoms : firstAtoms = secondAtoms := by
            have same := agree first
            simpa [sortedVecGet] using same
          subst secondAtoms
          have agreeRest : ∀ query, sortedVecGet rest query =
              sortedVecGet tail query := by
            intro query
            by_cases below : query ≤ first
            · have leftZero :=
                sorted_vec_read_below rest query first firstAbove below
              have rightZero :=
                sorted_vec_read_below tail query first secondAbove below
              exact leftZero.trans rightZero.symm
            · have greater : first < query := by omega
              have skipLeft :
                  sortedVecGet ((first, firstAtoms) :: rest) query =
                  sortedVecGet rest query := by
                simp [sortedVecGet, show ¬query < first by omega,
                  show query ≠ first by omega]
              have skipRight :
                  sortedVecGet ((first, firstAtoms) :: tail) query =
                  sortedVecGet tail query := by
                simp [sortedVecGet, show ¬query < first by omega,
                  show query ≠ first by omega]
              exact skipLeft.symm.trans ((agree query).trans skipRight)
          have equalTail := ih tail
            ⟨canonLeft.1.2, canonLeft.2.2⟩
            ⟨canonRight.1.2, canonRight.2.2⟩ agreeRest
          rw [equalTail]


/-!
## Stage 11: sound binary-search pivot decisions over arbitrary partitions

The production Vec uses `binary_search_by` and branches on key order.
These lemmas prove that *any* interior pivot of a strictly partitioned
finite key sequence supports the usual binary-search decisions. The
recursive tree below models an arbitrary finite sequence of such decisions,
including balanced midpoint trees. It does not prove the Rust standard
library chooses these pivots or manipulates Vec indices correctly.
-/

theorem linear_get_zero_without_key
    (rows : List (Nat × Nat)) (query : Nat)
    (absent : ∀ entry ∈ rows, entry.1 ≠ query) :
    linearColumnGet rows query = 0 := by
  induction rows with
  | nil => rfl
  | cons pair rest ih =>
      rcases pair with ⟨stored, atoms⟩
      have notHead : stored ≠ query :=
        absent (stored, atoms) (by simp)
      have notTail : ∀ entry ∈ rest, entry.1 ≠ query := by
        intro entry member
        exact absent entry (List.mem_cons.mpr (Or.inr member))
      simp [linearColumnGet, notHead, ih notTail]

theorem linear_get_append_without_prefix
    (front back : List (Nat × Nat)) (query : Nat)
    (absent : ∀ entry ∈ front, entry.1 ≠ query) :
    linearColumnGet (front ++ back) query =
      linearColumnGet back query := by
  induction front with
  | nil => rfl
  | cons pair tail ih =>
      rcases pair with ⟨stored, atoms⟩
      have notHead : stored ≠ query :=
        absent (stored, atoms) (by simp)
      have notTail : ∀ entry ∈ tail, entry.1 ≠ query := by
        intro entry member
        exact absent entry (List.mem_cons.mpr (Or.inr member))
      simp [linearColumnGet, notHead, ih notTail]

theorem linear_get_append_without_suffix
    (front back : List (Nat × Nat)) (query : Nat)
    (absent : ∀ entry ∈ back, entry.1 ≠ query) :
    linearColumnGet (front ++ back) query =
      linearColumnGet front query := by
  induction front with
  | nil =>
      have none := linear_get_zero_without_key back query absent
      simpa [linearColumnGet] using none
  | cons pair tail ih =>
      rcases pair with ⟨stored, atoms⟩
      by_cases same : stored = query
      · simp [linearColumnGet, same]
      · simp [linearColumnGet, same, ih]

/-- When left keys are smaller and right keys are larger, any pivot has
the correct binary-search three-way branching semantics. -/
theorem linear_get_pivot_split
    (left right : List (Nat × Nat))
    (pivot value query : Nat)
    (below : ∀ entry ∈ left, entry.1 < pivot)
    (above : ∀ entry ∈ right, pivot < entry.1) :
    linearColumnGet (left ++ (pivot, value) :: right) query =
      if query < pivot then linearColumnGet left query
      else if query = pivot then value else linearColumnGet right query := by
  by_cases before : query < pivot
  · have suffixAbsent :
        ∀ entry ∈ (pivot, value) :: right, entry.1 ≠ query := by
      intro entry member
      rcases List.mem_cons.mp member with atHead | atTail
      · subst entry
        omega
      · have greater := above entry atTail
        omega
    have unchanged := linear_get_append_without_suffix
      left ((pivot, value) :: right) query suffixAbsent
    simpa [before] using unchanged
  · by_cases equal : query = pivot
    · subst query
      have prefixAbsent : ∀ entry ∈ left, entry.1 ≠ pivot := by
        intro entry member
        have lt := below entry member
        omega
      have skipped := linear_get_append_without_prefix
        left ((pivot, value) :: right) pivot prefixAbsent
      simpa [before, linearColumnGet] using skipped
    · have after : pivot < query := by omega
      have prefixAbsent : ∀ entry ∈ left, entry.1 ≠ query := by
        intro entry member
        have lt := below entry member
        omega
      have skipped := linear_get_append_without_prefix
        left ((pivot, value) :: right) query prefixAbsent
      have notPivot : pivot ≠ query := by omega
      simpa [before, equal, linearColumnGet, notPivot] using skipped

/-- Any (balanced or unbalanced) finite decision tree of pivot keys. -/
inductive BinaryColumnTree where
  | empty
  | branch (key atoms : Nat)
      (left right : BinaryColumnTree)

def binaryColumnInorder : BinaryColumnTree → List (Nat × Nat)
  | .empty => []
  | .branch key atoms left right =>
      binaryColumnInorder left ++
        (key, atoms) :: binaryColumnInorder right

def binaryColumnRead : BinaryColumnTree → Nat → Nat
  | .empty, _ => 0
  | .branch key atoms left right, query =>
      if query < key then binaryColumnRead left query
      else if query = key then atoms else binaryColumnRead right query

/-- Every pivot separates the complete left/right subtrees; recursively
the same partition invariant must hold in both subtrees. -/
def BinaryColumnOrdered : BinaryColumnTree → Prop
  | .empty => True
  | .branch pivot _ left right =>
      BinaryColumnOrdered left ∧ BinaryColumnOrdered right ∧
      (∀ entry ∈ binaryColumnInorder left, entry.1 < pivot) ∧
      (∀ entry ∈ binaryColumnInorder right, pivot < entry.1)

/-- A sequence of binary-search pivot decisions is observationally
equivalent to scanning the entire represented ordered map. This is
proved for arbitrary finite tree depth and all query values. -/
theorem binary_column_read_matches_full_scan :
    ∀ tree : BinaryColumnTree,
      BinaryColumnOrdered tree →
      ∀ query,
        binaryColumnRead tree query =
        linearColumnGet (binaryColumnInorder tree) query := by
  intro tree
  induction tree with
  | empty =>
      intro _ query
      rfl
  | branch pivot atoms left right ihLeft ihRight =>
      intro ordered query
      rcases ordered with ⟨validLeft, validRight, below, above⟩
      change (if query < pivot then binaryColumnRead left query
        else if query = pivot then atoms
        else binaryColumnRead right query) =
          linearColumnGet
            (binaryColumnInorder left ++
              (pivot, atoms) :: binaryColumnInorder right) query
      rw [linear_get_pivot_split _ _ pivot atoms query below above]
      rw [ihLeft validLeft query, ihRight validRight query]

theorem binary_column_read_matches_sorted_lookup
    (tree : BinaryColumnTree) (query : Nat)
    (valid : BinaryColumnOrdered tree)
    (sorted : SortedVecKeys (binaryColumnInorder tree)) :
    binaryColumnRead tree query =
      sortedVecGet (binaryColumnInorder tree) query := by
  calc
    binaryColumnRead tree query =
      linearColumnGet (binaryColumnInorder tree) query :=
        binary_column_read_matches_full_scan tree valid query
    _ = sortedVecGet (binaryColumnInorder tree) query :=
      (sorted_vec_get_equals_linear _ query sorted).symm


/-- Concatenating sorted left/right sublists around an interior pivot
preserves strict sortedness whenever the pivot separates both sides. -/
theorem sorted_vec_join_pivot
    (left right : List (Nat × Nat)) (pivot value : Nat)
    (leftSorted : SortedVecKeys left)
    (rightSorted : SortedVecKeys right)
    (below : ∀ entry ∈ left, entry.1 < pivot)
    (above : ∀ entry ∈ right, pivot < entry.1) :
    SortedVecKeys (left ++ (pivot, value) :: right) := by
  induction left with
  | nil =>
      change (∀ entry ∈ right, pivot < entry.1) ∧ SortedVecKeys right
      exact ⟨above, rightSorted⟩
  | cons pair tail ih =>
      rcases pair with ⟨stored, oldValue⟩
      obtain ⟨headBeforeTail, tailSorted⟩ := leftSorted
      have headBeforePivot : stored < pivot :=
        below (stored, oldValue) (by simp)
      have tailBeforePivot : ∀ entry ∈ tail, entry.1 < pivot := by
        intro entry member
        exact below entry (List.mem_cons.mpr (Or.inr member))
      have tailJoined := ih tailSorted tailBeforePivot
      have headBeforeAll :
          ∀ entry ∈ tail ++ (pivot, value) :: right,
              stored < entry.1 := by
        intro entry member
        rcases List.mem_append.mp member with inTail | inSuffix
        · exact headBeforeTail entry inTail
        · rcases List.mem_cons.mp inSuffix with atPivot | inRight
          · subst entry
            exact headBeforePivot
          · have pivotBefore := above entry inRight
            omega
      change SortedVecKeys
        ((stored, oldValue) :: (tail ++ (pivot, value) :: right))
      exact ⟨headBeforeAll, tailJoined⟩

/-- The pivot-separation certificate alone is enough to establish that
an arbitrary tree's in-order traversal is globally and strictly sorted. -/
theorem binary_column_inorder_is_sorted :
    ∀ tree : BinaryColumnTree,
      BinaryColumnOrdered tree →
      SortedVecKeys (binaryColumnInorder tree) := by
  intro tree
  induction tree with
  | empty =>
      intro _
      trivial
  | branch pivot atoms left right ihLeft ihRight =>
      intro valid
      rcases valid with ⟨validLeft, validRight, below, above⟩
      change SortedVecKeys
        (binaryColumnInorder left ++
          (pivot, atoms) :: binaryColumnInorder right)
      exact sorted_vec_join_pivot _ _ pivot atoms
        (ihLeft validLeft) (ihRight validRight) below above

/-- One validated pivot-separation invariant certifies the binary-tree
lookup directly against the sorted-vector lookup for ANY finite depth. -/
theorem binary_column_read_refines_sorted_vec
    (tree : BinaryColumnTree) (query : Nat)
    (valid : BinaryColumnOrdered tree) :
    binaryColumnRead tree query =
      sortedVecGet (binaryColumnInorder tree) query := by
  exact binary_column_read_matches_sorted_lookup
    tree query valid (binary_column_inorder_is_sorted tree valid)


/-- Splitting a strictly increasing list at ANY interior pivot establishes
the left-less-than-pivot and right-greater-than-pivot conditions used by
the binary-search tree. This includes a midpoint selected at runtime. -/
theorem sorted_vec_any_pivot_has_valid_bounds
    (left right : List (Nat × Nat)) (pivot value : Nat)
    (sorted : SortedVecKeys (left ++ (pivot, value) :: right)) :
    (∀ entry ∈ left, entry.1 < pivot) ∧
      (∀ entry ∈ right, pivot < entry.1) := by
  induction left with
  | nil =>
      have bound : ∀ entry ∈ right, pivot < entry.1 := sorted.1
      refine ⟨?_, bound⟩
      intro entry member
      cases member
  | cons pair tail ih =>
      rcases pair with ⟨stored, atoms⟩
      have pivotInTail : (pivot, value) ∈ tail ++ (pivot, value) :: right :=
        List.mem_append.mpr (Or.inr (by simp))
      have storedBelow : stored < pivot :=
        sorted.1 (pivot, value) pivotInTail
      have next := ih sorted.2
      refine ⟨?_, next.2⟩
      intro entry member
      rcases List.mem_cons.mp member with atHead | inTail
      · subst entry
        exact storedBelow
      · exact next.1 entry inTail

/-- Every interior pivot leaves sorted sublists on both sides. -/
theorem sorted_vec_any_pivot_subtrees_sorted
    (left right : List (Nat × Nat)) (pivot value : Nat)
    (sorted : SortedVecKeys (left ++ (pivot, value) :: right)) :
    SortedVecKeys left ∧ SortedVecKeys right := by
  induction left with
  | nil =>
      exact ⟨by trivial, sorted.2⟩
  | cons pair tail ih =>
      rcases pair with ⟨stored, atoms⟩
      have next := ih sorted.2
      have below : ∀ entry ∈ tail, stored < entry.1 := by
        intro entry member
        exact sorted.1 entry (List.mem_append.mpr (Or.inl member))
      exact ⟨⟨below, next.1⟩, next.2⟩

/-- For any interior split of a sorted Vec, searching the appropriate
half according to the pivot comparison returns EXACTLY the same result
as searching the entire Vec. No particular pivot index is required. -/
theorem sorted_vec_any_pivot_search_sound
    (left right : List (Nat × Nat)) (pivot value query : Nat)
    (sorted : SortedVecKeys (left ++ (pivot, value) :: right)) :
    sortedVecGet (left ++ (pivot, value) :: right) query =
      if query < pivot then sortedVecGet left query
      else if query = pivot then value
      else sortedVecGet right query := by
  obtain ⟨below, above⟩ :=
    sorted_vec_any_pivot_has_valid_bounds left right pivot value sorted
  obtain ⟨leftSorted, rightSorted⟩ :=
    sorted_vec_any_pivot_subtrees_sorted left right pivot value sorted
  rw [sorted_vec_get_equals_linear _ query sorted]
  rw [linear_get_pivot_split left right pivot value query below above]
  rw [← sorted_vec_get_equals_linear left query leftSorted,
    ← sorted_vec_get_equals_linear right query rightSorted]

end TradeLayerFormal
