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

use crate::core::Dep;
use crate::core::Entity;
use crate::core::EntityId;
use crate::core::EntityKind;
use crate::core::FileDep;
use crate::core::FileKey;
use crate::core::PartialPosition;
use crate::core::PartialSpan;
use crate::core::Position;
use crate::core::Span;
use crate::core::Tag;
use crate::core::TagDep;
use crate::core::TagId;
use crate::languages::Lang;
use crate::loading::FileSystem;
use crate::sparse_vec::SparseVec;

pub fn extract_tag_set(fs: &FileSystem, file_key: &FileKey) -> TagSet {
    let lang = Lang::from_filename(&file_key.filename).unwrap();
    let source = &fs.load(file_key).unwrap();
    let filename = &file_key.filename;

    let tag_set = match &lang.config().tag_query {
        Some(query) => Tagger::new(lang.config().language, query).extract(source, filename).ok(),
        _ => Some(to_singleton_entity_set(source, filename).unwrap()),
    };

    if let Some(tag_set) = tag_set {
        tag_set
    } else {
        log::warn!("Failed to extract entities from {}", filename);
        to_singleton_entity_set(source, filename).unwrap()
    }
}

pub struct TagSet {
    tags: HashMap<TagId, Tag>,
    table: LocationTable,
}

impl TagSet {
    fn from_topo_vec(tags: Vec<Tag>) -> Self {
        let table = LocationTable::from_topo_slice(&tags);
        Self { tags: tags.into_iter().map(|e| (e.id, e)).collect(), table }
    }

    pub fn get_tag(&self, id: TagId) -> Option<&Tag> {
        self.tags.get(&id)
    }

    pub fn tag_ids(&self) -> impl IntoIterator<Item = TagId> + '_ {
        self.table.ids()
    }

    pub fn tags(&self) -> impl IntoIterator<Item = &Tag> {
        self.table.ids().into_iter().map(|id| &self.tags[&id])
    }

    pub fn entity_ids(&self) -> impl IntoIterator<Item = EntityId> + '_ {
        self.tags().into_iter().map(|t| t.entity.id)
    }

    pub fn find_tag_id(&self, position: PartialPosition) -> Option<TagId> {
        self.table.find_id(position)
    }

    pub fn find_tag(&self, position: PartialPosition) -> Option<&Tag> {
        self.find_tag_id(position).and_then(|id| self.tags.get(&id))
    }

    pub fn find_entity_id(&self, position: PartialPosition) -> Option<EntityId> {
        self.find_tag(position).map(|t| t.entity.id)
    }

    pub fn find_tag_ids(&self, span: PartialSpan) -> Counter<TagId> {
        self.table.find_ids(span)
    }

    pub fn find_tags(&self, span: PartialSpan) -> Counter<&Tag> {
        self.find_tag_ids(span).into_iter().map(|(id, c)| (&self.tags[&id], c)).collect()
    }

    pub fn find_entity_ids(&self, span: PartialSpan) -> Counter<EntityId> {
        self.find_tag_ids(span).into_iter().map(|(id, c)| (self.tags[&id].entity.id, c)).collect()
    }
}

impl FileDep {
    pub fn to_entity_dep(&self, src_set: &TagSet, tgt_set: &TagSet) -> TagDep {
        let src = src_set.find_tag_id(self.src.position).unwrap();
        let tgt = tgt_set.find_tag_id(self.tgt.position).unwrap();
        Dep::new(src, tgt, self.kind, self.position)
    }
}

struct LocationTable {
    ids: Vec<TagId>,
    bytes: SparseVec<TagId>,
    rows: SparseVec<TagId>,
}

impl LocationTable {
    fn from_topo_slice(tags: &[Tag]) -> Self {
        let mut ids = Vec::with_capacity(tags.len());
        let mut bytes = SparseVec::with_capacity(tags.len());
        let mut rows = SparseVec::with_capacity(tags.len());

        for Tag { id, entity: path, location, .. } in tags {
            ids.push(*id);

            // TODO: Ensure the file location calculation is correct so this isn't necessary
            if path.parent_id.is_none() {
                bytes.insert_many(usize::MIN, usize::MAX, *id);
                rows.insert_many(usize::MIN, usize::MAX, *id);
            } else {
                bytes.insert_many(location.start_byte(), location.end_byte(), *id);
                rows.insert_many(location.start_row(), location.end_row(), *id);
            }
        }

        Self { ids, bytes, rows }
    }

