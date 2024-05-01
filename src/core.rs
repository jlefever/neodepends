use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use anyhow::bail;
use anyhow::Result;
use itertools::Itertools;
use rusqlite::types::ToSqlOutput;
use rusqlite::ToSql;
use sha1::Digest;
use sha1::Sha1;

/// A 160 bit SHA-1 hash.
#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        self.into()
    }
}

impl AsRef<[u8]> for Sha1Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Into<String> for &Sha1Hash {
    fn into(self) -> String {
        hex::encode(self.0)
    }
}

impl serde::Serialize for Sha1Hash {
    fn serialize<S>(&self, serializer: S) -> std::prelude::v1::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<git2::Oid> for Sha1Hash {
    fn from(value: git2::Oid) -> Self {
        unsafe { std::mem::transmute(value) }
    }
}

impl From<Sha1Hash> for git2::Oid {
    fn from(value: Sha1Hash) -> Self {
        unsafe { std::mem::transmute(value) }
    }
}

impl Debug for Sha1Hash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Sha1Hash").field(&self.to_string()).finish()
    }
}

impl ToSql for Sha1Hash {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// The ID of a commit as identified by git.
///
/// Internally, git calculates this as a SHA-1 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct CommitId(pub Sha1Hash);

impl Display for CommitId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

impl From<git2::Oid> for CommitId {
    fn from(value: git2::Oid) -> Self {
        Self(value.into())
    }
}

impl From<CommitId> for git2::Oid {
    fn from(value: CommitId) -> Self {
        value.0.into()
    }
}

impl ToSql for CommitId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// Might refer to an actual commit or may refer to the project directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::EnumIs, strum::EnumTryAs)]
pub enum PseudoCommitId {
    CommitId(CommitId),
    WorkDir,
}

impl serde::Serialize for PseudoCommitId {
    fn serialize<S>(&self, serializer: S) -> std::prelude::v1::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PseudoCommitId::CommitId(commit_id) => commit_id.0.serialize(serializer),
            PseudoCommitId::WorkDir => serializer.serialize_str("WORKDIR"),
        }
    }
}

impl ToSql for PseudoCommitId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match self {
            PseudoCommitId::CommitId(c) => c.to_sql(),
            PseudoCommitId::WorkDir => Ok(ToSqlOutput::Owned(rusqlite::types::Value::Null)),
        }
    }
}

/// The SHA-1 hash of the content of a file.
///
/// This is exactly how git calculates the ID of a blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct ContentId(pub Sha1Hash);

impl ContentId {
    pub fn from_path<P: AsRef<Path>>(path: P) -> ContentId {
        git2::Oid::hash_file(git2::ObjectType::Blob, path).unwrap().into()
    }

    pub fn from_content(content: &str) -> ContentId {
        git2::Oid::hash_object(git2::ObjectType::Blob, content.as_bytes()).unwrap().into()
    }
}

impl From<git2::Oid> for ContentId {
    fn from(value: git2::Oid) -> Self {
        Self(value.into())
    }
}

impl From<ContentId> for git2::Oid {
    fn from(value: ContentId) -> Self {
        value.0.into()
    }
}

impl ToSql for ContentId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// A unique identifier for a particular version of a file.
///
/// Ordinarily, [ContentId] would be good enough to uniquely identify a file,
/// however, there are some cases where a filename is necessary. For instance,
/// entity extraction (the root entity depends on the filename), and dependency
/// resolution. The filename is simply the path of the file relative to the
/// project root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileKey {
    pub filename: String,
    pub content_id: ContentId,
}

impl FileKey {
    pub fn new(filename: String, content_id: ContentId) -> Self {
        Self { filename, content_id }
    }

    pub fn from_content(filename: String, content: &str) -> Self {
        Self { filename, content_id: ContentId::from_content(content) }
    }
}

/// A collection of [FileKey]s with unique [FileKey::filename]s.
///
/// Intended to represent the project at a particular version.
#[derive(Debug, Clone)]
pub struct FileSet {
    file_keys: Vec<FileKey>,
    filenames: HashMap<String, usize>,
    content_ids: HashMap<ContentId, Vec<usize>>,
}

