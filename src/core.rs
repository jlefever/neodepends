pub type Oid = git2::Oid;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
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
