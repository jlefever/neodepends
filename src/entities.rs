use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use itertools::Itertools;
use lazy_static::lazy_static;
use tree_sitter::Language;
use tree_sitter::Node;
use tree_sitter::Parser;
use tree_sitter::Query;
use tree_sitter::QueryCursor;

use crate::core::Dep;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::EntityId;
use crate::core::EntityKind;
use crate::core::FileDep;
use crate::core::Lang;
use crate::core::Loc;
use crate::core::SubFilePosition;
use crate::loading::FileSystem;
use crate::sparse_vec::SparseVec;
use crate::stackgraphs::JAVA_SG;

lazy_static! {
    static ref JAVA_EXTRACTOR: EntityExtractor =
        EntityExtractor::new(JAVA_SG.language(), include_str!("../languages/java/tags.scm"));
}

pub fn extract(fs: FileSystem, filename: &str) -> Option<EntitySet> {
    match Lang::from_filename(filename) {
        Some(Lang::Java) => {
            Some(JAVA_EXTRACTOR.extract(&fs.load_by_filename(filename).unwrap(), filename).unwrap())
        }
        Some(_) => Some(
            to_singleton_entity_set(&fs.load_by_filename(filename).unwrap(), filename).unwrap(),
        ),
        _ => None,
    }
}

pub struct EntitySet {
    file_id: EntityId,
    entities: HashMap<EntityId, Entity>,
    locations: HashMap<EntityId, Vec<Loc>>,
    byte_table: SparseVec<EntityId>,
    row_table: SparseVec<EntityId>,
    ordered_ids: Vec<EntityId>,
}

impl EntitySet {
    #[allow(dead_code)]
    pub fn file_id(&self) -> EntityId {
        self.file_id
    }

    #[allow(dead_code)]
    pub fn filename(&self) -> &str {
        &self.entities.get(&self.file_id).unwrap().name
    }

    pub fn entities(&self) -> impl IntoIterator<Item = &Entity> {
        self.ordered_ids.iter().map(|id| &self.entities[id])
    }

    pub fn get_locations_for(&self, id: EntityId) -> &[Loc] {
        &self.locations.get(&id).unwrap()
    }

    pub fn get_by_position(&self, position: SubFilePosition) -> EntityId {
        match position {
            SubFilePosition::Byte(byte) => self.byte_table.get(byte),
            SubFilePosition::Row(row) => self.row_table.get(row),
        }
        .unwrap_or(self.file_id)
    }
}

impl FileDep {
    pub fn to_entity_dep(&self, src_set: &EntitySet, tgt_set: &EntitySet) -> EntityDep {
        let src = src_set.get_by_position(self.src.position);
        let tgt = tgt_set.get_by_position(self.tgt.position);
        Dep::new(src, tgt, self.kind, self.position)
    }
}

