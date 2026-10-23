//! Canonical nonzero entry policy for production balance maps.
//!
//! This module controls insert/replace vs delete when a balance becomes zero.
//! The BTreeMap implementation uses the standard library directly. Kani checks
//! the identical branch logic with a bounded symbolic store, while Rust
//! regression tests exercise the real ordered tree and exact key ordering.

use crate::ordered_columns::OrderedColumns;
use std::collections::BTreeMap;

/// The only mutation methods needed by canonical balance columns.
/// Implementations must maintain all keys other than `key`.
pub(crate) trait ColumnWriter<K> {
    fn insert_column(&mut self, key: K, atoms: u128);
    fn remove_column(&mut self, key: &K);
}

impl<K: Ord> ColumnWriter<K> for BTreeMap<K, u128> {
    fn insert_column(&mut self, key: K, atoms: u128) {
        self.insert(key, atoms);
    }

    fn remove_column(&mut self, key: &K) {
        self.remove(key);
    }
}

impl<K: Ord> ColumnWriter<K> for OrderedColumns<K> {
    fn insert_column(&mut self, key: K, atoms: u128) {
        self.insert(key, atoms);
    }

    fn remove_column(&mut self, key: &K) {
        self.remove(key);
    }
}

/// A zero deletes exactly one key; any positive amount inserts or replaces.
/// This is the production branch used for available and channel-escrow maps.
fn put_canonical<K, W: ColumnWriter<K>>(store: &mut W, key: K, atoms: u128) {
    if atoms == 0 {
        store.remove_column(&key);
    } else {
        store.insert_column(key, atoms);
    }
}

pub(crate) fn put_nonzero<K, W: ColumnWriter<K>>(map: &mut W, key: K, atoms: u128) {
    put_canonical(map, key, atoms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_key_insertion_replacement_and_middle_erase_preserve_order() {
        let mut map = BTreeMap::from([(10u32, 7u128), (30, 11), (50, 13)]);
        put_nonzero(&mut map, 40, 17);
        put_nonzero(&mut map, 5, 19);
        put_nonzero(&mut map, 30, 23);
        assert_eq!(
            map.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            [(5, 19), (10, 7), (30, 23), (40, 17), (50, 13)]
        );
        put_nonzero(&mut map, 30, 0);
        put_nonzero(&mut map, 5, 0);
        put_nonzero(&mut map, 99, 0);
        assert_eq!(
            map.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            [(10, 7), (40, 17), (50, 13)]
        );
        assert!(map.values().all(|&v| v > 0));
    }

    #[test]
    fn composite_account_keys_update_only_the_selected_tuple() {
        let mut map = BTreeMap::from([
            ((7u32, "alice"), 100u128),
            ((7u32, "bob"), 50),
            ((8u32, "alice"), 25),
        ]);
        put_nonzero(&mut map, (7, "bob"), 0);
        put_nonzero(&mut map, (7, "carol"), 50);
        assert_eq!(map.get(&(7, "alice")), Some(&100));
        assert_eq!(map.get(&(8, "alice")), Some(&25));
        assert_eq!(map.get(&(7, "carol")), Some(&50));
        assert_eq!(map.get(&(7, "bob")), None);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Fixed two-slot writer. Symbolic amounts and initial presence cover
    /// both arms of the SAME production helper without verifying allocator,
    /// BTreeMap node splitting, or reference-counting internals.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TwoSlots {
        selected: Option<u128>,
        unrelated: Option<u128>,
    }

    impl ColumnWriter<u8> for TwoSlots {
        fn insert_column(&mut self, key: u8, atoms: u128) {
            match key {
                1 => self.selected = Some(atoms),
                2 => self.unrelated = Some(atoms),
                _ => panic!("fixture key outside declared support"),
            }
        }

        fn remove_column(&mut self, key: &u8) {
            match key {
                1 => self.selected = None,
                2 => self.unrelated = None,
                _ => panic!("fixture key outside declared support"),
            }
        }
    }

    /// Either key may be the target. Symbolic presence flags model inserted
    /// and existing columns; the remaining key must retain its original
    /// Option value for every possible nonzero/zero update.
    #[kani::proof]
    fn symbolic_key_choice_preserves_untouched_column() {
        let key: u8 = kani::any();
        kani::assume(key == 1 || key == 2);
        let prior_one: u128 = kani::any();
        let prior_two: u128 = kani::any();
        let has_one: bool = kani::any();
        let has_two: bool = kani::any();
        let amount: u128 = kani::any();
        let first = if has_one { Some(prior_one) } else { None };
        let second = if has_two { Some(prior_two) } else { None };
        let mut state = TwoSlots {
            selected: first,
            unrelated: second,
        };
        put_canonical(&mut state, key, amount);
        let expected = if amount == 0 { None } else { Some(amount) };
        if key == 1 {
            assert_eq!(state.selected, expected);
            assert_eq!(state.unrelated, second);
        } else {
            assert_eq!(state.selected, first);
            assert_eq!(state.unrelated, expected);
        }
    }

    #[kani::proof]
    fn canonical_put_preserves_unrelated_slot_and_nonzero_storage() {
        let old: u128 = kani::any();
        let other: u128 = kani::any();
        let amount: u128 = kani::any();
        let present: bool = kani::any();
        let original_other = Some(other);
        let mut state = TwoSlots {
            selected: if present { Some(old) } else { None },
            unrelated: original_other,
        };
        put_canonical(&mut state, 1u8, amount);
        assert_eq!(
            state.selected,
            if amount == 0 { None } else { Some(amount) }
        );
        assert_eq!(state.unrelated, original_other);
        if let Some(stored) = state.selected {
            assert!(stored > 0);
        }
        // Deleting the selected key twice leaves all unrelated state intact.
        put_canonical(&mut state, 1u8, 0);
        put_canonical(&mut state, 1u8, 0);
        assert_eq!(state.selected, None);
        assert_eq!(state.unrelated, original_other);
    }
}
