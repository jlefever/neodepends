use std::cmp::Ordering;
use std::ops::Range;

#[derive(Debug, Clone, Copy)]
struct Interval {
    a: usize,
    b: usize,
}

impl Interval {
    fn new(a: usize, b: usize) -> Option<Self> {
        if a <= b {
            Some(Self { a, b })
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
}

#[derive(Debug)]
pub struct SparseVec<T: Copy + Eq> {
    entries: Vec<Entry<T>>,
}

impl<T: Copy + Eq> SparseVec<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    #[allow(dead_code)]
    pub fn insert(&mut self, point: usize, value: T) {
        self.insert_many(point, point, value)
    }

    pub fn insert_many(&mut self, start: usize, end: usize, value: T) {
        self.insert_inner(Interval::new(start, end).unwrap(), value);
    }

    #[allow(dead_code)]
    pub fn get(&self, point: usize) -> Option<T> {
        self.search(point).ok().map(|i| self.entries[i].value)
    }

    pub fn get_many(&self, start: usize, end: usize) -> impl IntoIterator<Item = T> + '_ {
        let indices = self.find_overlapping_indices(Interval::new(start, end).unwrap());
        self.entries[indices].iter().map(|e| e.value)
    }

    fn insert_inner(&mut self, key: Interval, value: T) {
        let indices = self.find_overlapping_indices(key);

        if indices.is_empty() {
            self.entries.insert(indices.start, Entry::new(key, value));
            return;
        }

        let entry_u = self.entries[indices.start];
        let entry_v = self.entries[indices.end - 1];

        self.entries.drain(indices.clone());
        self.try_insert(indices.start, key.b + 1, entry_v.key.b, entry_v.value);
        self.entries.insert(indices.start, Entry::new(key, value));
        self.try_insert(indices.start, entry_u.key.a, key.a - 1, entry_u.value);
    }

    fn try_insert(&mut self, index: usize, start: usize, end: usize, value: T) {
        if let Some(key) = Interval::new(start, end) {
            self.entries.insert(index, Entry::new(key, value))
        }
    }

    fn find_overlapping_indices(&self, key: Interval) -> Range<usize> {
        let i = self.search(key.a).unwrap_or_else(|i| i);
        let j = self.search(key.b).map(|j| j + 1).unwrap_or_else(|j| j);
        i..j
    }

    fn search(&self, point: usize) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| {
            if e.key.b < point {
                Ordering::Less
            } else if e.key.a > point {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
    }
}

pub fn baz() {
    let mut sparse_vec = SparseVec::new();
    sparse_vec.insert_many(5, 10, 10001);
    sparse_vec.insert_many(12, 15, 10002);
    sparse_vec.insert_many(0, 2, 10003);
    println!("{:?}", sparse_vec);

    sparse_vec.insert_many(6, 8, 10111);
    println!("{:?}", sparse_vec);

    for x in sparse_vec.get_many(8, 20) {
        println!("{}", x);
    }
}
