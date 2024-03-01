use std::fmt::Display;
use std::fmt::Formatter;

use anyhow::bail;
use anyhow::Result;
use serde::Serialize;
use serde::Serializer;
use sha1::Digest;
use sha1::Sha1;

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl Serialize for ContentId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(Sha1Hash);

impl EntityId {
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

impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
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

    pub fn from_strings(filename: String, content_id: String) -> Result<Self> {
        Ok(Self::new(filename, ContentId::from_str(&content_id)?))
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
            start_byte: to_utf8_byte_index(&span.start),
            end_byte: to_utf8_byte_index(&span.end),
            start_row: span.start.as_point().row,
            end_row: span.end.as_point().row,
            start_column: span.start.as_point().column,
            end_column: span.end.as_point().column,
        }
    }
}

fn to_utf8_byte_index(position: &lsp_positions::Position) -> usize {
    position.containing_line.start + position.column.utf8_offset
}
