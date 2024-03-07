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
use crate::core::Tag;
use crate::loading::FileSystem;
use crate::sparse_vec::SparseVec;
use crate::stackgraphs::JAVA_SG;

lazy_static! {
    static ref JAVA_TAGGER: Tagger =
        Tagger::new(JAVA_SG.language(), include_str!("../languages/java/tags.scm"));
}

pub fn extract_tags(fs: FileSystem, filename: &str) -> Option<FileTagInfo> {
    let lang = Lang::from_filename(filename);

    if lang.is_none() {
        return None;
    }

    let source = &fs.load_by_filename(filename).unwrap();

    Some(match lang.unwrap() {
        Lang::Java => JAVA_TAGGER.extract_tags(source, filename).unwrap(),
        _ => to_singleton_entity_set(source, filename).unwrap(),
    })
}

pub struct FileTagInfo {
    entities: HashMap<EntityId, Entity>,
    table: LocationTable,
}

impl FileTagInfo {
    fn new<I: IntoIterator<Item = Entity>>(entities: I, table: LocationTable) -> Self {
        Self { entities: entities.into_iter().map(|e| (e.id, e)).collect(), table }
    }

    pub fn ids(&self) -> impl IntoIterator<Item = EntityId> + '_ {
        self.table.ids()
    }

    pub fn entities(&self) -> impl IntoIterator<Item = &Entity> {
        self.table.ids().into_iter().map(|id| &self.entities[&id])
    }

    pub fn tags(&self) -> impl IntoIterator<Item = Tag> + '_ {
        self.table.tags()
    }

    pub fn find_id(&self, position: PartialPosition) -> Option<EntityId> {
        self.table.find_id(position)
    }

    pub fn find_ids(&self, position: PartialSpan) -> impl IntoIterator<Item = EntityId> + '_ {
        self.table.find_ids(position)
    }
}

impl FileDep {
    pub fn to_entity_dep(&self, src_set: &FileTagInfo, tgt_set: &FileTagInfo) -> EntityDep {
        let src = src_set.find_id(self.src.position).unwrap();
        let tgt = tgt_set.find_id(self.tgt.position).unwrap();
        Dep::new(src, tgt, self.kind, self.position)
    }
}

struct LocationTable {
    tags: Vec<Tag>,
    bytes: SparseVec<EntityId>,
    rows: SparseVec<EntityId>,
}

impl LocationTable {
    fn from_topo_vec(tags: Vec<Tag>) -> Self {
        let mut bytes = SparseVec::with_capacity(tags.len());
        let mut rows = SparseVec::with_capacity(tags.len());

        for Tag { span, entity_id } in &tags {
            bytes.insert_many(span.start_byte(), span.end_byte(), *entity_id);
            rows.insert_many(span.start_row(), span.end_row(), *entity_id);
        }

        Self { tags, bytes, rows }
    }

    fn ids(&self) -> impl IntoIterator<Item = EntityId> + '_ {
        self.tags.iter().map(|&t| t.entity_id).unique()
    }

    fn tags(&self) -> impl IntoIterator<Item = Tag> + '_ {
        self.tags.iter().map(|&t| t)
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
struct CaptureId(usize);

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
struct Capture {
    id: CaptureId,
    ancestor_ids: Vec<CaptureId>,
    name: String,
    kind: EntityKind,
    location: Span,
}

impl Capture {
    fn singleton(filename: &str, end_position: Position) -> Self {
        Self {
            id: CaptureId(0),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            location: Span::new(Position::new(0, 0, 0), end_position),
        }
    }

    fn from_root_node(filename: &str, root: &Node<'_>) -> Self {
        Self {
            id: CaptureId(root.id()),
            ancestor_ids: vec![],
            name: filename.to_string(),
            kind: EntityKind::File,
            location: Span::from_ts(&root.range()),
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

    pub fn extract_tags(&self, source: &[u8], filename: &str) -> Result<FileTagInfo> {
        let mut parser = Parser::new();
        parser.set_language(self.language)?;
        let tree = parser.parse(source, None).context("failed to parse")?;
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();

        let mut captures = HashMap::new();
        let root_capture = Capture::from_root_node(filename, &root);
        captures.insert(root_capture.id, root_capture);

        for r#match in cursor.matches(&self.query, root, source) {
            let mut builder: CaptureBuilder = CaptureBuilder::default();

            for capture in r#match.captures {
                if capture.index == self.name {
                    builder.name(capture.node.utf8_text(source).unwrap().to_string());
                } else if let Some(kind) = self.kinds[capture.index as usize] {
                    builder.id(CaptureId(capture.node.id()));
                    builder.kind(kind);
                    builder.location(Span::from_ts(&capture.node.range()));
                    builder.ancestor_ids(collect_ancestor_ids(&capture.node));
                }
            }

            let capture = builder.build()?;
            captures.insert(capture.id, capture);
        }

        Ok(into_file_tag_info(captures))
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

fn to_singleton_entity_set(source: &[u8], filename: &str) -> Result<FileTagInfo> {
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
    let capture = Capture::singleton(filename, end_position);

    let mut captures = HashMap::new();
    captures.insert(capture.id, capture);
    Ok(into_file_tag_info(captures))
}

fn into_file_tag_info(captures: HashMap<CaptureId, Capture>) -> FileTagInfo {
    let mut tags = Vec::with_capacity(captures.len());
    let mut entities = Vec::with_capacity(captures.len());
    let mut entity_ids = HashMap::with_capacity(captures.len());
    let capture_ids = captures.keys().map(|&k| k).collect::<HashSet<_>>();

    for capture in captures.into_values().sorted_by_cached_key(|c| c.topo_key()) {
        let parent_capture_id = capture.find_parent_id(&capture_ids);
        let parent_entity_id = parent_capture_id.map(|id| *entity_ids.get(&id).unwrap());
        let entity_id = EntityId::new(parent_entity_id, &capture.name, capture.kind);
        let entity = Entity::new(entity_id, parent_entity_id, capture.name.clone(), capture.kind);
        tags.push(Tag::new(entity_id, capture.location));
        entities.push(entity);
        entity_ids.insert(capture.id, entity_id);
    }

    FileTagInfo::new(entities, LocationTable::from_topo_vec(tags))
}
