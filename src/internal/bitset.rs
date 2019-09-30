use crate::traits::record::Record;
use crate::types::arc_iter::ArcIter;
use crate::types::id::Id;
use crate::types::storage::Storage;
use std::borrow::Cow;
use std::iter::Flatten;
use std::iter::FromIterator;
use std::sync::Arc;

const CHUNK_SIZE: usize = 4096;
const BITS: usize = (std::mem::size_of::<usize>() * 8);

#[derive(Clone, Copy)]
pub(crate) struct Bitfield(usize, usize);

pub(crate) struct BitfieldIter {
    idx: usize,
    forward: usize,
    bits: usize,
}

/// A sparse bitset
#[derive(Clone)]
pub(crate) struct Bitset {
    ones: usize,
    bits: Arc<Storage<usize, usize, Bitfield>>,
}

impl Record<usize, usize> for Bitfield {
    fn chunk_key(&self) -> Cow<usize> {
        Cow::Owned(self.0 / CHUNK_SIZE)
    }

    fn item_key(&self) -> Cow<usize> {
        Cow::Owned(self.0)
    }
}

impl Bitfield {
    fn key(i: usize) -> (usize, usize, usize) {
        let idx = i / BITS;
        let chunk_idx = idx / CHUNK_SIZE;
        let bit = i % BITS;

        (chunk_idx, idx, 0b1 << bit)
    }
}

impl Bitset {
    /// True if there are no bits set
    pub fn is_empty(&self) -> bool {
        self.ones == 0
    }

    /// The number of bits set
    pub fn len(&self) -> usize {
        self.ones
    }

    /// Set the specific bit position in this Bitset
    pub fn set(&mut self, i: usize) {
        let (chunk_idx, idx, value) = Bitfield::key(i);
        let bits = &mut self.bits;
        let ones = &mut self.ones;

        Arc::make_mut(bits)
            .entry(&Id::new(chunk_idx, idx))
            .and_modify(|x| {
                if x.1 & value == 0 {
                    *ones += 1;
                    x.1 |= value;
                }
            })
            .or_insert_with(|| Bitfield(idx, value));
    }

    /// Set the specific bit position in this Bitset
    pub fn unset(&mut self, i: usize) {
        let (chunk_idx, idx, value) = Bitfield::key(i);
        let bits = &mut self.bits;
        let ones = &mut self.ones;

        Arc::make_mut(bits)
            .entry(&Id::new(chunk_idx, idx))
            .and_modify(|x| {
                if x.1 & value != 0 {
                    *ones += 1;
                    x.1 &= !value;
                }
            })
            .remove_if(|x| x.1 == 0b0);
    }

    /// Get the specific bit position in this Bitset
    pub fn get(&self, i: usize) -> bool {
        let (chunk_idx, idx, value) = Bitfield::key(i);

        self.bits
            .get(&Id::new(chunk_idx, idx))
            .map(|x| x.1)
            .unwrap_or(0b0)
            & value
            != 0b0
    }

    /// Iterate over all values set in this Bitset
    pub fn iter(&self) -> <Self as IntoIterator>::IntoIter {
        Storage::iter_arc(Arc::clone(&self.bits)).flatten()
    }
}

impl Default for Bitset {
    fn default() -> Self {
        Bitset {
            ones: 0,
            bits: Arc::new(Storage::new()),
        }
    }
}

impl IntoIterator for Bitfield {
    type IntoIter = BitfieldIter;
    type Item = usize;

    fn into_iter(self) -> Self::IntoIter {
        BitfieldIter {
            idx: self.0 * BITS,
            forward: 0,
            bits: self.1,
        }
    }
}

impl Iterator for BitfieldIter {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        if self.forward >= BITS {
            return None;
        }

        self.forward += (self.bits >> self.forward).trailing_zeros() as usize;

        if self.forward >= BITS {
            return None;
        }

