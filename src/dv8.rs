use std::collections::HashMap;
use std::collections::HashSet;
use std::num::NonZeroUsize;

use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize)]
pub struct Dv8Matrix {
    schema: String,
    name: String,
    variables: Vec<String>,
    cells: Vec<Dv8Cell>,
}

impl Dv8Matrix {
    pub fn build<S1: AsRef<str>, S2: AsRef<str>, S3: AsRef<str>>(
        name: S1,
        deps: Vec<(S2, S2)>,
        extras: Vec<S3>,
    ) -> Self {
        let mut variables = HashSet::new();

        for (src, dest) in &deps {
            variables.insert(src.as_ref().to_string());
            variables.insert(dest.as_ref().to_string());
        }

        for extra in &extras {
            variables.insert(extra.as_ref().to_string());
        }

        let variables = variables.into_iter().sorted().collect::<Vec<_>>();
        let lookup: HashMap<String, usize> = variables
            .iter()
            .enumerate()
            .map(|(i, s)| (s.to_string(), i))
            .collect();

        let mut cells: HashMap<(usize, usize), usize> = HashMap::new();

        for (src, dest) in &deps {
            let src_ix = *lookup.get(src.as_ref()).unwrap();
            let dest_ix = *lookup.get(dest.as_ref()).unwrap();

            if let Some(count) = cells.get_mut(&(src_ix, dest_ix)) {
                *count += 1;
            } else {
                cells.insert((src_ix, dest_ix), 1);
            }
        }

        let cells = cells
            .into_iter()
            .filter(|((src, dest), _)| src != dest)
            .sorted_by_key(|(k, _)| k.clone())
            .map(|((src, dest), n)| Dv8Cell::new(src, dest, n))
            .collect::<Vec<_>>();

        Self {
            schema: "1.0".to_string(),
            name: name.as_ref().to_string(),
            variables,
            cells,
        }
    }
}

#[derive(Serialize)]
struct Dv8Cell {
    src: usize,
    dest: usize,
    values: HashMap<String, usize>,
}

impl Dv8Cell {
    fn new(src: usize, dest: usize, n: usize) -> Self {
        Self {
            src,
            dest,
            values: HashMap::from([("Use".to_string(), n)]),
        }
    }
}
