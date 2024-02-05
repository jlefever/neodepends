use std::collections::HashMap;

use anyhow::Context;
use anyhow::Result;
use derive_new::new;
use itertools::Itertools;
use lazy_static::lazy_static;
use sha1::Digest;
use sha1::Sha1;
use strum_macros::AsRefStr;
use strum_macros::EnumString;
use tree_sitter::Language;
use tree_sitter::Node;
use tree_sitter::Parser;
use tree_sitter::Query;
use tree_sitter::QueryCursor;
use tree_sitter::Range;

use crate::languages::Lang;
use crate::sparse_vec::SparseVec;

lazy_static! {
    static ref JAVA_EXTRACTOR: EntityExtractor = EntityExtractor::new(
        Lang::Java.sg_config().language,
        include_str!("../languages/java/tags.scm"),
        |text| { JavaEntityKind::try_from(text).ok().map(LangSpecificEntityKind::Java) }
    );
}

pub fn get_entity_extractor(lang: Lang) -> Option<&'static EntityExtractor> {
    match lang {
        Lang::Java => Some(&JAVA_EXTRACTOR),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    File,
    LangSpecific(LangSpecificEntityKind),
}

impl EntityKind {
    pub fn as_str(&self) -> &str {
        match self {
            EntityKind::File => "file",
            EntityKind::LangSpecific(e) => e.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangSpecificEntityKind {
    Java(JavaEntityKind),
}

impl LangSpecificEntityKind {
    pub fn as_str(&self) -> &str {
        match self {
            LangSpecificEntityKind::Java(e) => e.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum JavaEntityKind {
    Annotation,
    Constructor,
    Class,
    Enum,
    Field,
    File,
    Interface,
    Method,
    Record,
}

impl From<JavaEntityKind> for LangSpecificEntityKind {
    fn from(value: JavaEntityKind) -> Self {
        LangSpecificEntityKind::Java(value)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId([u8; 20]);

impl EntityId {
    pub fn from(name: &str, kind: EntityKind, parent_id: Option<EntityId>) -> Self {
        let mut hasher = Sha1::new();
        parent_id.map(|p| hasher.update(p.as_bytes()));
        hasher.update(name.as_bytes());
        hasher.update(kind.as_str().as_bytes());
        Self(hasher.finalize().into())
    }

    pub fn to_string(&self) -> String {
        hex::encode(self.0)   
    }

    pub fn to_short_string(&self) -> String {
        let text = self.to_string();
        text[33..text.len()].to_string()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> [u8; 20] {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, new)]
pub struct Entity {
    pub id: EntityId,
    pub parent_id: Option<EntityId>,
    pub name: String,
    pub kind: EntityKind,
    pub range: Range, // ???
}

pub struct EntityContainer {
    pub entities: Vec<Entity>,
    pub locations: SparseVec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Builder)]
struct Tag {
    id: usize,
    parent_id: Option<usize>,
    ancestor_ids: Vec<usize>,
    name: String,
    kind: EntityKind,
    range: Range,
}

impl Tag {
    fn file(id: usize, name: String, range: Range) -> Self {
        Self { id, parent_id: None, ancestor_ids: vec![], name, kind: EntityKind::File, range }
    }
}

pub struct EntityExtractor {
    language: Language,
    query: Query,
    kinds: Vec<Option<LangSpecificEntityKind>>,
    name: u32,
}

impl EntityExtractor {
    pub fn new<F>(language: Language, query: &str, parse_kind: F) -> Self
    where
        F: Fn(&str) -> Option<LangSpecificEntityKind>,
    {
        let query = Query::new(language, query.as_ref()).context("failed to parse query").unwrap();

        let name = query.capture_index_for_name("name").unwrap();

        let kinds = query
            .capture_names()
            .iter()
            .map(|c| c.strip_prefix("tag.").map(|x| parse_kind(x).unwrap()))
            .collect::<Vec<_>>();

        Self { language, query, kinds, name }
    }

    pub fn extract(&self, source: &[u8], filename: &str) -> Result<EntityContainer> {
        let mut parser = Parser::new();
        parser.set_language(self.language)?;
        let tree = parser.parse(source, None).context("failed to parse")?;
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();

        let mut tags = HashMap::new();
        tags.insert(root.id(), Tag::file(root.id(), filename.to_string(), root.range()));

        // Create a tag for each match
        for r#match in cursor.matches(&self.query, root, source) {
            let mut builder: TagBuilder = TagBuilder::default();
            builder.parent_id(None);

            for capture in r#match.captures {
                if capture.index == self.name {
                    builder.name(capture.node.utf8_text(source).unwrap().to_string());
                } else if let Some(tag_kind) = &self.kinds[capture.index as usize] {
                    builder.id(capture.node.id());
                    builder.kind(EntityKind::LangSpecific(tag_kind.clone()));
                    builder.range(capture.node.range());
                    builder.ancestor_ids(Self::get_ancestor_ids(&capture.node));
                }
            }

            let tag = builder.build()?;
            tags.insert(tag.id, tag);
        }

        // Find the parent for each tag
        let mut parentage = Vec::with_capacity(tags.len());

        for tag in tags.values() {
            for ancestor_id in &tag.ancestor_ids {
                if let Some(ancestor) = tags.get(ancestor_id) {
                    parentage.push((tag.id, ancestor.id));
                    break;
                }
            }
        }

        for (child_id, parent_id) in parentage {
            tags.get_mut(&child_id).unwrap().parent_id = Some(parent_id);
        }

        Ok(Self::to_file_entity_data(tags))
    }

    fn to_file_entity_data(tags: HashMap<usize, Tag>) -> EntityContainer {
        // Calculate an EntityId for each tag
        let mut ids = HashMap::new();

        for tag in tags.values().sorted_by_key(|t| t.ancestor_ids.len()) {
            let parent_id = tag.parent_id.map(|p| *ids.get(&p).unwrap());
            ids.insert(tag.id, EntityId::from(&tag.name, tag.kind, parent_id));
        }

        // Set up locations
        let mut locations = SparseVec::new();

        for tag in tags.values() {
            let Range { start_byte, end_byte, .. } = tag.range;
            locations.insert_many(start_byte, end_byte, *ids.get(&tag.id).unwrap());
        }

        // Convert to entities
        let entities = tags
            .into_values()
            .sorted_by_key(|t| (t.ancestor_ids.len(), t.range.start_byte))
            .map(|t| {
                let id = *ids.get(&t.id).unwrap();
                let parent_id = t.parent_id.map(|p| *ids.get(&p).unwrap());
                Entity::new(id, parent_id, t.name, t.kind, t.range)
            })
            .collect::<Vec<_>>();

        EntityContainer { entities, locations }
    }

    fn get_ancestor_ids(node: &Node) -> Vec<usize> {
        let mut ids = Vec::new();
        let mut curr: Option<Node> = node.parent();

        while let Some(curr_node) = curr {
            ids.push(curr_node.id());
            curr = curr_node.parent();
        }

        ids
    }
}
