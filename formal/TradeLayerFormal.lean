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

end TradeLayerFormal
