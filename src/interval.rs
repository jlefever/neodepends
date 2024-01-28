use std::cmp::Ordering::*;

#[derive(Debug, Clone, Copy)]
pub struct Interval {
    a: usize,
    b: usize,
}

impl Interval {
    pub fn new(a: usize, b: usize) -> Option<Self> {
        if a <= b {
            Some(Self { a, b })
        } else {
            None
        }
    }

    pub fn singleton(point: usize) -> Self {
        Self { a: point, b: point }
    }
}

enum IntervalOrdering {
    Less,
    Greater,
    Overlap,
}

impl Interval {
    fn cmp(&self, other: &Self) -> IntervalOrdering {
        match (self.a.cmp(&other.a), self.b.cmp(&other.b)) {
            (Less, Less) => IntervalOrdering::Less,
            (Greater, Greater) => IntervalOrdering::Greater,
            _ => IntervalOrdering::Overlap,
        }
    }
}

#[derive(Debug)]
pub struct DisjointIntervalMap<T> {
    keys: Vec<Interval>,
    values: Vec<T>,
}

impl<T> DisjointIntervalMap<T> {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            values: Vec::new(),
        }
    }

    pub fn insert(&mut self, key: Interval, value: T) {
        if let Err(index) = self.search(&key) {
            self.keys.insert(index, key);
            self.values.insert(index, value);
            return;
        }

        panic!("attempted to insert an interval that overlaps with an existing interval");
    }

    pub fn get(&self, point: usize) -> Option<&T> {
        self.get_inner(point).map(|i| &self.values[i])
    }

    pub fn get_mut(&mut self, point: usize) -> Option<&mut T> {
        self.get_inner(point).map(|i| &mut self.values[i])
    }

    pub fn get_many(&self, key: &Interval) -> &[T] {
        let (i, j) = self.get_many_inner(key);
        &self.values[i..j]
    }

    pub fn get_many_mut(&mut self, key: &Interval) -> &mut [T] {
        let (i, j) = self.get_many_inner(key);
        &mut self.values[i..j]
    }

    fn get_inner(&self, point: usize) -> Option<usize> {
        self.search(&Interval::singleton(point)).ok()
    }

    fn get_many_inner(&self, key: &Interval) -> (usize, usize) {
        let a = Interval::singleton(key.a);
        let b = Interval::singleton(key.b);
        let i = self.search(&a).unwrap_or_else(|x| x);
        let j = self.search(&b).map(|x| x + 1).unwrap_or_else(|x| x);
        (i, j)
    }

    fn search(&self, key: &Interval) -> Result<usize, usize> {
        self.keys.binary_search_by(|x| match x.cmp(&key) {
            IntervalOrdering::Less => Less,
            IntervalOrdering::Greater => Greater,
            IntervalOrdering::Overlap => Equal,
        })
    }
}

pub fn baz() {
    let mut set = DisjointIntervalMap::new();
    set.insert(Interval::new(5, 10).unwrap(), "foo");
    set.insert(Interval::new(12, 15).unwrap(), "bar");
    set.insert(Interval::new(0, 2).unwrap(), "baz");

    println!("{:?}", set);
    println!("{:?}", set.get(5));
    println!("{:?}", set.get(10));
    println!("{:?}", set.get(11));
    println!("{:?}", set.get(1));
    println!("{:?}", set.get_many(&Interval::new(4, 4).unwrap()));
}