impl FileSet {
    /// Create a [FileSet] from an iterator of [FileKey]s.
    ///
    /// Panics if there are any duplicate [FileKey::filename]s.
    pub fn new<I: IntoIterator<Item = FileKey>>(file_keys: I) -> Self {
        let file_keys = file_keys.into_iter().sorted().collect_vec();
        let mut filenames = HashMap::with_capacity(file_keys.len());
        let mut content_ids: HashMap<_, Vec<_>> = HashMap::with_capacity(file_keys.len());

        for (i, file_key) in file_keys.iter().enumerate() {
            if let Some(_) = filenames.insert(file_key.filename.clone(), i) {
                panic!("filenames must be unique");
            }

            content_ids.entry(file_key.content_id).or_default().push(i);
        }

        Self { file_keys, filenames, content_ids }
    }

    pub fn iter(&self) -> impl Iterator<Item = &FileKey> {
        self.file_keys.iter()
    }

    pub fn get<S: AsRef<str>>(&self, filename: S) -> Option<&FileKey> {
        self.filenames.get(filename.as_ref()).map(|&i| &self.file_keys[i])
    }

    pub fn get_content_id<S: AsRef<str>>(&self, filename: S) -> Option<ContentId> {
        self.get(filename).map(|f| f.content_id)
    }

    pub fn get_filenames(&self, content_id: ContentId) -> impl Iterator<Item = &str> {
        self.content_ids
            .get(&content_id)
            .into_iter()
            .flat_map(|x| x.iter().map(|&i| self.file_keys[i].filename.as_str()))
    }
}

/// A collection of [FileSet]s.
///
/// Each `FileSet` is at a different version of the project. A version is
/// represented using [PseudoCommitId].
#[derive(Debug, Clone)]
pub struct MultiFileSet {
    files: HashSet<FileKey>,
    file_sets: HashMap<PseudoCommitId, FileSet>,
}

impl MultiFileSet {
    /// Create a [MultiFileSet] from a mapping from `PseudoCommitId` to
    /// `FileSet`.
    pub fn new(file_sets: HashMap<PseudoCommitId, FileSet>) -> Self {
        let files = file_sets.values().flat_map(|x| x.iter()).cloned().collect();
        Self { files, file_sets }
    }

    pub fn files(&self) -> &HashSet<FileKey> {
        &self.files
    }

    pub fn into_files(self) -> HashSet<FileKey> {
        self.files
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PseudoCommitId, &FileSet)> {
        self.file_sets.iter()
    }
}

/// A position (i.e. index) within a file.
///
/// Specified both in terms of (row, column) and byte offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
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

/// An inclusive range of text within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(serde::Serialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn from_ts(value: tree_sitter::Range) -> Self {
        value.into()
    }

    pub fn from_lsp(value: &lsp_positions::Span) -> Self {
        value.into()
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

impl From<tree_sitter::Range> for Span {
    fn from(value: tree_sitter::Range) -> Self {
        let tree_sitter::Range { start_byte, end_byte, start_point, end_point } = value;
        let start = Position::new(start_byte, start_point.row, start_point.column);
        let end = Position::new(end_byte, end_point.row, end_point.column);
        Self::new(start, end)
    }
}

impl From<&lsp_positions::Span> for Span {
    fn from(value: &lsp_positions::Span) -> Self {
        fn to_utf8_byte_index(position: &lsp_positions::Position) -> usize {
            position.containing_line.start + position.column.utf8_offset
        }
        let start = Position::new(
            to_utf8_byte_index(&value.start),
            value.start.as_point().row,
            value.start.as_point().column,
        );
        let end = Position::new(
            to_utf8_byte_index(&value.start),
            value.start.as_point().row,
            value.start.as_point().column,
        );
        Self::new(start, end)
    }
}

/// Like [Position] but it is possible that only the row is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialPosition {
    Row(usize),
    Whole(Position),
}

impl PartialPosition {
    pub fn byte(&self) -> Option<usize> {
        match self {
            PartialPosition::Whole(w) => Some(w.byte),
            _ => None,
        }
    }

    pub fn row(&self) -> usize {
        match self {
            PartialPosition::Row(r) => *r,
            PartialPosition::Whole(w) => w.row,
        }
    }

    pub fn column(&self) -> Option<usize> {
        match self {
            PartialPosition::Whole(w) => Some(w.column),
            _ => None,
        }
    }
}

/// Like [Span] but it is possible that only the rows are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialSpan {
    Row(usize, usize),
    #[allow(dead_code)]
    Whole(Span),
}

