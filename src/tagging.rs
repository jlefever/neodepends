use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context;
use anyhow::Result;
use counter::Counter;
use itertools::Itertools;
use tree_sitter::Language;
use tree_sitter::Node;
use tree_sitter::Parser;
use tree_sitter::Query;
use tree_sitter::QueryCursor;

use crate::core::ContentId;
use crate::core::Dep;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::EntityId;
use crate::core::EntityKind;
use crate::core::FileDep;
use crate::core::FileKey;
use crate::core::PartialPosition;
use crate::core::PartialSpan;
use crate::core::Position;
use crate::core::SimpleEntityId;
use crate::core::Span;
use crate::sparse_vec::SparseVec;

/// The ordered collection of entities contained within a particular [FileKey].
#[derive(Debug, Clone)]
pub struct EntitySet {
    entities: HashMap<EntityId, Entity>,
    table: LocationTable,
}

impl EntitySet {
    /// Create an [EntitySet] from a topologically ordered list of [Entity]s.
    ///
    /// In particular, an entity must appear later in the list than its parent.
    fn from_topo_vec(tags: Vec<Entity>) -> Self {
        let table = LocationTable::from_topo_slice(&tags);
        Self { entities: tags.into_iter().map(|e| (e.id, e)).collect(), table }
    }

    pub fn into_entities_vec(self) -> Vec<Entity> {
        let indices: HashMap<_, _> =
            self.table.ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        self.entities.into_values().sorted_by_key(|e| indices[&e.id]).collect()
    }

    pub fn find_id(&self, position: PartialPosition) -> Option<EntityId> {
        self.table.find_id(position)
    }

    pub fn count_simple_ids<I>(&self, spans: I) -> Counter<SimpleEntityId>
    where
        I: IntoIterator<Item = PartialSpan>,
    {
        spans
            .into_iter()
            .flat_map(|span| {
                self.table
                    .find_ids(span)
                    .into_iter()
                    .map(|(id, c)| (self.entities[&id].simple_id, c))
            })
            .collect()
    }
}

impl FileDep {
    pub fn to_entity_dep(&self, entity_sets: &HashMap<FileKey, EntitySet>) -> Option<EntityDep> {
        let src = entity_sets.get(&self.src.file_key)?.find_id(self.src.position)?;
        let tgt = entity_sets.get(&self.tgt.file_key)?.find_id(self.tgt.position)?;
        Some(Dep::new(src, tgt, self.kind, self.position, self.commit_id))
    }
}

#[derive(Debug, Clone)]
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

        for entity in entities {
            ids.push(entity.id);
            let location = entity.location();

            // TODO: Ensure the file location calculation is correct so this isn't necessary
            if entity.parent_id.is_none() {
                bytes.insert_many(usize::MIN, usize::MAX, entity.id);
                rows.insert_many(usize::MIN, usize::MAX, entity.id);
            } else {
                bytes.insert_many(location.start.byte, location.end.byte, entity.id);
                rows.insert_many(location.start.row, location.end.row, entity.id);
            }
        }

        Self { ids, bytes, rows }
    }

    fn find_id(&self, position: PartialPosition) -> Option<EntityId> {
        match position {
            PartialPosition::Row(row) => self.rows.get(row),
            PartialPosition::Whole(whole) => self.bytes.get(whole.byte),
        }
    }

    fn find_ids(&self, span: PartialSpan) -> Counter<EntityId> {
        match span {
            PartialSpan::Row(start, end) => self.rows.get_overlaps(start, end),
            PartialSpan::Whole(whole) => self.bytes.get_overlaps(whole.start.byte, whole.end.byte),
        }
    }
}

#[derive(Debug)]
pub enum Tagger {
    EntityLevel(EntityTagger),
    FileLevel,
}

impl Tagger {
    pub fn new(language: Option<Language>, tag_query: Option<&str>) -> Tagger {
        match (language, tag_query) {
            (Some(language), Some(query)) => Self::EntityLevel(EntityTagger::new(language, query)),
            _ => Self::FileLevel,
        }
    }

    pub fn tag(&self, filename: &str, content: &str, file_level: bool) -> EntitySet {
        if file_level {
            return to_singleton_entity_set(filename, content);
        }

        match self {
            Tagger::EntityLevel(tagger) => match tagger.tag(filename, content) {
                Ok(entity_set) => entity_set,
                Err(_) => to_singleton_entity_set(filename, content),
            },
            Tagger::FileLevel => to_singleton_entity_set(filename, content),
        }
    }
}

#[derive(Debug)]
pub struct EntityTagger {
    language: Language,
    query: Query,
    kinds: Vec<Option<EntityKind>>,
    ix_name: u32,
    ix_comment: Option<u32>,
}

