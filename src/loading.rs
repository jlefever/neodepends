use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use git2::ObjectType;
use git2::Oid;
use git2::Repository;
use git2::TreeWalkMode;
use git2::TreeWalkResult;
use walkdir::WalkDir;

use crate::core::ContentId;
use crate::core::FileKey;
use crate::languages::Lang;

#[derive(Debug, Clone)]
pub enum FileFilter {
    ByLang(HashSet<Lang>),
    ByFilename(HashSet<String>),
}

impl FileFilter {
    pub fn from_langs<I: IntoIterator<Item = Lang>>(langs: I) -> Self {
        Self::ByLang(langs.into_iter().collect())
    }

    pub fn from_filenames<I: IntoIterator<Item = String>>(filenames: I) -> Self {
        Self::ByFilename(filenames.into_iter().collect())
    }

    pub fn includes<S: AsRef<str>>(&self, filename: S) -> bool {
        match self {
            FileFilter::ByLang(langs) => {
                Lang::from_filename(filename).map(|l| langs.contains(&l)).unwrap_or(false)
            }
            FileFilter::ByFilename(filenames) => filenames.contains(filename.as_ref()),
        }
    }
}

#[derive(Clone)]
pub struct FileSystem {
    inner: FileSystemInner,
    file_keys: FileKeySet,
}

impl FileSystem {
    fn disk<P: AsRef<Path>>(root: P, filter: &FileFilter) -> Result<Self> {
        let inner = FileSystemInner::Disk(DiskStorage::new(root));
        let file_keys = FileKeySet::new(inner.list(filter)?)?;
        Ok(Self { inner, file_keys })
    }

    fn git<S: AsRef<str>>(repo: Repository, commit: S, filter: &FileFilter) -> Result<Self> {
        let inner = FileSystemInner::Git(GitStorage::new(repo), commit.as_ref().to_string());
        let file_keys = FileKeySet::new(inner.list(filter)?)?;
        Ok(Self { inner, file_keys })
    }

    pub fn open<P, S>(root: P, commit: &Option<S>, filter: &FileFilter) -> Result<Self>
    where
        P: AsRef<Path>,
        S: AsRef<str>,
    {
        let repo = Repository::open(&root).ok();
        let msg = "a commit was supplied but the project root does not refer to a git repository";
        match (repo, commit.as_ref()) {
            (None, None) => Self::disk(root, filter),
            (None, Some(_)) => bail!(msg),
            (Some(_), None) => Self::disk(root, filter),
            (Some(repo), Some(commit)) => Self::git(repo, commit, filter),
        }
    }

    pub fn repo(&self) -> Option<Arc<Mutex<Repository>>> {
        match &self.inner {
            FileSystemInner::Disk(_) => None,
            FileSystemInner::Git(git, _) => Some(git.repo.clone())
        }
    }

    pub fn list(&self) -> &[FileKey] {
        self.file_keys.file_keys()
    }

    pub fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        self.inner.load(key)
    }

    pub fn load_into_buf(&self, key: &FileKey, buf: &mut Vec<u8>) -> Result<usize> {
        self.inner.load_into_buf(key, buf)
    }

    #[allow(dead_code)]
    pub fn load_by_filename<F: AsRef<str>>(&self, filename: F) -> Result<Vec<u8>> {
        self.load(self.get_key_for_filename(filename)?)
    }

    pub fn get_key_for_filename<F: AsRef<str>>(&self, filename: F) -> Result<&FileKey> {
        self.file_keys.get_by_filename(&filename).with_context(|| {
            format!("no file named '{}' found in this filesystem", filename.as_ref())
        })
    }
}

#[derive(Clone)]
enum FileSystemInner {
    Disk(DiskStorage),
    Git(GitStorage, String),
}

impl FileSystemInner {
    fn list(&self, filter: &FileFilter) -> Result<Vec<FileKey>> {
        match self {
            FileSystemInner::Disk(fs) => fs.list(filter),
            FileSystemInner::Git(fs, commit) => fs.list(commit, filter),
        }
    }

    fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        match self {
            FileSystemInner::Disk(fs) => fs.load(&key.filename),
            FileSystemInner::Git(fs, _) => fs.load(&key.content_id),
        }
    }

    fn load_into_buf(&self, key: &FileKey, buf: &mut Vec<u8>) -> Result<usize> {
        match self {
            FileSystemInner::Disk(fs) => fs.load_into_buf(&key.filename, buf),
            FileSystemInner::Git(fs, _) => fs.load_into_buf(&key.content_id, buf),
        }
    }
}

#[derive(Clone)]
struct DiskStorage {
    root: PathBuf,
}

impl DiskStorage {
    fn new<P: AsRef<Path>>(root: P) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }

    fn list(&self, filter: &FileFilter) -> Result<Vec<FileKey>> {
        let mut keys = Vec::new();

        for entry in WalkDir::new(&self.root).follow_links(true) {
            match entry {
                Ok(entry) => {
                    let path = entry.path().strip_prefix(&self.root)?.to_string_lossy().to_string();

                    if filter.includes(&path) {
                        let oid = Oid::hash_file(ObjectType::Blob, &path)?;
                        keys.push(FileKey::new(path, oid.into()));
                    }
                }
                Err(err) => {
                    log::warn!("Failed to read directory entry: {}. Skipping...", err);
                }
            }
        }

        Ok(keys)
    }

    fn load<F: AsRef<Path>>(&self, filename: F) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.load_into_buf(filename, &mut buf)?;
        Ok(buf)
    }

    fn load_into_buf<F: AsRef<Path>>(&self, filename: F, buf: &mut Vec<u8>) -> Result<usize> {
        Ok(File::open(self.root.join(filename))?.read_to_end(buf)?)
    }
}

#[derive(Clone)]
struct GitStorage {
    repo: Arc<Mutex<Repository>>,
}

impl GitStorage {
    fn new(repo: Repository) -> Self {
        // This is a necessary config for Windows
        repo.config().unwrap().set_bool("core.longpaths", true).unwrap();
        Self { repo: Arc::new(Mutex::new(repo)) }
    }

    fn list<S: AsRef<str>>(&self, commit: S, filter: &FileFilter) -> Result<Vec<FileKey>> {
        let mut keys = Vec::new();

        let repo = self.repo.lock().unwrap();
        let reference = repo.resolve_reference_from_short_name(commit.as_ref());

        let commit = if let Ok(reference) = reference {
            reference.peel_to_commit()?
        } else if let Ok(oid) = Oid::from_str(commit.as_ref()) {
            repo.find_commit(oid)?
        } else {
            bail!("the given commit ('{}') was not found in this repository", commit.as_ref());
        };

        commit.tree()?.walk(TreeWalkMode::PreOrder, |dir, entry| {
            let path = dir.to_string() + entry.name().unwrap();

            if filter.includes(&path) {
                keys.push(FileKey::new(path, entry.id().into()));
            }

            TreeWalkResult::Ok
        })?;

        Ok(keys)
    }

    fn load(&self, blob_id: &ContentId) -> Result<Vec<u8>> {
        Ok(self.repo.lock().unwrap().find_blob(blob_id.to_oid())?.content().to_owned())
    }

    fn load_into_buf(&self, blob_id: &ContentId, buf: &mut Vec<u8>) -> Result<usize> {
        let repo = self.repo.try_lock().unwrap();
        let blob = repo.find_blob(blob_id.to_oid())?;
        let slice = blob.content();
        buf.extend_from_slice(slice);
        Ok(slice.len())
    }
}

#[derive(Clone)]
struct FileKeySet {
    file_keys: Vec<FileKey>,
    filenames: HashMap<String, usize>,
}

impl FileKeySet {
    fn new(mut file_keys: Vec<FileKey>) -> Result<Self> {
        let mut filenames = HashMap::with_capacity(file_keys.len());
        file_keys.sort();

        for (i, file_key) in file_keys.iter().enumerate() {
            if let Some(_) = filenames.insert(file_key.filename.clone(), i) {
                bail!("filenames must be unique");
            }
        }

        Ok(Self { file_keys, filenames })
    }

    fn file_keys(&self) -> &[FileKey] {
        &self.file_keys
    }

    fn get_by_filename<S: AsRef<str>>(&self, filename: S) -> Option<&FileKey> {
        self.filenames.get(filename.as_ref()).map(|&i| &self.file_keys[i])
    }
}
