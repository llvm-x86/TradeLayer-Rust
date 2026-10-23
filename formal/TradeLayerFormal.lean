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

end TradeLayerFormal
