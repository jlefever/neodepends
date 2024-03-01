use std::fmt::Display;
use std::fmt::Formatter;

use anyhow::bail;
use anyhow::Result;
use serde::Serialize;
use serde::Serializer;
use sha1::Digest;
use sha1::Sha1;
use strum_macros::AsRefStr;
use strum_macros::EnumString;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    Java,
    JavaScript,
    Python,
    TypeScript,
}

impl Lang {
    pub fn from_ext<S: AsRef<str>>(ext: S) -> Option<Lang> {
        match ext.as_ref().to_lowercase().as_ref() {
            "java" => Some(Lang::Java),
            "js" => Some(Lang::JavaScript),
            "py" => Some(Lang::Python),
            "ts" => Some(Lang::TypeScript),
            _ => None,
        }
    }

    pub fn from_filename<S: AsRef<str>>(filename: S) -> Option<Lang> {
        filename.as_ref().split(".").last().and_then(Self::from_ext)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, EnumString, AsRefStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum EntityKind {
    File,
    Annotation,
    Constructor,
    Class,
    Enum,
    Field,
    Interface,
    Method,
    Record,
}

impl EntityKind {
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}

impl Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, EnumString, AsRefStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum DepKind {
    Use,
}

impl DepKind {
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}

impl Display for DepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha1Hash([u8; 20]);

impl Sha1Hash {
    pub fn new(arr: [u8; 20]) -> Self {
        Self(arr)
    }

    pub fn hash(data: &[u8]) -> Self {
        let mut hasher = Sha1::new();
        hasher.update(data);
        Self(hasher.finalize().into())
    }

    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        if let Ok(arr) = TryInto::<[u8; 20]>::try_into(hex::decode(str.as_ref())?) {
            Ok(Sha1Hash(arr))
        } else {
            bail!("expected 20 digit hexadecimal string")
        }
    }

    pub fn to_string(&self) -> String {
        hex::encode(self.0)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[allow(dead_code)]
    pub fn into_bytes(self) -> [u8; 20] {
        self.0
    }
}

impl AsRef<[u8]> for Sha1Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Display for Sha1Hash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Serialize for Sha1Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ContentId(Sha1Hash);

impl ContentId {
    pub fn from(hash: Sha1Hash) -> Self {
        Self(hash)
    }

    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn into_bytes(self) -> [u8; 20] {
        self.0.into_bytes()
    }
}

impl Display for ContentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct EntityId(Sha1Hash);

impl EntityId {
    pub fn new(parent_id: Option<EntityId>, name: &str, kind: EntityKind) -> Self {
        let mut bytes = Vec::new();
        parent_id.map(|p| bytes.extend(p.as_bytes()));
        bytes.extend(name.as_bytes());
        bytes.extend(kind.as_str().as_bytes());
        Self::from(Sha1Hash::hash(&bytes))
    }

    pub fn from(hash: Sha1Hash) -> Self {
        Self(hash)
    }

    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn into_bytes(self) -> [u8; 20] {
        self.0.into_bytes()
    }
}

impl Display for EntityId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Entity {
    pub id: EntityId,
    pub parent_id: Option<EntityId>,
    pub name: String,
    pub kind: EntityKind,
}

impl Entity {
    pub fn new(id: EntityId, parent_id: Option<EntityId>, name: String, kind: EntityKind) -> Self {
        Self { id, parent_id, name, kind }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Dep<E> {
    pub src: E,
    pub tgt: E,
    pub kind: DepKind,
    pub byte: usize,
}

impl<E> Dep<E> {
    pub fn new(src: E, tgt: E, kind: DepKind, byte: usize) -> Self {
        Self { src, tgt, kind, byte }
    }
}

impl<E: Eq> Dep<E> {
    pub fn is_loop(&self) -> bool {
        self.src == self.tgt
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileEndpoint {
    pub filename: String,
    pub byte: usize,
}

impl FileEndpoint {
    pub fn new(filename: String, byte: usize) -> Self {
        Self { filename, byte }
    }
}

pub type FileDep = Dep<FileEndpoint>;

pub type EntityDep = Dep<EntityId>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileKey {
    pub filename: String,
    pub content_id: ContentId,
}

impl FileKey {
    pub fn new(filename: String, content_id: ContentId) -> Self {
        Self { filename, content_id }
    }

    pub fn to_sha1_hash(&self) -> Sha1Hash {
        let mut bytes = Vec::new();
        bytes.extend(self.filename.as_bytes());
        bytes.extend(self.content_id.as_bytes());
        Sha1Hash::hash(&bytes)
    }
}

impl Display for FileKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.content_id, self.filename)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Loc {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub end_row: usize,
    pub start_column: usize,
    pub end_column: usize,
}

impl Loc {
    pub fn from_range(range: &tree_sitter::Range) -> Self {
        Self {
            start_byte: range.start_byte,
            end_byte: range.end_byte,
            start_row: range.start_point.row,
            end_row: range.end_point.row,
            start_column: range.start_point.column,
            end_column: range.end_point.column,
        }
    }

    pub fn from_span(span: &lsp_positions::Span) -> Self {
        Self {
            start_byte: Self::to_utf8_byte_index(&span.start),
            end_byte: Self::to_utf8_byte_index(&span.end),
            start_row: span.start.as_point().row,
            end_row: span.end.as_point().row,
            start_column: span.start.as_point().column,
            end_column: span.end.as_point().column,
        }
    }

    fn to_utf8_byte_index(position: &lsp_positions::Position) -> usize {
        position.containing_line.start + position.column.utf8_offset
    }
}
