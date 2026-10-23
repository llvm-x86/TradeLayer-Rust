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
    (boundedDebit : debit ≤ maxU128) (boundedCredit : credit ≤ maxU128)
    (accepted : checkedMove debit credit amount =
      .ok (afterDebit, afterCredit)) :
    afterDebit ≤ maxU128 ∧ afterCredit ≤ maxU128 := by
  obtain ⟨_, funded, fits, debitEq, creditEq⟩ :=
    checked_move_success_is_exact debit credit amount afterDebit afterCredit accepted
  omega

end TradeLayerFormal
