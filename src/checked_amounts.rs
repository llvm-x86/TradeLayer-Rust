//! Production arithmetic kernel shared by direct transfers, channel commits,
//! channel transfers and King settlement. The proof harnesses compile this
//! exact source with Kani, for symbolic inputs across the entire u128 domain.
//! This function does not establish transaction authorization or chain origin.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveError {
    Zero,
    Insufficient,
    Overflow,
}

/// For two *distinct* ledger columns, atomically compute both new values.
/// The caller must validate column ownership and only write after Ok.
/// Same-account transfers are handled separately by the ledger as a no-op
/// (after a funds check), because double-counting would create false overflow.
pub(crate) fn move_atoms(
    debit: u128,
    credit: u128,
    amount: u128,
) -> Result<(u128, u128), MoveError> {
    if amount == 0 {
        return Err(MoveError::Zero);
    }
    if debit < amount {
        return Err(MoveError::Insufficient);
    }
    let credited = credit.checked_add(amount).ok_or(MoveError::Overflow)?;
    Ok((debit - amount, credited))
}


/** Add two contributions to the same property's issued-supply account.
 * Returned None means u128 overflow; no implicit wrapping is allowed. */
pub(crate) fn checked_supply_add(prior: u128, atoms: u128) -> Option<u128> {
    prior.checked_add(atoms)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn supply_add_bounds_are_exact() {
        assert_eq!(checked_supply_add(0, u128::MAX), Some(u128::MAX));
        assert_eq!(checked_supply_add(u128::MAX, 1), None);
        assert_eq!(checked_supply_add(100, 200), Some(300));
    }

    #[test]
    fn movement_and_rejection_boundaries() {
        assert_eq!(move_atoms(8, 4, 3), Ok((5, 7)));
        assert_eq!(move_atoms(0, 0, 0), Err(MoveError::Zero));
        assert_eq!(move_atoms(2, 0, 3), Err(MoveError::Insufficient));
        assert_eq!(move_atoms(10, u128::MAX, 1), Err(MoveError::Overflow));
        assert_eq!(move_atoms(u128::MAX, 0, u128::MAX), Ok((0, u128::MAX)));
        assert_eq!(
            move_atoms(u128::MAX, u128::MAX - 1, 1),
            Ok((u128::MAX - 1, u128::MAX))
        );
        assert_eq!(
            move_atoms(u128::MAX, u128::MAX, 1),
            Err(MoveError::Overflow)
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Exact guarded semantics of the production function for any u128 triple.
    /// Covers zero, insufficient debit, receiver overflow, success, and absence
    /// of arithmetic panic. Kani explores symbolic 128-bit inputs, not samples.
    #[kani::proof]
    fn all_u128_movements_match_checked_spec() {
        let debit: u128 = kani::any();
        let credit: u128 = kani::any();
        let amount: u128 = kani::any();
        let result = move_atoms(debit, credit, amount);
        if amount == 0 {
            assert_eq!(result, Err(MoveError::Zero));
        } else if debit < amount {
            assert_eq!(result, Err(MoveError::Insufficient));
        } else if credit.checked_add(amount).is_none() {
            assert_eq!(result, Err(MoveError::Overflow));
        } else {
            let (debited, credited) = result.unwrap();
            assert_eq!(debited + amount, debit);
            assert_eq!(credited - amount, credit);
            assert!(debited <= debit);
            assert!(credited >= credit);
        }
    }


    /// This exact helper is used in all supply accumulation and audit rows.
    /// Symbolic inputs span the complete representable u128 domain.
    #[kani::proof]
    fn checked_supply_add_is_precise_and_commutative() {
        let prior: u128 = kani::any();
        let atoms: u128 = kani::any();
        let sum = checked_supply_add(prior, atoms);
        assert_eq!(sum, prior.checked_add(atoms));
        assert_eq!(sum, checked_supply_add(atoms, prior));
        if let Some(value) = sum {
            assert!(value >= prior);
            assert!(value >= atoms);
            assert_eq!(value - prior, atoms);
            assert_eq!(value - atoms, prior);
        } else {
            assert!(prior > u128::MAX - atoms);
        }
    }

    /// When total supply fits in a u128, the same production function
    /// preserves it exactly for all successful moves.
    #[kani::proof]
    fn all_successful_movements_conserve_representable_supply() {
        let debit: u128 = kani::any();
        let credit: u128 = kani::any();
        let amount: u128 = kani::any();
        if let Some(total) = debit.checked_add(credit) {
            if let Ok((debited, credited)) = move_atoms(debit, credit, amount) {
                assert_eq!(debited.checked_add(credited), Some(total));
            }
        }
    }
}