        let result = self.idx + self.forward;
        self.forward += 1;
        Some(result)
    }
}

impl IntoIterator for Bitset {
    type Item = usize;
    type IntoIter = Flatten<ArcIter<usize, usize, Bitfield>>;

    fn into_iter(self) -> Self::IntoIter {
        Storage::iter_arc(self.bits).flatten()
    }
}

impl FromIterator<usize> for Bitset {
    fn from_iter<I: IntoIterator<Item = usize>>(iter: I) -> Self {
        let mut result = Self::default();

        for i in iter {
            result.set(i);
        }

        result
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::Rng;
    use std::collections::hash_set::HashSet;

    #[test]
    fn test_single_bit() {
        let mut b = Bitset::default();

        assert!(!b.get(7));
        b.set(7);
        assert!(b.get(7));

        assert!(!b.get(0));
        assert!(!b.get(8));
        assert!(!b.get(6));
        assert!(!b.get(257));

        assert_eq!(1, b.iter().count());

        for i in b.iter() {
            assert_eq!(7, i);
        }
    }

    #[test]
    fn test_tight_cluster() {
        let mut b = Bitset::default();

        b.set(19);
        b.set(20);
        b.set(21);
        b.set(23);
        b.set(24);
        b.set(27);

        assert!(b.get(19));
        assert!(b.get(20));
        assert!(b.get(21));
        assert!(!b.get(22));
        assert!(b.get(23));
        assert!(b.get(24));
        assert!(!b.get(25));
        assert!(!b.get(26));
        assert!(b.get(27));

        assert_eq!(6, b.iter().count());

        let v: Vec<_> = b.iter().collect();
        assert_eq!(&v, &[19, 20, 21, 23, 24, 27]);
    }

    #[test]
    fn test_unset() {
        let mut b = Bitset::default();

        b.set(19);
        b.set(20);
        b.set(21);
        b.set(23);
        b.set(24);
        b.set(27);

        assert!(b.get(19));
        assert!(b.get(20));
        assert!(b.get(21));
        assert!(!b.get(22));
        assert!(b.get(23));
        assert!(b.get(24));
        assert!(!b.get(25));
        assert!(!b.get(26));
        assert!(b.get(27));

        b.unset(19);
        b.unset(20);
        b.unset(21);
        b.unset(23);
        b.unset(24);
        b.unset(27);

        assert_eq!(0, b.iter().count());
    }

    #[test]
    fn test_sparse() {
        let mut b = Bitset::default();

        b.set(10);
        b.set(20);
        b.set(40);
        b.set(80);
        b.set(100);

        b.set(1000);
        b.set(2000);
        b.set(4000);
        b.set(8000);
        b.set(10000);

        b.set(20000);
        b.set(40000);
        b.set(80000);
        b.set(100_000);
        b.set(200_000);

        b.set(400_000);
        b.set(800_000);
        b.set(1_000_000);
        b.set(2_000_000);
        b.set(4_000_000);

        b.set(8_000_000);
        b.set(10_000_000);
        b.set(20_000_000);
        b.set(40_000_000);
        b.set(80_000_000);

        b.set(100_000_000);
        b.set(200_000_000);
        b.set(400_000_000);
        b.set(800_000_000);

        assert!(!b.get(600_000_000));
        assert!(b.get(800_000_000));

        assert_eq!(29, b.iter().count());
    }

    #[test]
    fn test_random() {
        let mut b = Bitset::default();
        let mut h = HashSet::new();

        for _ in 0..1000 {
          let x = rand::thread_rng().gen_range(0,10_000);
          b.set(x);
          h.insert(x);
        }

        for i in b.iter() {
          assert!(h.contains(&i));
        }

        for i in h.iter() {
          assert!(b.get(*i));
        }

        for i in h.iter() {
          assert!(b.get(*i));
        }

        for x in 0..10_000 {
          assert_eq!(b.get(x), h.contains(&x));
        }
    }
}
