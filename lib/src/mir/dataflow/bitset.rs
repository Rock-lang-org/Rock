use std::marker::PhantomData;

use crate::ids::Idx;
use crate::mir::Local;

use super::Lattice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitSet<I> {
    bits: Vec<bool>,
    _marker: PhantomData<fn() -> I>,
}

impl<I> Default for BitSet<I> {
    fn default() -> Self {
        Self {
            bits: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<I: Idx> BitSet<I> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bits: vec![false; capacity],
            _marker: PhantomData,
        }
    }

    pub fn insert(&mut self, id: I) -> bool {
        let index = id.index();
        if self.bits.len() <= index {
            self.bits.resize(index + 1, false);
        }
        let changed = !self.bits[index];
        self.bits[index] = true;
        changed
    }

    pub fn remove(&mut self, id: I) -> bool {
        let index = id.index();
        if index >= self.bits.len() || !self.bits[index] {
            return false;
        }
        self.bits[index] = false;
        true
    }

    pub fn contains(&self, id: I) -> bool {
        self.bits.get(id.index()).copied().unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        !self.bits.iter().any(|bit| *bit)
    }

    pub fn iter(&self) -> BitSetIter<'_, I> {
        BitSetIter { set: self, next: 0 }
    }
}

impl<I: Idx> Lattice for BitSet<I> {
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        if self.bits.len() < other.bits.len() {
            self.bits.resize(other.bits.len(), false);
        }
        for (index, bit) in other.bits.iter().enumerate() {
            if *bit && !self.bits[index] {
                self.bits[index] = true;
                changed = true;
            }
        }
        changed
    }
}

pub struct BitSetIter<'a, I> {
    set: &'a BitSet<I>,
    next: usize,
}

impl<I: Idx> Iterator for BitSetIter<'_, I> {
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.set.bits.len() {
            let index = self.next;
            self.next += 1;
            if self.set.bits[index] {
                return Some(I::from_raw(index as u32));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalSet {
    bits: Vec<bool>,
}

impl LocalSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bits: vec![false; capacity],
        }
    }

    pub fn insert(&mut self, local: Local) -> bool {
        let index = local.0;
        if self.bits.len() <= index {
            self.bits.resize(index + 1, false);
        }
        let changed = !self.bits[index];
        self.bits[index] = true;
        changed
    }

    pub fn remove(&mut self, local: Local) -> bool {
        let index = local.0;
        if index >= self.bits.len() || !self.bits[index] {
            return false;
        }
        self.bits[index] = false;
        true
    }

    pub fn contains(&self, local: Local) -> bool {
        self.bits.get(local.0).copied().unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        !self.bits.iter().any(|bit| *bit)
    }

    pub fn iter(&self) -> LocalSetIter<'_> {
        LocalSetIter { set: self, next: 0 }
    }
}

impl Lattice for LocalSet {
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        if self.bits.len() < other.bits.len() {
            self.bits.resize(other.bits.len(), false);
        }
        for (index, bit) in other.bits.iter().enumerate() {
            if *bit && !self.bits[index] {
                self.bits[index] = true;
                changed = true;
            }
        }
        changed
    }
}

pub struct LocalSetIter<'a> {
    set: &'a LocalSet,
    next: usize,
}

impl Iterator for LocalSetIter<'_> {
    type Item = Local;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.set.bits.len() {
            let index = self.next;
            self.next += 1;
            if self.set.bits[index] {
                return Some(Local(index));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
