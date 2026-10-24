//! Canonical ordered token columns stored as a sorted vector.
//!
//! This replaces BTreeMap only for wallet and channel escrow balances.
//! Indices come from an explicit half-open lower-bound loop; updates preserve strict key order.
//! A raw insert permits zero for snapshot corruption tests, while all
//! production balance mutations use the canonical delete-on-zero helper.
//! Prototype O(n) updates: NOT suitable for high-throughput production.

#![allow(unexpected_cfgs)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderedColumns<K: Ord> {
    entries: Vec<(K, u128)>,
}

impl<K: Ord> Default for OrderedColumns<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

fn refs<K>(pair: &(K, u128)) -> (&K, &u128) {
    (&pair.0, &pair.1)
}

impl<K: Ord> OrderedColumns<K> {
    /// Exact lower-bound position in the half-open [left, right) interval.
    ///
    /// On a strictly sorted support, every index below left has a key
    /// smaller than the query; every index from right onward has a key
    /// greater than or equal to the query. The interval shrinks on each
    /// iteration. left + (right - left) / 2 avoids midpoint overflow.
    fn lower_bound(&self, key: &K) -> usize {
        let mut left = 0usize;
        let mut right = self.entries.len();
        while left < right {
            let middle = left + (right - left) / 2;
            if self.entries[middle].0.cmp(key).is_lt() {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }

    fn locate(&self, key: &K) -> Result<usize, usize> {
        let index = self.lower_bound(key);
        match self.entries.get(index) {
            Some((stored, _)) if stored == key => Ok(index),
            _ => Err(index),
        }
    }

    pub(crate) fn get(&self, key: &K) -> Option<&u128> {
        self.locate(key).ok().map(|index| &self.entries[index].1)
    }

    /// Return the replaced value, if any. Keys remain strictly sorted.
    pub(crate) fn insert(&mut self, key: K, atoms: u128) -> Option<u128> {
        match self.locate(&key) {
            Ok(index) => Some(core::mem::replace(&mut self.entries[index].1, atoms)),
            Err(index) => {
                self.entries.insert(index, (key, atoms));
                None
            }
        }
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<u128> {
        self.locate(key)
            .ok()
            .map(|index| self.entries.remove(index).1)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = (&K, &u128)> + DoubleEndedIterator {
        self.entries.iter().map(refs::<K>)
    }

    #[cfg(test)]
    fn has_strict_keys(&self) -> bool {
        self.entries.windows(2).all(|p| p[0].0 < p[1].0)
    }
}

impl<'a, K: Ord> IntoIterator for &'a OrderedColumns<K> {
    type Item = (&'a K, &'a u128);
    type IntoIter = std::iter::Map<std::slice::Iter<'a, (K, u128)>, fn(&(K, u128)) -> (&K, &u128)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries
            .iter()
            .map(refs::<K> as fn(&(K, u128)) -> (&K, &u128))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn updates_match_independent_btree_oracle() {
        let mut baseline = BTreeMap::new();
        let mut actual = OrderedColumns::<(u32, u32)>::default();
        let mut seed = 0xdead_beef_cafe_2026u64;
        for step in 0u64..5000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (((seed >> 16) % 13) as u32, ((seed >> 33) % 31) as u32);
            let atoms = (seed >> 5) as u128;
            if step.is_multiple_of(5) {
                assert_eq!(actual.remove(&key), baseline.remove(&key));
            } else {
                assert_eq!(actual.insert(key, atoms), baseline.insert(key, atoms));
            }
            assert!(actual.has_strict_keys());
            assert_eq!(actual.len(), baseline.len());
            assert_eq!(actual.get(&key).copied(), baseline.get(&key).copied());
            assert_eq!(
                actual.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
                baseline.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn canonical_write_matches_tree_on_zero_and_nonzero_balances() {
        let mut baseline = BTreeMap::new();
        let mut actual = OrderedColumns::<(u16, u16)>::default();
        let mut seed = 0xb1a5_2026_00ff_1234u64;
        for step in 0u64..3000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = (((seed >> 14) % 9) as u16, ((seed >> 29) % 23) as u16);
            let atoms = if step.is_multiple_of(7) {
                0u128
            } else {
                u128::from(((seed >> 6) % 600) as u16 + 1)
            };
            crate::canonical_columns::put_nonzero(&mut actual, key, atoms);
            if atoms == 0 {
                baseline.remove(&key);
            } else {
                baseline.insert(key, atoms);
            }
            assert!(actual.has_strict_keys());
            assert_eq!(actual.len(), baseline.len());
            assert_eq!(
                actual.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
                baseline.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>()
            );
            assert!(actual.iter().all(|(_, atoms)| *atoms != 0));
        }
    }

    /// A deterministic native-key corpus spanning the full parser domain,
    /// including prefixes, boundary byte lengths and u32::MAX property IDs.
    /// BTreeMap checks storage effects; the separate Python oracle checks
    /// Rust's native tuple/byte ordering against an independent language.
    fn native_wallet_keys() -> Vec<(crate::ledger::PropertyId, crate::ledger::AccountId)> {
        use crate::ledger::{AccountId, PropertyId};

        let properties = [1, 2, 7, 65535, u32::MAX]
            .map(|id| PropertyId::try_from(id).expect("nonzero property"));
        let edge_names = vec![
            "!".to_owned(),
            "~".to_owned(),
            "A".to_owned(),
            "A!".to_owned(),
            "AA".to_owned(),
            "A~".to_owned(),
            "a".repeat(89),
            "a".repeat(90),
            format!("{}b", "a".repeat(89)),
            "~".repeat(90),
            "!".repeat(90),
            "Z".repeat(90),
        ];
        let mut keys = Vec::with_capacity(1100);
        for (index, name) in edge_names.iter().enumerate() {
            let property = properties[index % properties.len()];
            keys.push((property, AccountId::try_from(name.as_str()).unwrap()));
        }
        for index in 0..1024u32 {
            let property = properties[index as usize % properties.len()];
            let name = format!("wallet-{index:04}");
            keys.push((property, AccountId::try_from(name.as_str()).unwrap()));
        }
        keys.sort();
        keys.dedup();
        assert!(keys.len() >= 1024);
        keys
    }

    fn assert_native_matches_tree(
        actual: &OrderedColumns<(crate::ledger::PropertyId, crate::ledger::AccountId)>,
        oracle: &BTreeMap<(crate::ledger::PropertyId, crate::ledger::AccountId), u128>,
        step: usize,
        canonical: bool,
    ) {
        let observed: Vec<_> = actual
            .iter()
            .map(|(key, &atoms)| (key.clone(), atoms))
            .collect();
        let expected: Vec<_> = oracle
            .iter()
            .map(|(key, &atoms)| (key.clone(), atoms))
            .collect();
        assert_eq!(observed, expected, "state diverged after mutation {step}");
        assert!(
            actual.has_strict_keys(),
            "unsorted/duplicate keys at {step}"
        );
        assert_eq!(actual.len(), oracle.len(), "size mismatch at {step}");
        if canonical {
            assert!(
                observed.iter().all(|(_, atoms)| *atoms > 0),
                "stored zero under canonical policy at {step}"
            );
        }
    }

    #[test]
    fn native_keys_canonical_growth_shrink_and_trace_match_btree() {
        let keys = native_wallet_keys();
        let mut actual = OrderedColumns::default();
        let mut oracle = BTreeMap::new();
        let mut capacity_increases = 0usize;
        let mut last_capacity = actual.entries.capacity();
        let mut step = 0usize;

        // Grow through 1, 3, 16, 64, 256 and 1024 occupied slots.
        // Inserting in sorted order forces back insertion and Vec reallocations.
        for (index, key) in keys.iter().enumerate() {
            let amount = (index + 1) as u128;
            crate::canonical_columns::put_nonzero(&mut actual, key.clone(), amount);
            oracle.insert(key.clone(), amount);
            if actual.entries.capacity() > last_capacity {
                last_capacity = actual.entries.capacity();
                capacity_increases += 1;
            }
            assert_native_matches_tree(&actual, &oracle, step, true);
            step += 1;
        }
        assert!(capacity_increases >= 2, "did not exercise Vec growth");
        assert!(actual.len() >= 1024);

        // Descending deletions force repeat shrink, including empty support.
        for key in keys.iter().rev() {
            crate::canonical_columns::put_nonzero(&mut actual, key.clone(), 0);
            oracle.remove(key);
            assert_native_matches_tree(&actual, &oracle, step, true);
            step += 1;
        }
        assert_eq!(actual.len(), 0);

        // The deterministic mixed history forces interior insertions, zero
        // deletion of absent/present keys, replacement and support changes.
        let mut seed = 0x17_2026_beef_51a9u64;
        for iteration in 0..12_000usize {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let index = ((seed >> 16) as usize) % keys.len();
            let key = keys[index].clone();
            let amount = if iteration % 7 == 0 {
                0
            } else {
                u128::from((seed >> 33) % 8192 + 1)
            };
            crate::canonical_columns::put_nonzero(&mut actual, key.clone(), amount);
            if amount == 0 {
                oracle.remove(&key);
            } else {
                oracle.insert(key.clone(), amount);
            }
            assert_eq!(actual.get(&key).copied(), oracle.get(&key).copied());
            assert_native_matches_tree(&actual, &oracle, step, true);
            step += 1;
        }
    }

    #[test]
    fn native_keys_raw_insert_remove_returns_match_tree() {
        let keys = native_wallet_keys();
        let mut actual = OrderedColumns::default();
        let mut oracle = BTreeMap::new();
        let mut seed = 0x2026_1700_cafe_f00du64;
        for step in 0..12_000usize {
            seed = seed
                .wrapping_mul(2862933555777941757)
                .wrapping_add(3037000493);
            let key = keys[((seed >> 21) as usize) % keys.len()].clone();
            if step % 5 == 0 {
                assert_eq!(
                    actual.remove(&key),
                    oracle.remove(&key),
                    "remove result at {step}"
                );
            } else {
                // Raw storage permits zero, unlike canonical account updates.
                let amount = if step % 11 == 0 {
                    0
                } else {
                    u128::from(seed >> 7)
                };
                assert_eq!(
                    actual.insert(key.clone(), amount),
                    oracle.insert(key.clone(), amount),
                    "insert result at {step}"
                );
            }
            assert_eq!(actual.get(&key).copied(), oracle.get(&key).copied());
            assert_native_matches_tree(&actual, &oracle, step, false);
        }
    }

    #[test]
    fn native_lower_bound_matches_independent_linear_count() {
        use crate::ledger::{AccountId, PropertyId};

        let keys = native_wallet_keys();
        let mut columns = OrderedColumns::default();
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(columns.insert(key.clone(), (index + 1) as u128), None);
        }
        let mut queries = keys.clone();
        // All 1–90-byte prefix lengths, two families, and both extreme
        // properties: tests prefix matching and missing query insertion.
        for property in [1u32, u32::MAX] {
            let property = PropertyId::try_from(property).unwrap();
            for width in 1..=90 {
                for symbol in ["A", "~"] {
                    queries.push((
                        property,
                        AccountId::try_from(symbol.repeat(width).as_str()).unwrap(),
                    ));
                }
            }
        }
        assert!(queries.len() > 1_000);
        for (case, query) in queries.iter().enumerate() {
            let index = keys.iter().take_while(|existing| *existing < query).count();
            let present = keys.get(index).is_some_and(|existing| existing == query);
            let expected = if present { Ok(index) } else { Err(index) };
            assert_eq!(columns.locate(query), expected, "query {case}");
            assert_eq!(columns.lower_bound(query), index);
            assert_eq!(
                columns.get(query).copied(),
                present.then_some((index + 1) as u128),
                "query {case}"
            );
        }
    }

    #[test]
    fn front_middle_end_insert_replace_and_delete() {
        let mut map = OrderedColumns::<u16>::default();
        assert_eq!(map.insert(50, 5), None);
        assert_eq!(map.insert(10, 1), None);
        assert_eq!(map.insert(90, 9), None);
        assert_eq!(map.insert(40, 4), None);
        assert_eq!(map.insert(100, 10), None);
        assert_eq!(map.insert(40, 400), Some(4));
        assert_eq!(map.remove(&50), Some(5));
        assert_eq!(map.remove(&10), Some(1));
        assert_eq!(map.remove(&100), Some(10));
        assert_eq!(map.remove(&123), None);
        assert!(map.has_strict_keys());
        assert_eq!(
            map.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            vec![(40, 400), (90, 9)]
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;
    use crate::canonical_columns::put_nonzero;

    /// The front/tail Kani cases supplement the existing symbolic middle-key
    /// Vec harness; Rust differentials exercise all other key positions.
    /// Each case selects zero deletion or positive insertion symbolically.
    /// The fixed-slot oracle avoids comparator and Vec update semantics.
    fn assert_fixed_slot_update(target: u8, amount: u8) {
        let mut columns = OrderedColumns::<u8>::default();
        assert_eq!(columns.insert(2, 11), None);
        assert_eq!(columns.insert(4, 33), None);

        // Reference state uses fixed-slot assignment, independent of Vec
        // positional insert/remove, binary search and sorted iteration.
        let mut oracle: [Option<u128>; 5] = [None, Some(11), None, Some(33), None];
        oracle[usize::from(target - 1)] = if amount == 0 {
            None
        } else {
            Some(u128::from(amount))
        };

        put_nonzero(&mut columns, target, u128::from(amount));
        assert_eq!(
            columns.len(),
            oracle.iter().filter(|entry| entry.is_some()).count()
        );
        if columns.len() > 1 {
            assert!(columns.entries[0].0 < columns.entries[1].0);
        }
        if columns.len() > 2 {
            assert!(columns.entries[1].0 < columns.entries[2].0);
        }
        assert_eq!(columns.get(&1).copied(), oracle[0]);
        assert_eq!(columns.get(&2).copied(), oracle[1]);
        assert_eq!(columns.get(&3).copied(), oracle[2]);
        assert_eq!(columns.get(&4).copied(), oracle[3]);
        assert_eq!(columns.get(&5).copied(), oracle[4]);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn actual_vec_before_first_anchor() {
        let amount = if kani::any() { 0 } else { 17 };
        assert_fixed_slot_update(1, amount);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn actual_vec_after_second_anchor() {
        let amount = if kani::any() { 0 } else { 17 };
        assert_fixed_slot_update(5, amount);
    }

    /// Proof is over the actual Vec-backed production column store,
    /// not a substitute two-slot model. Fixed key geometry keeps allocation
    /// and iterator unwinding bounded; values span symbolic u8 inputs.
    #[kani::proof]
    #[kani::unwind(16)]
    fn actual_vec_backend_inserts_and_erases_a_middle_key() {
        let debit: u8 = kani::any();
        let credit: u8 = kani::any();
        let amount: u8 = kani::any();
        let mut columns = OrderedColumns::<u8>::default();
        assert_eq!(columns.insert(1, u128::from(debit)), None);
        assert_eq!(columns.insert(3, u128::from(credit)), None);
        put_nonzero(&mut columns, 2, u128::from(amount));
        assert_eq!(columns.get(&1).copied(), Some(u128::from(debit)));
        assert_eq!(columns.get(&3).copied(), Some(u128::from(credit)));
        if amount == 0 {
            assert_eq!(columns.get(&2).copied(), None);
            assert_eq!(columns.len(), 2);
        } else {
            assert_eq!(columns.get(&2).copied(), Some(u128::from(amount)));
            assert_eq!(columns.len(), 3);
        }
        put_nonzero(&mut columns, 2, 0);
        assert_eq!(columns.get(&2).copied(), None);
        assert_eq!(columns.get(&1).copied(), Some(u128::from(debit)));
        assert_eq!(columns.get(&3).copied(), Some(u128::from(credit)));
        assert_eq!(columns.len(), 2);
    }
    #[kani::proof]
    #[kani::unwind(16)]
    fn actual_vec_symbolic_query_locate_matches_two_anchor_scan() {
        // Cover all 256 u8 query keys against a fixed, strictly sorted
        // two-record Vec. The independent expected index is the count of
        // anchor keys strictly less than the queried key.
        let query: u8 = kani::any();
        let mut columns = OrderedColumns::<u8>::default();
        assert_eq!(columns.insert(2, 11), None);
        assert_eq!(columns.insert(4, 33), None);

        let expected_index = usize::from(2 < query) + usize::from(4 < query);
        let expected = if query == 2 {
            Ok(0)
        } else if query == 4 {
            Ok(1)
        } else {
            Err(expected_index)
        };

        assert_eq!(columns.lower_bound(&query), expected_index);
        assert_eq!(columns.locate(&query), expected);
        assert_eq!(
            columns.get(&query).copied(),
            if query == 2 {
                Some(11)
            } else if query == 4 {
                Some(33)
            } else {
                None
            }
        );
    }

    /// Isolate the real AccountId(String) derived comparator from Vec
    /// allocation and element shifting. For an arbitrary printable u8
    /// query against one fixed ASCII account, the Rust Ord result must be
    /// exactly the independent byte comparison.
    /// A proper-prefix and second-byte comparator fixture, with all 94
    /// printable ASCII suffix bytes symbolically quantified. This isolates
    /// production AccountId(String) Ord/equals without symbolic Vec shifts.
    #[kani::proof]
    #[kani::unwind(24)]
    fn native_account_two_byte_prefix_ord_matches_byte_oracle() {
        use crate::ledger::AccountId;

        let suffix: u8 = kani::any();
        kani::assume((0x21..=0x7e).contains(&suffix));
        let candidate = [b'A', suffix];
        let candidate_str = std::str::from_utf8(&candidate).unwrap();
        let query = AccountId::try_from(candidate_str).unwrap();
        let prefix = AccountId::try_from("A").unwrap();
        let anchor = AccountId::try_from("AZ").unwrap();

        // A proper prefix is always smaller than its two-byte extension,
        // regardless of the printable second byte.
        assert!(prefix < query);
        assert!(query > prefix);
        assert_ne!(query, prefix);

        // First bytes agree: the second byte determines the comparison.
        assert_eq!(query.cmp(&anchor), suffix.cmp(&b'Z'));
        assert_eq!(anchor.cmp(&query), b'Z'.cmp(&suffix));
        assert_eq!(query == anchor, suffix == b'Z');
    }

    #[kani::proof]
    #[kani::unwind(20)]
    fn native_account_one_byte_ascii_ord_matches_byte_oracle() {
        use crate::ledger::AccountId;

        let symbol: u8 = kani::any();
        kani::assume((0x21..=0x7e).contains(&symbol));
        let bytes = [symbol];
        let text = std::str::from_utf8(&bytes).unwrap();
        let query = AccountId::try_from(text).unwrap();
        let anchor = AccountId::try_from("A").unwrap();

        assert_eq!(anchor.cmp(&query), b'A'.cmp(&symbol));
        assert_eq!(query.cmp(&anchor), symbol.cmp(&b'A'));
        assert_eq!(query == anchor, symbol == b'A');
    }
}
