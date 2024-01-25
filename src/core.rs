use anyhow::Result;

pub type Oid = git2::Oid;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct FileKey {
    pub filename: String,
    pub content_hash: Oid,
}

impl FileKey {
    pub fn new(filename: String, content_hash: Oid) -> Self {
        Self {
            filename,
            content_hash,
        }
    }

    pub fn from_string(filename: String, content_hash: String) -> anyhow::Result<Self> {
        Ok(Self::new(filename, Oid::from_str(&content_hash)?))
    }
}

impl std::fmt::Display for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.filename, self.content_hash)
    }
}

pub fn hash(content: &[u8]) -> Result<Oid> {
    Ok(Oid::hash_object(git2::ObjectType::Blob, content)?)
}

pub trait FileSource {
    /// Discover all files in this file source.
    fn discover(&self) -> Result<Vec<FileKey>>;

    /// Load the content of a file given a file key. The key must be from the
    /// `discover` method on the same file source.
    fn load(&self, key: &FileKey) -> Result<Vec<u8>>;
}
