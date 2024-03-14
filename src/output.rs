use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::bail;
use anyhow::Result;
use itertools::Itertools;
use serde::Serialize;

use crate::core::Change;
use crate::core::DepKind;
use crate::core::EntityKind;
use crate::core::FileDep;
use crate::core::Span;
use crate::core::Tag;
use crate::core::TagDep;
use crate::core::TagId;

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
            let src_filename = &dep.src.file_key.filename;
            let tgt_filename = &dep.tgt.file_key.filename;

            if !filenames.contains(src_filename) || !filenames.contains(tgt_filename) {
                bail!("filename not found");
            }
        }

        let variables = filenames.into_iter().sorted().collect::<Vec<_>>();
        let lookup: HashMap<String, usize> =
            variables.iter().enumerate().map(|(i, s)| (s.to_string(), i)).collect();

        let mut cells: HashMap<(usize, usize), CellV1> = HashMap::new();

        for dep in deps.iter().sorted().filter(|d| !d.is_loop()) {
            let src_ix = *lookup.get(&dep.src.file_key.filename).unwrap();
            let tgt_ix = *lookup.get(&dep.tgt.file_key.filename).unwrap();

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
    entities: Vec<TagRes>,
    cells: Vec<CellV2>,
    changes: Vec<Change<TagId>>,
}

impl OutputV2 {
    pub fn build(tags: Vec<Tag>, deps: Vec<TagDep>, changes: Vec<Change<TagId>>) -> Result<Self> {
        let tags = tags.into_iter().map(|t| TagRes::from_tag(t)).collect_vec();
        let ids = tags.iter().map(|t| t.id).collect::<HashSet<_>>();

        for dep in &deps {
            if !ids.contains(&dep.src) || !ids.contains(&dep.tgt) {
                bail!("tag not found");
            }
        }

        let mut cells: HashMap<(TagId, TagId), CellV2> = HashMap::new();

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

        Ok(Self { schema: "2.0".to_string(), entities: tags, cells, changes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
struct TagRes {
    id: TagId,
    parent_id: Option<TagId>,
    name: String,
    kind: EntityKind,
    location: Span,
}

impl TagRes {
    fn from_tag(tag: Tag) -> Self {
        Self {
            id: tag.id,
            parent_id: tag.parent_id,
            name: tag.entity.name,
            kind: tag.entity.kind,
            location: tag.location,
        }
    }
}

#[derive(Serialize)]
struct CellV2 {
    src: TagId,
    dst: TagId,
    values: HashMap<DepKind, usize>,
}

impl CellV2 {
    fn new(src: TagId, dst: TagId) -> Self {
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
