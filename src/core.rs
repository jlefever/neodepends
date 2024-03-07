use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Display;
use std::fmt::Formatter;

use anyhow::bail;
use anyhow::Result;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;
use sha1::Digest;
use sha1::Sha1;
use strum_macros::AsRefStr;
use strum_macros::EnumString;

use crate::sparse_vec::SparseVec;

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
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Deserialize,
    Serialize,
    EnumString,
    AsRefStr,
)]
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
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Deserialize,
    Serialize,
    EnumString,
    AsRefStr,
)]
pub enum DepKind {
    Annotation,
    Call,
    Cast,
    Contain,
    Create,
    Dependency,
    Extend,
    Implement,
    Import,
    Link,
    MixIn,
    Parameter,
    Parent,
    Plugin,
    Receive,
    Return,
    Set,
    Throw,
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

    pub fn to_oid(&self) -> git2::Oid {
        git2::Oid::from_bytes(self.as_bytes()).unwrap()
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
    pub fn new(parent_id: Option<EntityId>, name: &str, kind: EntityKind, location: Span) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().as_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(kind.as_str().as_bytes());

        fn to_bytes(num: usize) -> [u8; 4] {
            unsafe { std::mem::transmute(u32::try_from(num).unwrap().to_be()) }
        }

        bytes.extend(to_bytes(location.start_byte()));
        bytes.extend(to_bytes(location.start_row()));
        bytes.extend(to_bytes(location.start_column()));
        bytes.extend(to_bytes(location.end_byte()));
        bytes.extend(to_bytes(location.end_row()));
        bytes.extend(to_bytes(location.end_column()));
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct StableId(Sha1Hash);

impl StableId {
    pub fn new(parent_id: Option<StableId>, name: &str, kind: EntityKind) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().as_bytes());
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
    pub stable_id: StableId,
    pub name: String,
    pub kind: EntityKind,
    pub location: Span,
}

impl Entity {
    pub fn new(
        id: EntityId,
        parent_id: Option<EntityId>,
        stable_id: StableId,
        name: String,
        kind: EntityKind,
        location: Span,
    ) -> Self {
        Self { id, parent_id, stable_id, name, kind, location }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Position {
    pub byte: usize,
    pub row: usize,
    pub column: usize,
}

impl Position {
    pub fn new(byte: usize, row: usize, column: usize) -> Self {
        Self { byte, row, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn from_ts(range: &tree_sitter::Range) -> Self {
        let &tree_sitter::Range { start_byte, end_byte, start_point, end_point } = range;
        let start = Position::new(start_byte, start_point.row, start_point.column);
        let end = Position::new(end_byte, end_point.row, end_point.column);
        Self::new(start, end)
    }

    pub fn from_lsp(span: &lsp_positions::Span) -> Self {
        fn to_utf8_byte_index(position: &lsp_positions::Position) -> usize {
            position.containing_line.start + position.column.utf8_offset
        }
        let start = Position::new(
            to_utf8_byte_index(&span.start),
            span.start.as_point().row,
            span.start.as_point().column,
        );
        let end = Position::new(
            to_utf8_byte_index(&span.start),
            span.start.as_point().row,
            span.start.as_point().column,
        );
        Self::new(start, end)
    }

    pub fn start_byte(self) -> usize {
        self.start.byte
    }

    pub fn start_row(self) -> usize {
        self.start.row
    }

    pub fn start_column(self) -> usize {
        self.start.column
    }

    pub fn end_byte(self) -> usize {
        self.end.byte
    }

    pub fn end_row(self) -> usize {
        self.end.row
    }

    pub fn end_column(self) -> usize {
        self.end.column
    }
}

impl PartialOrd for Span {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(match self.start.cmp(&other.start) {
            Ordering::Less => Ordering::Less,
            Ordering::Equal => self.end.cmp(&other.start).reverse(),
            Ordering::Greater => Ordering::Greater,
        })
    }
}

impl Ord for Span {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum PartialPosition {
    Byte(usize),
    Row(usize),
    Whole(Position),
}

impl PartialPosition {
    pub fn byte(self) -> Option<usize> {
        match self {
            PartialPosition::Byte(byte) => Some(byte),
            PartialPosition::Row(_) => None,
            PartialPosition::Whole(whole) => Some(whole.byte),
        }
    }

    pub fn row(self) -> Option<usize> {
        match self {
            PartialPosition::Byte(_) => None,
            PartialPosition::Row(row) => Some(row),
            PartialPosition::Whole(whole) => Some(whole.row),
        }
    }

    pub fn column(self) -> Option<usize> {
        match self {
            PartialPosition::Byte(_) => None,
            PartialPosition::Row(_) => None,
            PartialPosition::Whole(whole) => Some(whole.column),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum PartialSpan {
    Byte(usize, usize),
    Row(usize, usize),
    Whole(Span),
}

impl PartialSpan {
    pub fn start(self) -> PartialPosition {
        match self {
            PartialSpan::Byte(start, _) => PartialPosition::Byte(start),
            PartialSpan::Row(start, _) => PartialPosition::Row(start),
            PartialSpan::Whole(whole) => PartialPosition::Whole(whole.start),
        }
    }

    pub fn end(self) -> PartialPosition {
        match self {
            PartialSpan::Byte(_, end) => PartialPosition::Byte(end),
            PartialSpan::Row(_, end) => PartialPosition::Row(end),
            PartialSpan::Whole(whole) => PartialPosition::Whole(whole.end),
        }
    }

    pub fn start_byte(self) -> Option<usize> {
        self.start().byte()
    }

    pub fn start_row(self) -> Option<usize> {
        self.start().row()
    }

    pub fn start_column(self) -> Option<usize> {
        self.start().column()
    }

    pub fn end_byte(self) -> Option<usize> {
        self.end().byte()
    }

    pub fn end_row(self) -> Option<usize> {
        self.end().row()
    }

    pub fn end_column(self) -> Option<usize> {
        self.end().column()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Dep<E> {
    pub src: E,
    pub tgt: E,
    pub kind: DepKind,
    pub position: PartialPosition,
}

impl<E> Dep<E> {
    pub fn new(src: E, tgt: E, kind: DepKind, position: PartialPosition) -> Self {
        Self { src, tgt, kind, position }
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
    pub position: PartialPosition,
}

impl FileEndpoint {
    pub fn new(filename: String, position: PartialPosition) -> Self {
        Self { filename, position }
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
