use std::cmp::Ordering;
use std::hash::Hash;
use std::ops::Range;

use counter::Counter;
use itertools::Itertools;

#[derive(Debug, Clone, Copy)]
struct Interval {
    start: usize,
    end: usize,
}

impl Interval {
    fn new(start: usize, end: usize) -> Self {
        Self::try_new(start, end).expect("`end` must be greater than or equal to `start`")
    }

    fn try_new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry<T: Copy + Eq> {
    key: Interval,
    value: T,
}

impl<T: Copy + Eq> Entry<T> {
    fn new(key: Interval, value: T) -> Self {
        Self { key, value }
    }

    fn try_from_triple(triple: (usize, usize, T)) -> Option<Self> {
        let (start, end, value) = triple;
        Some(Entry::new(Interval::try_new(start, end)?, value))
    }
}

#[derive(Debug, Clone)]
pub struct SparseVec<T: Copy + Eq> {
    entries: Vec<Entry<T>>,
}

impl<T: Copy + Eq + Hash> SparseVec<T> {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity) }
    }

    pub fn get(&self, point: usize) -> Option<T> {
        self.search(point).ok().map(|i| self.entries[i].value)
    }

    #[allow(dead_code)]
    pub fn get_many(&self, start: usize, end: usize) -> impl IntoIterator<Item = T> + '_ {
        let indices = self.find_overlapping_indices(Interval::new(start, end));
        self.entries[indices].iter().map(|e| e.value).dedup()
    }

    pub fn get_overlaps(&self, start: usize, end: usize) -> Counter<T> {
        let mut counts = Counter::new();

        for i in self.find_overlapping_indices(Interval::new(start, end)) {
            let entry = self.entries[i];
            let start = usize::max(entry.key.start, start);
            let end = usize::min(entry.key.end, end);
            counts[&entry.value] += 1 + end - start;
        }

        counts
    }

    #[allow(dead_code)]
    pub fn insert(&mut self, point: usize, value: T) {
        self.insert_many(point, point, value)
    }

    pub fn insert_many(&mut self, start: usize, end: usize, value: T) {
        let key = Interval::new(start, end);
        let indices = self.find_overlapping_indices(key);

        if indices.is_empty() {
            self.entries.insert(indices.start, Entry::new(key, value));
            return;
        }

        let entry_a = self.entries[indices.start];
        let entry_z = self.entries[indices.end - 1];

        let replacements = [
            (entry_a.key.start, start - 1, entry_a.value),
            (start, end, value),
            (end + 1, entry_z.key.end, entry_z.value),
        ]
        .into_iter()
        .filter_map(Entry::try_from_triple);

        self.entries.splice(indices, replacements);
    }

    fn find_overlapping_indices(&self, key: Interval) -> Range<usize> {
        let i = self.search(key.start).unwrap_or_else(|i| i);
        let j = self.search(key.end).map(|j| j + 1).unwrap_or_else(|j| j);
        i..j
    }

    fn search(&self, point: usize) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| {
            if e.key.end < point {
                Ordering::Less
            } else if e.key.start > point {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
    }
}
