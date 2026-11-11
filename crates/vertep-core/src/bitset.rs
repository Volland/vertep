//! A small fixed-capacity bitset over dense state indices.
//!
//! The interpreter numbers states in document order, so ascending iteration is document order
//! (entry order) and descending iteration is reverse document order (exit order). The set is a
//! plain vector of words: no interior mutability, no unsafe code and no trait objects, which keeps
//! it inside the subset that a later Aeneas translation supports.

use alloc::vec;
use alloc::vec::Vec;

const BITS: usize = 64;

/// A set of state indices in `0..capacity`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StateSet {
    words: Vec<u64>,
}

impl StateSet {
    /// An empty set able to hold indices `0..capacity`.
    pub fn new(capacity: usize) -> Self {
        StateSet {
            words: vec![0; capacity.div_ceil(BITS)],
        }
    }

    /// The number of indices the set can hold (rounded up to a whole word).
    pub fn capacity(&self) -> usize {
        self.words.len() * BITS
    }

    /// Add `i`. Indices beyond the capacity grow the set.
    pub fn insert(&mut self, i: usize) {
        let w = i / BITS;
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        self.words[w] |= 1u64 << (i % BITS);
    }

    /// Remove `i`.
    pub fn remove(&mut self, i: usize) {
        let w = i / BITS;
        if w < self.words.len() {
            self.words[w] &= !(1u64 << (i % BITS));
        }
    }

    /// Whether `i` is a member.
    pub fn contains(&self, i: usize) -> bool {
        let w = i / BITS;
        w < self.words.len() && (self.words[w] >> (i % BITS)) & 1 == 1
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Remove every member.
    pub fn clear(&mut self) {
        for w in self.words.iter_mut() {
            *w = 0;
        }
    }

    /// Add every member of `other`.
    pub fn union_with(&mut self, other: &StateSet) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (a, b) in self.words.iter_mut().zip(other.words.iter()) {
            *a |= *b;
        }
    }

    /// Remove every member of `other`.
    pub fn difference_with(&mut self, other: &StateSet) {
        for (a, b) in self.words.iter_mut().zip(other.words.iter()) {
            *a &= !*b;
        }
    }

    /// Whether the two sets share a member.
    pub fn intersects(&self, other: &StateSet) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .any(|(a, b)| a & b != 0)
    }

    /// Whether any index in `lo..=hi` is a member.
    pub fn any_in_range(&self, lo: usize, hi: usize) -> bool {
        let mut i = lo;
        while i <= hi {
            if self.contains(i) {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Members in ascending order (document order).
    pub fn to_vec(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (wi, w) in self.words.iter().enumerate() {
            let mut bits = *w;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                out.push(wi * BITS + b);
                bits &= bits - 1;
            }
        }
        out
    }

    /// Members in descending order (reverse document order).
    pub fn to_vec_rev(&self) -> Vec<usize> {
        let mut v = self.to_vec();
        v.reverse();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_remove_iterate() {
        let mut s = StateSet::new(130);
        s.insert(0);
        s.insert(64);
        s.insert(129);
        assert!(s.contains(64));
        assert_eq!(s.to_vec(), vec![0, 64, 129]);
        assert_eq!(s.to_vec_rev(), vec![129, 64, 0]);
        s.remove(64);
        assert_eq!(s.len(), 2);
        assert!(s.any_in_range(100, 129));
        assert!(!s.any_in_range(1, 128));
        let mut t = StateSet::new(130);
        t.insert(129);
        assert!(s.intersects(&t));
        s.difference_with(&t);
        assert_eq!(s.to_vec(), vec![0]);
        s.insert(300);
        assert!(s.contains(300));
        s.clear();
        assert!(s.is_empty());
    }
}