impl PartialSpan {
    #[allow(dead_code)]
    pub fn start_byte(&self) -> Option<usize> {
        match self {
            PartialSpan::Whole(w) => Some(w.start.byte),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn start_row(&self) -> usize {
        match self {
            PartialSpan::Row(s, _) => *s,
            PartialSpan::Whole(w) => w.start.row,
        }
    }

    #[allow(dead_code)]
    pub fn start_column(&self) -> Option<usize> {
        match self {
            PartialSpan::Whole(w) => Some(w.start.column),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn end_byte(&self) -> Option<usize> {
        match self {
            PartialSpan::Whole(w) => Some(w.end.byte),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn end_row(&self) -> usize {
        match self {
            PartialSpan::Row(s, _) => *s,
            PartialSpan::Whole(w) => w.end.row,
        }
    }

    #[allow(dead_code)]
    pub fn end_column(&self) -> Option<usize> {
        match self {
            PartialSpan::Whole(w) => Some(w.end.column),
            _ => None,
        }
    }
}

/// A number representing the type of an [Entity].
///
/// Most languages only use a subset of these.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
#[derive(strum::AsRefStr, strum::EnumIs, strum::EnumString)]
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

impl ToSql for EntityKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

/// A "simpler" [EntityId] that is only calculated from `parent_id`, `name`, and
/// `kind`.
///
/// This is how we correlate entities from different versions. Unfortunately,
/// entities in the same version may sometimes re-use the same
/// `SimpleEntityId``. For instance, overloaded Java methods will all be given
/// the same `SimpleEntityId`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct SimpleEntityId(pub Sha1Hash);

impl SimpleEntityId {
    pub fn new(parent_id: Option<SimpleEntityId>, name: &str, kind: EntityKind) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().0.as_ref());
        bytes.extend(name.as_bytes());
        bytes.extend(kind.as_ref().as_bytes());
        Self(Sha1Hash::hash(&bytes))
    }
}

impl ToSql for SimpleEntityId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// A unique identifier for an [Entity].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct EntityId(pub Sha1Hash);

impl EntityId {
    pub fn new(
        parent_id: Option<EntityId>,
        name: &str,
        kind: EntityKind,
        location: Span,
        content_id: ContentId,
        simple_id: SimpleEntityId,
    ) -> Self {
        let mut bytes = Vec::new();
        bytes.extend(parent_id.unwrap_or_default().0.as_ref());
        bytes.extend(name.as_bytes());
        bytes.extend(kind.as_ref().as_bytes());

        fn to_bytes(num: usize) -> [u8; 4] {
            unsafe { std::mem::transmute(u32::try_from(num).unwrap().to_be()) }
        }

        bytes.extend(to_bytes(location.start.byte));
        bytes.extend(to_bytes(location.start.row));
        bytes.extend(to_bytes(location.start.column));
        bytes.extend(to_bytes(location.end.byte));
        bytes.extend(to_bytes(location.end.row));
        bytes.extend(to_bytes(location.end.column));
        bytes.extend(content_id.0.as_ref());
        bytes.extend(simple_id.0.as_ref());
        Self(Sha1Hash::hash(&bytes))
    }
}

impl ToSql for EntityId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// An interesting block of source code with a name.
///
/// Entities are discovered inside a file using
/// [tree-sitter](https://tree-sitter.github.io/tree-sitter/). An entity is at the root
/// (`parent_id.is_none() = true`) if and only if it is an [EntityKind::File].
/// Entities are also called "tags".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct Entity {
    pub id: EntityId,
    pub parent_id: Option<EntityId>,
    pub name: String,
    pub kind: EntityKind,
    pub location: Span,
    pub content_id: ContentId,
    pub simple_id: SimpleEntityId,
}

impl Entity {
    pub fn new(
        parent_id: Option<EntityId>,
        name: String,
        kind: EntityKind,
        location: Span,
        content_id: ContentId,
        simple_id: SimpleEntityId,
    ) -> Self {
        let id = EntityId::new(parent_id, &name, kind, location, content_id, simple_id);
        Self { id, parent_id, name, kind, location, content_id, simple_id }
    }
}

/// The content of a file
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct Content {
    pub id: ContentId,
    pub content: String,
}

impl Content {
    pub fn new(id: ContentId, content: String) -> Self {
        Self { id, content }
    }
}

/// A number representing the type of an [Dep].
///
/// Most languages use only a subset of these.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Deserialize, serde::Serialize)]
#[derive(strum::AsRefStr, strum::Display, strum::EnumIs, strum::EnumString)]
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

impl ToSql for DepKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

/// A syntactic dependency between two files.
///
/// [Self::position] refers to the location of the dependency within the source
/// file. For instance, the exact byte where a function is invoked.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct Dep<E> {
    pub src: E,
    pub tgt: E,
    pub kind: DepKind,
    pub position: PartialPosition,
    pub commit_id: PseudoCommitId,
}

impl<E> Dep<E> {
    pub fn new(
        src: E,
        tgt: E,
        kind: DepKind,
        position: PartialPosition,
        commit_id: PseudoCommitId,
    ) -> Self {
        Self { src, tgt, kind, position, commit_id }
    }
}

impl<E: Eq> Dep<E> {
    pub fn is_loop(&self) -> bool {
        self.src == self.tgt
    }
}

/// Intended to be used with [Dep] to represent a file-level dependency.
///
/// See [FileDep].
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

/// Intended to be used with [Dep] to represent a file-level dependency.
///
/// Similar to [FileEndpoint], but without having to know
/// [FileKey::content_id].
///
/// See [FileDep].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FilenameEndpoint {
    pub filename: String,
    pub position: PartialPosition,
}

impl FilenameEndpoint {
    pub fn new(filename: String, position: PartialPosition) -> Self {
        Self { filename, position }
    }

