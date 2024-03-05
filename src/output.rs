use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::bail;
use anyhow::Result;
use itertools::Itertools;
use serde::Serialize;

use crate::core::DepKind;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::EntityId;
use crate::core::FileDep;

#[derive(Serialize)]
pub struct OutputV1 {
    schema: String,
    name: String,
    variables: Vec<String>,
    cells: Vec<CellV1>,
}

impl OutputV1 {
    pub fn build(name: &str, filenames: HashSet<String>, deps: Vec<FileDep>) -> Result<Self> {
        for dep in &deps {
            if !filenames.contains(&dep.src.filename) || !filenames.contains(&dep.tgt.filename) {
                bail!("filename not found");
            }
        }

        let variables = filenames.into_iter().sorted().collect::<Vec<_>>();
        let lookup: HashMap<String, usize> =
            variables.iter().enumerate().map(|(i, s)| (s.to_string(), i)).collect();

        let mut cells: HashMap<(usize, usize), CellV1> = HashMap::new();

        for dep in deps.iter().sorted().filter(|d| !d.is_loop()) {
            let src_ix = *lookup.get(&dep.src.filename).unwrap();
            let tgt_ix = *lookup.get(&dep.tgt.filename).unwrap();

            if let Some(cell) = cells.get_mut(&(src_ix, tgt_ix)) {
                cell.increment(dep.kind);
            } else {
                let mut cell = CellV1::new(src_ix, tgt_ix);
                cell.increment(dep.kind);
                cells.insert((src_ix, tgt_ix), cell);
            }
        }

        let cells =
            cells.into_iter().sorted_by_key(|(k, _)| k.clone()).map(|(_, c)| c).collect_vec();

        Ok(Self { schema: "1.0".to_string(), name: name.to_string(), variables, cells })
    }
}

#[derive(Serialize)]
struct CellV1 {
    src: usize,
    dest: usize,
    values: HashMap<DepKind, f32>,
}

impl CellV1 {
    fn new(src: usize, dest: usize) -> Self {
        Self { src, dest, values: HashMap::new() }
    }

    fn increment(&mut self, kind: DepKind) {
        if let Some(values) = self.values.get_mut(&kind) {
            *values += 1.0;
        } else {
            self.values.insert(kind, 1.0);
        }
    }
}

#[derive(Serialize)]
pub struct OutputV2 {
    schema: String,
    name: String,
    entities: Vec<Entity>,
    cells: Vec<CellV2>,
}

impl OutputV2 {
    pub fn build(name: &str, entities: Vec<Entity>, deps: Vec<EntityDep>) -> Result<Self> {
        let entities_set = entities.iter().map(|e| e.id).collect::<HashSet<_>>();

        for dep in &deps {
            if !entities_set.contains(&dep.src) || !entities_set.contains(&dep.tgt) {
                bail!("entity not found");
            }
        }

        let mut cells: HashMap<(EntityId, EntityId), CellV2> = HashMap::new();

        for dep in deps.iter().sorted().filter(|d| !d.is_loop()) {
            if let Some(cell) = cells.get_mut(&(dep.src, dep.tgt)) {
                cell.increment(dep.kind);
            } else {
                let mut cell = CellV2::new(dep.src, dep.tgt);
                cell.increment(dep.kind);
                cells.insert((dep.src, dep.tgt), cell);
            }
        }

        let cells =
            cells.into_iter().sorted_by_key(|(k, _)| k.clone()).map(|(_, c)| c).collect_vec();

        Ok(Self { schema: "2.0".to_string(), name: name.to_string(), entities, cells })
    }
}

#[derive(Serialize)]
struct CellV2 {
    src: EntityId,
    dst: EntityId,
    values: HashMap<DepKind, usize>,
}

impl CellV2 {
    fn new(src: EntityId, dst: EntityId) -> Self {
        Self { src, dst, values: HashMap::new() }
    }

    fn increment(&mut self, kind: DepKind) {
        if let Some(values) = self.values.get_mut(&kind) {
            *values += 1;
        } else {
            self.values.insert(kind, 1);
        }
    }
}
