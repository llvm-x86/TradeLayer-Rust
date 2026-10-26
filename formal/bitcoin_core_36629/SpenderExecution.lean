import Std
import Lean.Elab.Tactic.Omega

/-!
Execution model for spender resolution in bitcoin/bitcoin#36629, pinned at
cf9b981fcc7ae52c7864beae35df5d60b5c322b4.

Sources:
  src/rpc/blockchain.cpp, getscripthistory steps 6-10
  src/index/txospenderindex.cpp, BuildSpenderPositions and FindSpender.

Confirmed spentness depends on the UTXO view. When a spender is resolved,
the transaction must carry the spent outpoint among its inputs. Counting extra
spenders is keyed by (block_hash,txid), just as count_transaction does.
-/
set_option autoImplicit false

namespace BitcoinCore36629.SpenderExecution

structure Outpoint where
  txid : Nat
  index : Nat
  deriving Repr, DecidableEq, BEq

structure SpendingTx where
  txid : Nat
  inputs : List Outpoint
  serializedBytes : Nat
  deriving Repr, DecidableEq

structure ObservedOutput where
  outpoint : Outpoint
  spent : Bool
  spendingTxid : Option Nat
  deriving Repr, DecidableEq

/-- An already spent output remains spent; otherwise a mempool conflict marks it. -/
def markMempool (includeSpenders : Bool) (out : ObservedOutput)
    (conflict : Option Nat) : ObservedOutput :=
  if out.spent then out
  else match conflict with
  | none => out
  | some id => { out with spent := true,
                 spendingTxid := if includeSpenders then some id else none }

/-- With spending details disabled, a fresh output gets only the spent flag. -/
theorem omission_preserves_missing_spender (out : ObservedOutput)
    (h : out.spendingTxid = none) (id : Nat) :
    (markMempool false out (some id)).spendingTxid = none := by
  cases hspent : out.spent <;> simp [markMempool, hspent, h]

/-- A conflict always marks an unspent output spent, regardless of verbosity. -/
theorem mempool_conflict_marks_spent (includeSpenders : Bool)
    (out : ObservedOutput) (id : Nat) :
    (markMempool includeSpenders out (some id)).spent = true := by
  cases hspent : out.spent <;> simp [markMempool, hspent]

/-- Matches the source resolve_spender condition for a single output. -/
def eligible (out : ObservedOutput) (spender : SpendingTx) : Prop :=
  out.spent = true ∧ out.spendingTxid = none ∧ out.outpoint ∈ spender.inputs

def resolve (out : ObservedOutput) (spender : SpendingTx) : ObservedOutput :=
  if eligible out spender then
    { out with spendingTxid := some spender.txid }
  else out

theorem newly_resolved_is_actual_input (out : ObservedOutput)
    (spender : SpendingTx) (hprior : out.spendingTxid = none)
    (hafter : (resolve out spender).spendingTxid = some spender.txid) :
    out.outpoint ∈ spender.inputs := by
  unfold resolve at hafter
  split at hafter
  · rename_i hit
    exact hit.2.2
  · simp [hprior] at hafter

/-- A completed resolution preserves the output's original spent flag. -/
theorem resolve_preserves_spent (out : ObservedOutput) (spender : SpendingTx) :
    (resolve out spender).spent = out.spent := by
  unfold resolve
  split <;> rfl

/-- Unspendable outputs and BIP30-excluded coinbases cannot be marked spent
    by a missing UTXO in the confirmed-chain stage. -/
def confirmedSpent (unspendable bip30Excluded foundInCoins : Bool) : Bool :=
  !unspendable && !bip30Excluded && !foundInCoins

theorem unspendable_is_unspent (bip30 found : Bool) :
    confirmedSpent true bip30 found = false := by decide

theorem bip30_exception_is_unspent (unspendable found : Bool) :
    confirmedSpent unspendable true found = false := by decide

theorem available_coin_is_unspent (unspendable bip30 : Bool) :
    confirmedSpent unspendable bip30 true = false := by decide

/-- Source count_transaction uses block hash plus transaction ID. -/
structure TxKey where
  blockHash : Nat
  txid : Nat
  deriving Repr, DecidableEq

/-- Transaction bodies are charged once per (blockHash, txid) key. -/
def countTransaction (seen : List TxKey) (budget : Nat)
    (key : TxKey) (txBytes : Nat) : Option (Nat × List TxKey × Bool) :=
  if key ∈ seen then some (budget, seen, false)
  else if txBytes ≤ budget then
    some (budget - txBytes, key :: seen, true)
  else none

theorem duplicate_spender_free (seen : List TxKey) (budget bytes : Nat)
    (key : TxKey) (h : key ∈ seen) :
    countTransaction seen budget key bytes = some (budget, seen, false) := by
  simp [countTransaction, h]

theorem new_spender_exhausts (seen : List TxKey) (budget bytes : Nat)
    (key : TxKey) (h : key ∉ seen) (tooLarge : budget < bytes) :
    countTransaction seen budget key bytes = none := by
  simp [countTransaction, h, Nat.not_le.mpr tooLarge]

theorem new_spender_charged_once (seen : List TxKey) (budget bytes : Nat)
    (key : TxKey) (h : key ∉ seen) (affordable : bytes ≤ budget) :
    countTransaction seen budget key bytes =
      some (budget - bytes, key :: seen, true) := by
  simp [countTransaction, h, affordable]

theorem charged_then_duplicate (seen : List TxKey) (budget bytes : Nat)
    (key : TxKey) (h : key ∉ seen) (affordable : bytes ≤ budget) :
    countTransaction (key :: seen) (budget - bytes) key bytes =
      some (budget - bytes, key :: seen, false) := by
  simp [countTransaction]

structure BlockPos where
  height : Nat
  offset : Nat
  deriving Repr, DecidableEq

structure LocatedSpender where
  txid : Nat
  block : Option BlockPos
  deriving Repr, DecidableEq

/-- Exact confirmed/mempool extra-spender ordering in getscripthistory. -/
def before (a b : LocatedSpender) : Bool :=
  match a.block, b.block with
  | some p, some q =>
      decide (p.height < q.height ∨
        (p.height = q.height ∧ p.offset < q.offset))
  | some _, none => true
  | none, some _ => false
  | none, none => decide (a.txid < b.txid)

theorem confirmed_precedes_mempool (a b : LocatedSpender)
    (ha : a.block.isSome = true) (hb : b.block = none) :
    before a b = true := by
  cases hblock : a.block with
  | none => simp [hblock] at ha
  | some pos => simp [before, hblock, hb]

theorem mempool_order_by_txid (a b : LocatedSpender)
    (ha : a.block = none) (hb : b.block = none)
    (hid : a.txid < b.txid) : before a b = true := by
  simp [before, ha, hb, hid]

theorem same_block_order_by_offset (a b : LocatedSpender)
    (height left right : Nat) (ha : a.block = some ⟨height, left⟩)
    (hb : b.block = some ⟨height, right⟩)
    (hOffset : left < right) : before a b = true := by
  simp [before, ha, hb, hOffset]

end BitcoinCore36629.SpenderExecution