    pub fn into_file_endpoint(self, file_set: &FileSet) -> Option<FileEndpoint> {
        let content_id = file_set.get_content_id(&self.filename);
        content_id.map(|c| FileEndpoint::new(FileKey::new(self.filename, c), self.position))
    }
}

/// A dependency between files.
pub type FileDep = Dep<FileEndpoint>;

/// A dependency between filenames.
pub type FilenameDep = Dep<FilenameEndpoint>;

/// A dependency between entities.
pub type EntityDep = Dep<EntityId>;

impl FilenameDep {
    pub fn into_file_dep(self, file_set: &FileSet) -> Option<FileDep> {
        let src = self.src.into_file_endpoint(file_set)?;
        let tgt = self.tgt.into_file_endpoint(file_set)?;
        Some(Dep::new(src, tgt, self.kind, self.position, self.commit_id))
    }
}

/// A number representing the type of a [Change].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
#[derive(strum::AsRefStr, strum::EnumIs, strum::EnumString)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
}

impl ToSql for ChangeKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

/// A record of an [Entity] being touched by a commit.
///
/// The number of lines added and deleted are stored in [Self::adds] and
/// [Self::dels] respectively. Be careful not to count these more than once when
/// mapping [SimpleEntityId] back to [EntityId].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize)]
pub struct Change {
    pub simple_id: SimpleEntityId,
    pub commit_id: CommitId,
    pub kind: ChangeKind,
    pub adds: usize,
    pub dels: usize,
}

impl Change {
    pub fn new(
        simple_id: SimpleEntityId,
        commit_id: CommitId,
        kind: ChangeKind,
        adds: usize,
        dels: usize,
    ) -> Self {
        Self { simple_id, commit_id, kind, adds, dels }
    }
}

/// A record of a block of text that has been changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hunk {
    /// A block of text from the old version that has been deleted
    pub old: PartialSpan,

    /// A block of text from the new version that has been added
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

/// A collection of [Hunk]s for a particular file in a particular commit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Diff {
    pub commit_id: CommitId,
    pub old: Option<FileKey>,
    pub new: Option<FileKey>,
    pub hunks: Vec<Hunk>,
}

impl Diff {
    pub fn new(id: CommitId, old: Option<FileKey>, new: Option<FileKey>, hunks: Vec<Hunk>) -> Self {
        Self { commit_id: id, old, new, hunks }
    }

    pub fn change_kind(&self) -> ChangeKind {
        match (&self.old, &self.new) {
            (None, None) => panic!(),
            (None, Some(_)) => ChangeKind::Added,
            (Some(_), None) => ChangeKind::Deleted,
            (Some(_), Some(_)) => ChangeKind::Modified,
        }
    }

    pub fn iter_file_keys(&self) -> impl Iterator<Item = &FileKey> {
        self.old.as_ref().into_iter().chain(self.new.as_ref().into_iter())
    }

    pub fn iter_old_spans(&self) -> impl Iterator<Item = PartialSpan> + '_ {
        self.hunks.iter().map(|h| h.old)
    }

    pub fn iter_new_spans(&self) -> impl Iterator<Item = PartialSpan> + '_ {
        self.hunks.iter().map(|h| h.new)
    }
}