impl EntitySet {
    // Must be in topological order
    fn from_topo_vec(pairs: Vec<(Entity, Loc)>) -> Result<Self> {
        let mut file_id = None;
        let mut entities = HashMap::with_capacity(pairs.len());
        let mut locations: HashMap<EntityId, Vec<Loc>> = HashMap::with_capacity(pairs.len());
        let mut byte_table = SparseVec::with_capacity(pairs.len());
        let mut row_table = SparseVec::with_capacity(pairs.len());
        let mut ordered_ids = Vec::with_capacity(pairs.len());

        for (entity, loc) in pairs {
            if matches!(entity.kind, EntityKind::File) {
                if file_id.is_none() {
                    file_id = Some(entity.id);
                } else {
                    bail!("entity set cannot contain more than one file");
                }
            }

            if let Some(parent_id) = entity.parent_id {
                if !entities.contains_key(&parent_id) {
                    bail!("pairs must be sorted in topological order")
                }
            }

            if let Some(locs) = locations.get_mut(&entity.id) {
                locs.push(loc.clone());
            } else {
                locations.insert(entity.id, vec![loc.clone()]);
            }

            byte_table.insert_many(loc.start_byte, loc.end_byte, entity.id);
            row_table.insert_many(loc.start_row, loc.end_row, entity.id);
            ordered_ids.push(entity.id);
            entities.insert(entity.id, entity);
        }

        if let Some(file_id) = file_id {
            Ok(EntitySet { file_id, entities, locations, byte_table, row_table, ordered_ids })
        } else {
            bail!("entity set must contain exactly one file")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TagId(usize);

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
struct Tag {
    id: TagId,
    ancestor_ids: Vec<TagId>,
    name: String,
    kind: EntityKind,
    location: Loc,
}

impl Tag {
    fn file(id: TagId, name: String, location: Loc) -> Self {
        Self { id, ancestor_ids: vec![], name, kind: EntityKind::File, location }
    }

    fn find_parent_id(&self, tags: &HashSet<TagId>) -> Option<TagId> {
        for ancestor_id in &self.ancestor_ids {
            if tags.contains(&ancestor_id) {
                return Some(*ancestor_id);
            }
        }

        None
    }
}

pub struct EntityExtractor {
    language: Language,
    query: Query,
    kinds: Vec<Option<EntityKind>>,
    name: u32,
}

impl EntityExtractor {
    pub fn new(language: Language, query: &str) -> Self {
        let query = Query::new(language, query.as_ref()).context("failed to parse query").unwrap();

        let name = query.capture_index_for_name("name").unwrap();

        let kinds = query
            .capture_names()
            .iter()
            .map(|c| c.strip_prefix("tag.").map(|k| EntityKind::try_from(k).unwrap()))
            .collect::<Vec<_>>();

        Self { language, query, kinds, name }
    }

    pub fn extract(&self, source: &[u8], filename: &str) -> Result<EntitySet> {
        let mut parser = Parser::new();
        parser.set_language(self.language)?;
        let tree = parser.parse(source, None).context("failed to parse")?;
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();

        let mut tags = HashMap::new();
        tags.insert(
            TagId(root.id()),
            Tag::file(TagId(root.id()), filename.to_string(), Loc::from_range(&root.range())),
        );

        // Create a tag for each match
        for r#match in cursor.matches(&self.query, root, source) {
            let mut builder: TagBuilder = TagBuilder::default();

            for capture in r#match.captures {
                if capture.index == self.name {
                    builder.name(capture.node.utf8_text(source).unwrap().to_string());
                } else if let Some(kind) = self.kinds[capture.index as usize] {
                    builder.id(TagId(capture.node.id()));
                    builder.kind(kind);
                    builder.location(Loc::from_range(&capture.node.range()));
                    builder.ancestor_ids(collect_ancestor_ids(&capture.node));
                }
            }

            let tag = builder.build()?;
            tags.insert(tag.id, tag);
        }

        into_entity_set(tags)
    }
}

fn to_singleton_entity_set(source: &[u8], filename: &str) -> Result<EntitySet> {
    let text = String::from_utf8(source.to_vec())?;
    let mut newlines = Vec::new();

    for (byte, c) in text.char_indices() {
        if c == '\n' {
            newlines.push(byte);
        }
    }

    let end_byte = source.len() - 1;
    let end_row = newlines.len();
    let preceeding_length = newlines.last().map(|last| last + 1).unwrap_or(0);
    let end_column = source.len() - preceeding_length;

    let mut tags = HashMap::new();
    let loc = Loc { start_byte: 0, start_row: 0, start_column: 0, end_byte, end_row, end_column };
    tags.insert(TagId(0), Tag::file(TagId(0), filename.to_string(), loc));
    into_entity_set(tags)
}

fn into_entity_set(tags: HashMap<TagId, Tag>) -> Result<EntitySet> {
    let mut entities = Vec::with_capacity(tags.len());
    let mut entity_ids = HashMap::with_capacity(tags.len());
    let tag_ids = tags.keys().map(|&k| k).collect::<HashSet<_>>();

    let mut tags = tags.into_values().collect_vec();
    tags.sort_by_key(|t| (t.ancestor_ids.len(), t.location));

    for tag in tags {
        let parent_tag_id = tag.find_parent_id(&tag_ids);
        let parent_entity_id = parent_tag_id.map(|id| *entity_ids.get(&id).unwrap());
        let entity_id = EntityId::new(parent_entity_id, &tag.name, tag.kind);
        entity_ids.insert(tag.id, entity_id);
        let entity = Entity::new(entity_id, parent_entity_id, tag.name.clone(), tag.kind);
        entities.push((entity, tag.location.clone()));
    }

    EntitySet::from_topo_vec(entities)
}

fn collect_ancestor_ids(node: &Node) -> Vec<TagId> {
    let mut ids = Vec::new();
    let mut curr: Option<Node> = node.parent();

    while let Some(curr_node) = curr {
        ids.push(TagId(curr_node.id()));
        curr = curr_node.parent();
    }

    ids
}
