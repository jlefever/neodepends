use std::cmp::Ordering;
use std::fmt::Display;
use std::fmt::Formatter;

use anyhow::bail;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;
use sha1::Digest;
use sha1::Sha1;
use strum_macros::AsRefStr;
use strum_macros::EnumString;

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
    #[allow(dead_code)]
    pub fn new(arr: [u8; 20]) -> Self {
        Self(arr)
    }

    pub fn hash(bytes: &[u8]) -> Self {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
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
    #[allow(dead_code)]
    pub fn from(hash: Sha1Hash) -> Self {
        Self(hash)
    }

    #[allow(dead_code)]
    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn to_oid(&self) -> git2::Oid {
        git2::Oid::from_bytes(self.as_bytes()).unwrap()
    }
}

impl From<git2::Oid> for ContentId {
    fn from(value: git2::Oid) -> Self {
        unsafe { std::mem::transmute(value) }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CommitId(Sha1Hash);

impl CommitId {
    #[allow(dead_code)]
    pub fn from(hash: Sha1Hash) -> Self {
        Self(hash)
    }

    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn to_oid(&self) -> git2::Oid {
        git2::Oid::from_bytes(self.as_bytes()).unwrap()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct EntityId(Sha1Hash);

impl EntityId {
    pub fn new(parent_id: Option<EntityId>, name: &str, kind: EntityKind) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().as_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(kind.as_str().as_bytes());
        Self::from(Sha1Hash::hash(&bytes))
    }

    pub fn from(hash: Sha1Hash) -> Self {
        Self(hash)
    }

    #[allow(dead_code)]
    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
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
    pub fn new(parent_id: Option<EntityId>, name: String, kind: EntityKind) -> Self {
        let id = EntityId::new(parent_id, &name, kind);
        Self { id, parent_id, name, kind }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct TagId(Sha1Hash);

impl TagId {
    pub fn new(parent_id: Option<TagId>, entity_id: EntityId, location: Span) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().as_bytes());
        bytes.extend(entity_id.as_bytes());

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

    #[allow(dead_code)]
    pub fn from_str<S: AsRef<str>>(str: S) -> Result<Self> {
        Ok(Self(Sha1Hash::from_str(str)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Tag {
    pub id: TagId,
    pub parent_id: Option<TagId>,
    pub entity: Entity,
    pub location: Span,
}

impl Tag {
    pub fn new(parent_id: Option<TagId>, entity: Entity, location: Span) -> Self {
        let id = TagId::new(parent_id, entity.id, location);
        Self { id, parent_id, entity, location }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum PartialSpan {
    Byte(usize, usize),
    Row(usize, usize),
    Whole(Span),
}

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
    pub file_key: FileKey,
    pub position: PartialPosition,
}

impl FileEndpoint {
    pub fn new(file_key: FileKey, position: PartialPosition) -> Self {
        Self { file_key, position }
    }
}

pub type FileDep = Dep<FileEndpoint>;

pub type TagDep = Dep<TagId>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, AsRefStr)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Change<T> {
    pub target_id: T,
    pub commit_id: CommitId,
    pub kind: ChangeKind,
    pub adds: usize,
    pub dels: usize,
}

impl<T> Change<T> {
    pub fn new(
        target_id: T,
        commit_id: CommitId,
        kind: ChangeKind,
        adds: usize,
        dels: usize,
    ) -> Self {
        Self { target_id, commit_id, kind, adds, dels }
    }

    pub fn with_id<U>(&self, target_id: U) -> Change<U> {
        Change::<U>::new(target_id, self.commit_id, self.kind, self.adds, self.dels)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hunk {
    pub old: PartialSpan,
    pub new: PartialSpan,
}

impl Hunk {
    pub fn from_git(hunk: &git2::DiffHunk) -> Self {
        let old_start = usize::try_from(hunk.old_start()).unwrap() - 1;
        let old_end = old_start + usize::try_from(hunk.old_lines()).unwrap();
        let old = PartialSpan::Row(old_start, old_end);
        let new_start = usize::try_from(hunk.new_start()).unwrap() - 1;
        let new_end = new_start + usize::try_from(hunk.new_lines()).unwrap();
        let new = PartialSpan::Row(new_start, new_end);
        Self { old, new }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Diff {
    pub commit_id: CommitId,
    pub old: Option<FileKey>,
    pub new: Option<FileKey>,
    pub hunks: Vec<Hunk>,
}

impl Diff {
    pub fn deleted(commit_id: CommitId, old: FileKey) -> Self {
        Self { commit_id, old: Some(old), new: None, hunks: Vec::new() }
    }

    pub fn added(commit_id: CommitId, new: FileKey) -> Self {
        Self { commit_id, old: None, new: Some(new), hunks: Vec::new() }
    }

    pub fn modified(commit_id: CommitId, old: FileKey, new: FileKey, hunks: Vec<Hunk>) -> Self {
        Self { commit_id, old: Some(old), new: Some(new), hunks }
    }

    pub fn change_kind(&self) -> ChangeKind {
        match (&self.old, &self.new) {
            (None, None) => panic!(),
            (None, Some(_)) => ChangeKind::Added,
            (Some(_), None) => ChangeKind::Deleted,
            (Some(_), Some(_)) => ChangeKind::Modified,
        }
    }
}
