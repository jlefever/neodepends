use std::collections::HashMap;
use std::collections::HashSet;

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
use crate::core::PartialPosition;
use crate::core::PartialSpan;
use crate::core::Position;
use crate::core::Span;
use crate::core::StableId;
use crate::loading::FileSystem;
use crate::sparse_vec::SparseVec;
use crate::stackgraphs::JAVA_SG;

lazy_static! {
    static ref JAVA_TAGGER: Tagger =
        Tagger::new(JAVA_SG.language(), include_str!("../languages/java/tags.scm"));
}

pub fn extract_entity_set(fs: FileSystem, filename: &str) -> Option<EntitySet> {
    let lang = Lang::from_filename(filename);

    if lang.is_none() {
        return None;
    }

    let source = &fs.load_by_filename(filename).unwrap();

    Some(match lang.unwrap() {
        Lang::Java => JAVA_TAGGER.extract(source, filename).unwrap(),
        _ => to_singleton_entity_set(source, filename).unwrap(),
    })
}

pub struct EntitySet {
    entities: HashMap<EntityId, Entity>,
    table: LocationTable,
}

impl EntitySet {
    fn from_topo_vec(entities: Vec<Entity>) -> Self {
        let table = LocationTable::from_topo_slice(&entities);
        Self { entities: entities.into_iter().map(|e| (e.id, e)).collect(), table }
    }

    pub fn ids(&self) -> impl IntoIterator<Item = EntityId> + '_ {
        self.table.ids()
    }

    pub fn entities(&self) -> impl IntoIterator<Item = &Entity> {
        self.table.ids().into_iter().map(|id| &self.entities[&id])
    }

    pub fn find_id(&self, position: PartialPosition) -> Option<EntityId> {
        self.table.find_id(position)
    }

    pub fn find_ids(&self, position: PartialSpan) -> impl IntoIterator<Item = EntityId> + '_ {
        self.table.find_ids(position)
    }
}

impl FileDep {
    pub fn to_entity_dep(&self, src_set: &EntitySet, tgt_set: &EntitySet) -> EntityDep {
        let src = src_set.find_id(self.src.position).unwrap();
        let tgt = tgt_set.find_id(self.tgt.position).unwrap();
        Dep::new(src, tgt, self.kind, self.position)
    }
}

struct LocationTable {
    ids: Vec<EntityId>,
    bytes: SparseVec<EntityId>,
    rows: SparseVec<EntityId>,
}

impl LocationTable {
    fn from_topo_slice(entities: &[Entity]) -> Self {
        let mut ids = Vec::with_capacity(entities.len());
        let mut bytes = SparseVec::with_capacity(entities.len());
        let mut rows = SparseVec::with_capacity(entities.len());

        for Entity { id, parent_id, location, .. } in entities {
            ids.push(*id);

            // TODO: Ensure the file location calculation is correct so this isn't necessary
            if parent_id.is_none() {
                bytes.insert_many(usize::MIN, usize::MAX, *id);
                rows.insert_many(usize::MIN, usize::MAX, *id);
            } else {
                bytes.insert_many(location.start_byte(), location.end_byte(), *id);
                rows.insert_many(location.start_row(), location.end_row(), *id);
            }
        }

        Self { ids, bytes, rows }
    }

    fn ids(&self) -> impl IntoIterator<Item = EntityId> + '_ {
        self.ids.iter().map(|id| *id)
    }

    fn find_id(&self, position: PartialPosition) -> Option<EntityId> {
        match position {
            PartialPosition::Byte(byte) => self.bytes.get(byte),
            PartialPosition::Row(row) => self.rows.get(row),
            PartialPosition::Whole(tag) => self.bytes.get(tag.byte),
        }
    }

    fn find_ids(&self, span: PartialSpan) -> impl IntoIterator<Item = EntityId> + '_ {
        match span {
            PartialSpan::Byte(start, end) => self.bytes.get_many(start, end),
            PartialSpan::Row(start, end) => self.rows.get_many(start, end),
            PartialSpan::Whole(whole) => self.bytes.get_many(whole.start.byte, whole.end.byte),
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
    location: Span,
}

impl Tag {
    fn singleton(filename: &str, end_position: Position) -> Self {
        Self {
            id: TagId(0),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            location: Span::new(Position::new(0, 0, 0), end_position),
        }
    }

    fn from_root_node(filename: &str, root: &Node<'_>) -> Self {
        Self {
            id: TagId(root.id()),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            location: Span::from_ts(&root.range()),
        }
    }

    fn find_parent_id(&self, tags: &HashSet<TagId>) -> Option<TagId> {
        for ancestor_id in &self.ancestor_ids {
            if tags.contains(&ancestor_id) {
                return Some(*ancestor_id);
            }
        }

        None
    }

    fn topo_key(&self) -> (usize, Span, String, EntityKind) {
        (self.ancestor_ids.len(), self.location, self.name.clone(), self.kind)
    }
}

pub struct Tagger {
    language: Language,
    query: Query,
    kinds: Vec<Option<EntityKind>>,
    name: u32,
}

impl Tagger {
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
        let root_tag = Tag::from_root_node(filename, &root);
        tags.insert(root_tag.id, root_tag);

        for r#match in cursor.matches(&self.query, root, source) {
            let mut builder: TagBuilder = TagBuilder::default();

            for capture in r#match.captures {
                if capture.index == self.name {
                    builder.name(capture.node.utf8_text(source).unwrap().to_string());
                } else if let Some(kind) = self.kinds[capture.index as usize] {
                    builder.id(TagId(capture.node.id()));
                    builder.kind(kind);
                    builder.location(Span::from_ts(&capture.node.range()));
                    builder.ancestor_ids(collect_ancestor_ids(&capture.node));
                }
            }

            let tag = builder.build()?;
            tags.insert(tag.id, tag);
        }

        Ok(into_file_entity_info(tags))
    }
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
    let end_position = Position::new(end_byte, end_row, end_column);
    let tag = Tag::singleton(filename, end_position);

    let mut tags = HashMap::new();
    tags.insert(tag.id, tag);
    Ok(into_file_entity_info(tags))
}

fn into_file_entity_info(tags: HashMap<TagId, Tag>) -> EntitySet {
    let mut entities = Vec::with_capacity(tags.len());
    let mut entity_ids = HashMap::with_capacity(tags.len());
    let mut stable_ids = HashMap::with_capacity(tags.len());
    let tag_ids = tags.keys().map(|&k| k).collect::<HashSet<_>>();

    for tag in tags.into_values().sorted_by_cached_key(|c| c.topo_key()) {
        let parent_tag_id = tag.find_parent_id(&tag_ids);
        let parent_entity_id = parent_tag_id.map(|id| *entity_ids.get(&id).unwrap());
        let parent_stable_id = parent_tag_id.map(|id| *stable_ids.get(&id).unwrap());

        let entity_id = EntityId::new(parent_entity_id, &tag.name, tag.kind, tag.location);
        entity_ids.insert(tag.id, entity_id);

        let stable_id = StableId::new(parent_stable_id, &tag.name, tag.kind);
        stable_ids.insert(tag.id, stable_id);

        entities.push(Entity::new(
            entity_id,
            parent_entity_id,
            stable_id,
            tag.name.clone(),
            tag.kind,
            tag.location,
        ));
    }

    EntitySet::from_topo_vec(entities)
}
