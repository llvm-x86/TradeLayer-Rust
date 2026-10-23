//! Single-use journal movement used by King settlement.
//!
//! Production moves use BTreeMap through a small storage interface. Kani
//! checks the same guard/mutation control flow using a finite-map model,
//! avoiding unbounded BTreeMap internal allocation and traversal paths.
//! The BTreeMap adapter is covered by Rust unit and integration tests;
//! this is not a bit-precise proof of the standard-library tree itself.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveJournalError {
    AlreadySwept,
    NotPending,
}

/// Minimal operations needed to check and move one journal entry.
/// Implementations must preserve all unrelated keys and never overwrite a
/// checked-absent destination key. The BTreeMap adapter below obeys that
/// contract under ordinary Rust collection semantics.
trait JournalStore<K, V> {
    fn contains(&self, key: &K) -> bool;
    fn take(&mut self, key: &K) -> Option<V>;
    fn put(&mut self, key: K, value: V);
}

impl<K: Ord, V> JournalStore<K, V> for BTreeMap<K, V> {
    fn contains(&self, key: &K) -> bool {
        self.contains_key(key)
    }

    fn take(&mut self, key: &K) -> Option<V> {
        self.remove(key)
    }

    fn put(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

/// Shared, checked journal-transition control flow.
/// The destination is checked first; removing an absent source is a no-op.
fn move_checked<K: Copy, V, P: JournalStore<K, V>, S: JournalStore<K, V>>(
    pending: &mut P,
    swept: &mut S,
    key: K,
) -> Result<(), MoveJournalError> {
    if swept.contains(&key) {
        return Err(MoveJournalError::AlreadySwept);
    }
    let claim = pending.take(&key).ok_or(MoveJournalError::NotPending)?;
    swept.put(key, claim);
    Ok(())
}

/// Actual production path used by `Ledger::perform_king_settlement`.
pub(crate) fn move_pending_once<K: Ord + Copy, V>(
    pending: &mut BTreeMap<K, V>,
    swept: &mut BTreeMap<K, V>,
    key: K,
) -> Result<(), MoveJournalError> {
    move_checked(pending, swept, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_entry_moves_once_and_other_keys_remain() {
        let mut pending = BTreeMap::from([(1u8, 17u8), (3, 19)]);
        let mut swept = BTreeMap::from([(2u8, 23u8)]);
        assert_eq!(move_pending_once(&mut pending, &mut swept, 1), Ok(()));
        assert_eq!(pending, BTreeMap::from([(3, 19)]));
        assert_eq!(swept, BTreeMap::from([(1, 17), (2, 23)]));
        let original_pending = pending.clone();
        let original_swept = swept.clone();
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 1),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 4),
            Err(MoveJournalError::NotPending)
        );
        assert_eq!(pending, original_pending);
        assert_eq!(swept, original_swept);
    }

    #[test]
    fn collision_preserves_original_pending_and_swept_values() {
        let mut pending = BTreeMap::from([(7u8, 11u8)]);
        let mut swept = BTreeMap::from([(7u8, 99u8)]);
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 7),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(pending.get(&7), Some(&11));
        assert_eq!(swept.get(&7), Some(&99));
    }

    #[test]
    fn absent_key_fails_without_mutating_either_map() {
        let mut pending = BTreeMap::from([(1u8, 19u8)]);
        let mut swept = BTreeMap::from([(2u8, 23u8)]);
        let original_pending = pending.clone();
        let original_swept = swept.clone();
        assert_eq!(
            move_pending_once(&mut pending, &mut swept, 9),
            Err(MoveJournalError::NotPending)
        );
        assert_eq!(pending, original_pending);
        assert_eq!(swept, original_swept);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// A finite, fixed-key journal model with slots for keys 0, 1 and 2.
    /// Symbolic values cover all u8 payloads. Only the shared checked
    /// control flow is verified; standard BTreeMap internals stay outside
    /// this harness's trusted boundaries.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TinyJournal([Option<u8>; 3]);

    impl JournalStore<u8, u8> for TinyJournal {
        fn contains(&self, key: &u8) -> bool {
            self.0
                .get(*key as usize)
                .is_some_and(|entry| entry.is_some())
        }

        fn take(&mut self, key: &u8) -> Option<u8> {
            self.0.get_mut(*key as usize).and_then(Option::take)
        }

        fn put(&mut self, key: u8, value: u8) {
            *self.0.get_mut(key as usize).expect("bounded proof key") = Some(value);
        }
    }

    #[kani::proof]
    fn single_use_retains_unrelated_symbolic_values() {
        let pending_value: u8 = kani::any();
        let unrelated_value: u8 = kani::any();
        let mut pending = TinyJournal([None, Some(pending_value), None]);
        let mut swept = TinyJournal([None, None, Some(unrelated_value)]);
        assert_eq!(move_checked(&mut pending, &mut swept, 1), Ok(()));
        assert_eq!(pending.0, [None, None, None]);
        assert_eq!(swept.0, [None, Some(pending_value), Some(unrelated_value)]);
        let original_pending = pending;
        let original_swept = swept;
        assert_eq!(
            move_checked(&mut pending, &mut swept, 1),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(
            move_checked(&mut pending, &mut swept, 0),
            Err(MoveJournalError::NotPending)
        );
        assert_eq!(pending, original_pending);
        assert_eq!(swept, original_swept);
    }

    #[kani::proof]
    fn collision_rejects_before_mutating_symbolic_claims() {
        let source_value: u8 = kani::any();
        let consumed_value: u8 = kani::any();
        let mut pending = TinyJournal([None, Some(source_value), None]);
        let mut swept = TinyJournal([None, Some(consumed_value), None]);
        assert_eq!(
            move_checked(&mut pending, &mut swept, 1),
            Err(MoveJournalError::AlreadySwept)
        );
        assert_eq!(pending.0, [None, Some(source_value), None]);
        assert_eq!(swept.0, [None, Some(consumed_value), None]);
    }
}