    fn ids(&self) -> impl IntoIterator<Item = TagId> + '_ {
        self.ids.iter().map(|id| *id)
    }

    fn find_id(&self, position: PartialPosition) -> Option<TagId> {
        match position {
            PartialPosition::Byte(byte) => self.bytes.get(byte),
            PartialPosition::Row(row) => self.rows.get(row),
            PartialPosition::Whole(whole) => self.bytes.get(whole.byte),
        }
    }

    fn find_ids(&self, span: PartialSpan) -> Counter<TagId> {
        match span {
            PartialSpan::Byte(start, end) => self.bytes.get_overlaps(start, end),
            PartialSpan::Row(start, end) => self.rows.get_overlaps(start, end),
            PartialSpan::Whole(whole) => self.bytes.get_overlaps(whole.start.byte, whole.end.byte),
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

pub struct Tagger<'a> {
    language: Language,
    query: &'a Query,
    kinds: Vec<Option<EntityKind>>,
    name: u32,
}

impl<'a> Tagger<'a> {
    pub fn new(language: Language, query: &'a Query) -> Self {
        let name = query.capture_index_for_name("name").unwrap();

        let kinds = query
            .capture_names()
            .iter()
            .map(|c| c.strip_prefix("tag.").map(|k| EntityKind::try_from(k).unwrap()))
            .collect::<Vec<_>>();

        Self { language, query, kinds, name }
    }

    pub fn extract(&self, source: &[u8], filename: &str) -> Result<TagSet> {
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
                    builder.ancestor_ids(collect_ancestor_ids(&capture.node));
                    builder.kind(kind);
                    builder.location(Span::from_ts(&capture.node.range()));
                }
            }

            let capture = builder.build()?;
            captures.insert(capture.id, capture);
        }

        Ok(into_file_entity_info(captures))
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

fn to_singleton_entity_set(source: &[u8], filename: &str) -> Result<TagSet> {
    // I doubt this location calculation is correct...
    let text = String::from_utf8(source.to_vec())?;
    let mut newlines = Vec::new();

    for (byte, c) in text.char_indices() {
        if c == '\n' {
            newlines.push(byte);
        }
    }

    let end_byte = source.len(); // or source.len() - 1?
    let end_row = newlines.len();
    let preceeding_length = newlines.last().map(|last| last + 1).unwrap_or(0);
    let end_column = source.len() - preceeding_length;
    let end_position = Position::new(end_byte, end_row, end_column);
    let capture = Capture::singleton(filename, end_position);

    let mut captures = HashMap::new();
    captures.insert(capture.id, capture);
    Ok(into_file_entity_info(captures))
}

fn into_file_entity_info(captures: HashMap<CaptureId, Capture>) -> TagSet {
    let mut tags = Vec::with_capacity(captures.len());
    let mut entity_ids = HashMap::with_capacity(captures.len());
    let mut tag_ids = HashMap::with_capacity(captures.len());
    let capture_ids = captures.keys().map(|&k| k).collect::<HashSet<_>>();

    for capture in captures.into_values().sorted_by_cached_key(|c| c.topo_key()) {
        let parent_capture_id = capture.find_parent_id(&capture_ids);

        let parent_entity_id = parent_capture_id.map(|id| *entity_ids.get(&id).unwrap());
        let entity = Entity::new(parent_entity_id, capture.name, capture.kind);
        entity_ids.insert(capture.id, entity.id);

        let parent_tag_id = parent_capture_id.map(|id| *tag_ids.get(&id).unwrap());
        let tag = Tag::new(parent_tag_id, entity, capture.location);
        tag_ids.insert(capture.id, tag.id);

        tags.push(tag);
    }

    TagSet::from_topo_vec(tags)
}
