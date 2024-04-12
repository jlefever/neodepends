use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use itertools::Itertools;

use crate::core::Change;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::EntityId;
use crate::core::EntityKind;

pub fn dsm_v1(entities: &[Entity], deps: &[EntityDep], changes: &[Change]) -> String {
    if entities.iter().any(|e| !e.kind.is_file()) {
        panic!("DSMv1 can only be made with files");
    }

    if entities.len() != entities.iter().map(|e| &e.name).unique().count() {
        panic!("DSMv1 must have unique filenames");
    }

    let indices: HashMap<_, _> = entities.iter().enumerate().map(|(i, e)| (e.id, i)).collect();

    let cochanges = calc_cochanges(&entities, &changes)
        .into_iter()
        .map(|(a, b)| ((indices[&a], indices[&b]), "Cochange"));

    let cells = deps
        .iter()
        .map(|d| ((indices[&d.src], indices[&d.tgt]), d.kind.as_ref()))
        .chain(cochanges)
        .into_group_map()
        .into_iter()
        .map(|((src, tgt), kinds)| CellV1::new(src, tgt, kinds))
        .sorted_by_key(|c| c.as_pair())
        .collect();

    let variables = entities.into_iter().map(|e| &e.name).collect();
    let matrix = Matrix { schema: "1.0".to_string(), variables, cells };
    serde_json::to_string_pretty(&matrix).unwrap()
}

pub fn dsm_v2(entities: &[Entity], deps: &[EntityDep], changes: &[Change]) -> String {
    if entities.len() != entities.iter().map(|e| &e.id).unique().count() {
        panic!("DSMv2 must have unique entity ids");
    }

    let indices: HashMap<_, _> = entities.iter().enumerate().map(|(i, e)| (e.id, i)).collect();

    let cochanges =
        calc_cochanges(&entities, &changes).into_iter().map(|(a, b)| ((a, b), "Cochange"));

    let cells = deps
        .iter()
        .map(|d| ((d.src, d.tgt), d.kind.as_ref()))
        .chain(cochanges)
        .into_group_map()
        .into_iter()
        .map(|((src, tgt), kinds)| CellV2::new(src, tgt, kinds))
        .sorted_by_key(|c| (indices[&c.src], indices[&c.tgt]))
        .collect();

    let variables = entities.into_iter().map(|e| EntityVar::from(e.clone())).collect();
    let matrix = Matrix { schema: "2.0".to_string(), variables, cells };
    serde_json::to_string_pretty(&matrix).unwrap()
}

#[derive(Debug, Clone)]
#[derive(serde::Serialize)]
struct Matrix<V, C> {
    schema: String,
    variables: Vec<V>,
    cells: Vec<C>,
}

/// This is just [Entity] but with less fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
struct EntityVar {
    id: EntityId,
    parent_id: Option<EntityId>,
    name: String,
    kind: EntityKind,
}

impl EntityVar {
    fn from(entity: Entity) -> Self {
        let Entity { id, parent_id, name, kind, .. } = entity;
        Self { id, parent_id, name, kind }
    }
}

#[derive(Debug, Clone)]
#[derive(serde::Serialize)]
struct CellV1 {
    src: usize,
    #[serde(rename = "dest")]
    tgt: usize,
    values: BTreeMap<String, f64>,
}

impl CellV1 {
    fn new(src: usize, tgt: usize, kinds: Vec<&str>) -> Self {
        let values = to_cell_values(kinds).into_iter().map(|(k, c)| (k, c as f64)).collect();
        Self { src, tgt, values }
    }

    fn as_pair(&self) -> (usize, usize) {
        (self.src, self.tgt)
    }
}

#[derive(Debug, Clone)]
#[derive(serde::Serialize)]
struct CellV2 {
    src: EntityId,
    #[serde(rename = "dest")]
    tgt: EntityId,
    values: BTreeMap<String, usize>,
}

impl CellV2 {
    fn new(src: EntityId, tgt: EntityId, kinds: Vec<&str>) -> Self {
        Self { src, tgt, values: to_cell_values(kinds) }
    }
}

fn to_cell_values(kinds: Vec<&str>) -> BTreeMap<String, usize> {
    kinds.into_iter().counts().into_iter().sorted().map(|(k, c)| (k.to_string(), c)).collect()
}

fn calc_cochanges(entities: &[Entity], changes: &[Change]) -> Vec<(EntityId, EntityId)> {
    let id_map = entities.iter().map(|e| (e.simple_id, e.id)).into_group_map();

    let commits = changes
        .iter()
        .map(|c| (c.simple_id, c.commit_id))
        .unique()
        .filter_map(|(s, c)| id_map.get(&s).map(|es| (c, es)))
        .flat_map(|(c, es)| es.iter().map(move |&e| (e, c)))
        .into_grouping_map()
        .collect::<HashSet<_>>();

    let mut pairs = Vec::new();
    let entity_ids = commits.keys().collect_vec();

    for i in 0..entity_ids.len() {
        let i_id = entity_ids[i];
        let i_commits = &commits[i_id];
        for j in (i + 1)..entity_ids.len() {
            let j_id = entity_ids[j];
            let j_commits = &commits[j_id];

            for _ in i_commits.intersection(j_commits) {
                pairs.push((*i_id, *j_id));
                pairs.push((*j_id, *i_id));
            }
        }
    }

    pairs
}
