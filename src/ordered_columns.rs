//! Canonical ordered token columns stored as a sorted vector.
//!
//! This replaces BTreeMap only for wallet and channel escrow balances.
//! Indices come from binary search; every insert preserves strict key order.
//! A raw insert permits zero for snapshot corruption tests, while all
//! production balance mutations use the canonical delete-on-zero helper.
//! Prototype O(n) updates: NOT suitable for high-throughput production.

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
    fn locate(&self, key: &K) -> Result<usize, usize> {
        self.entries
            .binary_search_by(|(existing, _)| existing.cmp(key))
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