impl EntityTagger {
    fn new(language: Language, tag_query: &str) -> Self {
        let query = Query::new(language, tag_query).unwrap();
        let ix_name = query.capture_index_for_name("name").unwrap();
        let ix_comment = query.capture_index_for_name("comment");

        let kinds = query
            .capture_names()
            .iter()
            .map(|c| c.strip_prefix("tag.").map(|k| EntityKind::try_from(k).unwrap()))
            .collect::<Vec<_>>();

        Self { language, query, kinds, ix_name, ix_comment }
    }

    fn tag(&self, filename: &str, content: &str) -> Result<EntitySet> {
        let mut parser = Parser::new();
        parser.set_language(self.language)?;
        let tree = parser.parse(content, None).context("failed to parse")?;
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();

        let mut captures = HashMap::new();
        let root_capture = Capture::from_root_node(filename, &root);
        captures.insert(root_capture.id, root_capture);

        for r#match in cursor.matches(&self.query, root, content.as_bytes()) {
            let mut builder: CaptureBuilder = CaptureBuilder::default();
            builder.comment(None);

            for capture in r#match.captures {
                if capture.index == self.ix_name {
                    builder.name(capture.node.utf8_text(content.as_bytes()).unwrap().to_string());
                } else if Some(capture.index) == self.ix_comment {
                    builder.comment(Some(Span::from_ts(capture.node.range())));
                } else if let Some(kind) = self.kinds[capture.index as usize] {
                    builder.id(CaptureId(capture.node.id()));
                    builder.ancestor_ids(collect_ancestor_ids(&capture.node));
                    builder.kind(kind);
                    builder.code(Span::from_ts(capture.node.range()));
                }
            }

            let capture = builder.build()?;
            captures.insert(capture.id, capture);
        }

        Ok(into_entity_set(captures, ContentId::from_content(content)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CaptureId(usize);

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
struct Capture {
    id: CaptureId,
    ancestor_ids: Vec<CaptureId>,
    name: String,
    kind: EntityKind,
    code: Span,
    comment: Option<Span>,
}

impl Capture {
    fn singleton(filename: &str, end_position: Position) -> Self {
        Self {
            id: CaptureId(0),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            code: Span::new(Position::new(0, 0, 0), end_position),
            comment: None,
        }
    }

    fn from_root_node(filename: &str, root: &Node<'_>) -> Self {
        Self {
            id: CaptureId(root.id()),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            code: root.range().into(),
            comment: None,
        }
    }

    fn find_parent_id(&self, captures: &HashSet<CaptureId>) -> Option<CaptureId> {
        for ancestor_id in &self.ancestor_ids {
            if captures.contains(&ancestor_id) {
                return Some(*ancestor_id);
            }
        }

        None
    }

    fn topo_key(&self) -> (usize, Span, String, EntityKind) {
        (self.ancestor_ids.len(), self.code, self.name.clone(), self.kind)
    }
}

fn collect_ancestor_ids(node: &Node) -> Vec<CaptureId> {
    let mut ids = Vec::new();
    let mut curr: Option<Node> = node.parent();

    while let Some(curr_node) = curr {
        ids.push(CaptureId(curr_node.id()));
        curr = curr_node.parent();
    }

    ids
}

fn to_singleton_entity_set(filename: &str, content: &str) -> EntitySet {
    let last = content.split_inclusive('\n').enumerate().last();

    let (end_row, end_col) = if let Some((end_row, end_line)) = last {
        (end_row, end_line.len())
    } else {
        (0, 0)
    };

    let end_position = Position::new(content.len(), end_row, end_col);
    let capture = Capture::singleton(filename, end_position);
    let mut captures = HashMap::with_capacity(1);
    captures.insert(capture.id, capture);
    into_entity_set(captures, ContentId::from_content(content))
}

fn into_entity_set(captures: HashMap<CaptureId, Capture>, content_id: ContentId) -> EntitySet {
    let mut entities = Vec::with_capacity(captures.len());
    let mut simple_ids = HashMap::with_capacity(captures.len());
    let mut entity_ids = HashMap::with_capacity(captures.len());
    let capture_ids = captures.keys().map(|&k| k).collect::<HashSet<_>>();

    for capture in captures.into_values().sorted_by_cached_key(|c| c.topo_key()) {
        let parent_capture_id = capture.find_parent_id(&capture_ids);

        let parent_simple_id = parent_capture_id.map(|id| *simple_ids.get(&id).unwrap());
        let simple_id = SimpleEntityId::new(parent_simple_id, &capture.name, capture.kind);
        simple_ids.insert(capture.id, simple_id);

        let parent_entity_id = parent_capture_id.map(|id| *entity_ids.get(&id).unwrap());
        let entity = Entity::new(
            parent_entity_id,
            capture.name,
            capture.kind,
            capture.code,
            capture.comment,
            content_id,
            simple_id,
        );
        entity_ids.insert(capture.id, entity.id);

        entities.push(entity);
    }

    EntitySet::from_topo_vec(entities)
}
