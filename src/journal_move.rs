//! Single-use journal entry movement shared with King settlement.
//!
//! This helper operates on the actual ordered maps used by the ledger.
//! It checks both source and destination before any map mutation. The
//! surrounding Ledger::apply_batch staging controls transaction atomicity.
//! Proof harnesses below exercise a bounded, concrete-key map fixture;
//! they do not prove arbitrary Rust BTreeMap topology or source authenticity.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveJournalError {
    AlreadySwept,
    NotPending,
}

/// Move one keyed claim exactly once. Neither map changes on an error.
///
/// The destination collision check happens before removing the source.
/// A valid caller supplies distinct maps and a key that identifies the
/// same claim on both sides of the movement.
pub(crate) fn move_pending_once<K: Ord + Copy, V>(
    pending: &mut BTreeMap<K, V>,
    swept: &mut BTreeMap<K, V>,
    key: K,
) -> Result<(), MoveJournalError> {
    if swept.contains_key(&key) {
        return Err(MoveJournalError::AlreadySwept);
    }
    let claim = pending.remove(&key).ok_or(MoveJournalError::NotPending)?;
    swept.insert(key, claim);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_entry_moves_exactly_once_and_other_keys_are_untouched() {
        let mut pending = BTreeMap::from([(1u8, 17u8), (3, 19)]);
        let mut swept = BTreeMap::from([(2u8, 23u8)]);
        assert_eq!(move_pending_once(&mut pending, &mut swept, 1), Ok(()));
        assert_eq!(pending, BTreeMap::from([(3, 19)]));
        assert_eq!(swept, BTreeMap::from([(1, 17), (2, 23)]));
        let before_pending = pending.clone();
        let before_swept = swept.clone();
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 1),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 4),
            Err(MoveJournalError::NotPending)
        );
        assert_eq!(pending, before_pending);
        assert_eq!(swept, before_swept);
    }

    #[test]
    fn collision_rejects_before_removing_original_pending_claim() {
        let mut pending = BTreeMap::from([(7u8, 11u8)]);
        let mut swept = BTreeMap::from([(7u8, 99u8)]);
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 7),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(pending.get(&7), Some(&11));
        assert_eq!(swept.get(&7), Some(&99));
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Executes the same production BTreeMap helper with symbolic values
    /// and fixed tiny map geometry. Checks success, duplicate, missing-key
    /// failure, and preservation of unrelated entries.
    #[kani::proof]
    #[kani::unwind(32)]
    fn real_ordered_map_movement_is_single_use() {
        let value: u8 = kani::any();
        let other: u8 = kani::any();
        let mut pending = BTreeMap::from([(1u8, value)]);
        let mut swept = BTreeMap::from([(2u8, other)]);
        let result = move_pending_once(&mut pending, &mut swept, 1);
        assert_eq!(result, Ok(()));
        assert!(pending.is_empty());
        assert_eq!(swept.len(), 2);
        assert_eq!(swept.get(&1), Some(&value));
        assert_eq!(swept.get(&2), Some(&other));

        let before_pending = pending.clone();
        let before_swept = swept.clone();
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 1),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 3),
            Err(MoveJournalError::NotPending)
        );
        assert_eq!(pending, before_pending);
        assert_eq!(swept, before_swept);
    }
}
