//! Atomic clone-validate-commit boundary for the offline ledger.
//! Validator side effects outside the supplied state are caller-owned.

#![allow(unexpected_cfgs)]

/// All in-memory state changes remain private until validation succeeds.
/// Kani covers concrete scalar instantiations; the real Ledger map operations
/// are covered by regression tests. This is not a generic proof of Clone.
pub(crate) fn stage_validate_commit<State: Clone, Error>(
    state: &mut State,
    validate: impl FnOnce(&mut State) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut pending = state.clone();
    validate(&mut pending)?;
    *state = pending;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_stage_is_not_published() {
        let mut live = (12_u128, false, false);
        let original = live;
        let result: Result<(), &str> = stage_validate_commit(&mut live, |pending| {
            pending.0 = 50;
            pending.1 = true;
            Err("invalid")
        });
        assert_eq!(result, Err("invalid"));
        assert_eq!(live, original);
        let result: Result<(), &str> = stage_validate_commit(&mut live, |pending| {
            pending.0 = 14;
            pending.1 = true;
            pending.2 = true;
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(live, (14, true, true));
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct MiniLedger {
        amount: u128,
        first_replay: bool,
        second_replay: bool,
        journal: u16,
    }

    /// Symbolic input checks cover early, middle and late validation failures
    /// after the staged copy has been changed.
    #[kani::proof]
    fn partially_mutated_rejected_stage_never_publishes() {
        let original = MiniLedger {
            amount: kani::any(),
            first_replay: kani::any(),
            second_replay: kani::any(),
            journal: kani::any(),
        };
        let new_amount: u128 = kani::any();
        let failure_point: u8 = kani::any();
        let mut live = original;
        let result: Result<(), u8> = stage_validate_commit(&mut live, |pending| {
            if failure_point == 1 {
                return Err(1);
            }
            pending.first_replay = true;
            pending.amount = new_amount;
            if failure_point == 2 {
                return Err(2);
            }
            pending.second_replay = true;
            pending.journal = 99;
            if failure_point == 3 {
                return Err(3);
            }
            Ok(())
        });
        if failure_point >= 1 && failure_point <= 3 {
            assert_eq!(result, Err(failure_point));
            assert_eq!(live, original);
        } else {
            assert_eq!(result, Ok(()));
            assert_eq!(live.amount, new_amount);
            assert!(live.first_replay && live.second_replay);
            assert_eq!(live.journal, 99);
        }
    }

    /// A repeated replay marker aborts even after its first occurrence had
    /// already modified the staged journal.
    #[kani::proof]
    fn duplicate_replay_rolls_back_first_staged_operation() {
        let original = MiniLedger {
            amount: kani::any(),
            first_replay: kani::any(),
            second_replay: kani::any(),
            journal: kani::any(),
        };
        let mut live = original;
        let result: Result<(), u8> = stage_validate_commit(&mut live, |pending| {
            if pending.first_replay {
                return Err(1);
            }
            pending.first_replay = true;
            pending.journal = pending.journal.wrapping_add(1);
            if pending.first_replay {
                return Err(2);
            }
            pending.second_replay = true;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(live, original);
    }
}
