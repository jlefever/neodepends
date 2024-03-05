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
use crate::core::Lang;

#[derive(Clone)]
pub struct FileSystem {
    inner: FileSystemInner,
    file_keys: FileKeySet,
}

impl FileSystem {
    fn disk<P: AsRef<Path>>(root: P, langs: &HashSet<Lang>) -> Result<Self> {
        let inner = FileSystemInner::Disk(DiskStorage::new(root));
        let file_keys = FileKeySet::new(inner.list(langs)?)?;
        Ok(Self { inner, file_keys })
    }

    fn git<S: AsRef<str>>(repo: Repository, commit: S, langs: &HashSet<Lang>) -> Result<Self> {
        let inner = FileSystemInner::Git(GitStorage::new(repo), commit.as_ref().to_string());
        let file_keys = FileKeySet::new(inner.list(langs)?)?;
        Ok(Self { inner, file_keys })
    }

    pub fn open<P, S>(root: P, commit: &Option<S>, langs: &HashSet<Lang>) -> Result<Self>
    where
        P: AsRef<Path>,
        S: AsRef<str>,
    {
        let repo = Repository::open(&root).ok();
        let msg = "a commit was supplied but the project root does not refer to a git repository";
        match (repo, commit.as_ref()) {
            (None, None) => Self::disk(root, langs),
            (None, Some(_)) => bail!(msg),
            (Some(_), None) => Self::disk(root, langs),
            (Some(repo), Some(commit)) => Self::git(repo, commit, langs),
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

    pub fn get_by_filename<F: AsRef<str>>(&self, filename: F) -> Result<&FileKey> {
        self.file_keys.get_by_filename(&filename).with_context(|| {
            format!("no file named '{}' found in this filesystem", filename.as_ref())
        })
    }

    pub fn load_by_filename<F: AsRef<str>>(&self, filename: F) -> Result<Vec<u8>> {
        self.load(self.get_by_filename(filename)?)
    }

    fn get_by_content_id(&self, content_id: &ContentId) -> Result<&FileKey> {
        self.file_keys.get_by_content_id(content_id).with_context(|| {
            format!("no files with content id '{}' found in this filesystem", content_id)
        })
    }

    pub fn load_by_content_id(&self, content_id: &ContentId) -> Result<Vec<u8>> {
        self.load(self.get_by_content_id(content_id)?)
    }
}

#[derive(Clone)]
enum FileSystemInner {
    Disk(DiskStorage),
    Git(GitStorage, String),
}

impl FileSystemInner {
    fn list(&self, langs: &HashSet<Lang>) -> Result<Vec<FileKey>> {
        match self {
            FileSystemInner::Disk(fs) => fs.list(langs),
            FileSystemInner::Git(fs, commit) => fs.list(langs, commit),
        }
    }

    fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        match self {
            FileSystemInner::Disk(fs) => fs.load(&key.filename),
            FileSystemInner::Git(fs, _) => fs.load(&key.content_id),
        }
    }

    pub fn load_into_buf(&self, key: &FileKey, buf: &mut Vec<u8>) -> Result<usize> {
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

    fn list(&self, langs: &HashSet<Lang>) -> Result<Vec<FileKey>> {
        let mut keys = Vec::new();

        for entry in WalkDir::new(&self.root).follow_links(true) {
            match entry {
                Ok(entry) => {
                    let path = entry.path().strip_prefix(&self.root)?.to_string_lossy().to_string();

                    if is_lang_enabled(&path, langs) {
                        let oid = Oid::hash_file(ObjectType::Blob, &path)?;
                        keys.push(FileKey::new(path, to_content_id(oid)));
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

    pub fn load_into_buf<F: AsRef<Path>>(&self, filename: F, buf: &mut Vec<u8>) -> Result<usize> {
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

    fn list<S: AsRef<str>>(&self, langs: &HashSet<Lang>, commit: S) -> Result<Vec<FileKey>> {
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

            if is_lang_enabled(&path, langs) {
                keys.push(FileKey::new(path, to_content_id(entry.id())));
            }

            TreeWalkResult::Ok
        })?;

        Ok(keys)
    }

    fn load(&self, content_id: &ContentId) -> Result<Vec<u8>> {
        Ok(self.repo.try_lock().unwrap().find_blob(content_id.to_oid())?.content().to_owned())
    }

    fn load_into_buf(&self, content_id: &ContentId, buf: &mut Vec<u8>) -> Result<usize> {
        let repo = self.repo.try_lock().unwrap();
        let blob = repo.find_blob(content_id.to_oid())?;
        let slice = blob.content();
        buf.extend_from_slice(slice);
        Ok(slice.len())
    }
}

#[derive(Clone)]
struct FileKeySet {
    file_keys: Vec<FileKey>,
    filenames: HashMap<String, usize>,
    content_ids: HashMap<ContentId, Vec<usize>>,
}

impl FileKeySet {
    pub fn new(mut file_keys: Vec<FileKey>) -> Result<Self> {
        let mut filenames = HashMap::with_capacity(file_keys.len());
        let mut content_ids: HashMap<ContentId, Vec<usize>> =
            HashMap::with_capacity(file_keys.len());

        file_keys.sort();

        for (i, file_key) in file_keys.iter().enumerate() {
            if let Some(_) = filenames.insert(file_key.filename.clone(), i) {
                bail!("filenames must be unique");
            }

            if let Some(indices) = content_ids.get_mut(&file_key.content_id) {
                indices.push(i);
            } else {
                content_ids.insert(file_key.content_id, vec![i]);
            }
        }

        Ok(Self { file_keys, filenames, content_ids })
    }

    pub fn file_keys(&self) -> &[FileKey] {
        &self.file_keys
    }

    pub fn get_by_filename<S: AsRef<str>>(&self, filename: S) -> Option<&FileKey> {
        self.filenames.get(filename.as_ref()).map(|&i| &self.file_keys[i])
    }

    // Get an ARBITRARY FileKey with the given content_id
    pub fn get_by_content_id(&self, content_id: &ContentId) -> Option<&FileKey> {
        self.content_ids.get(content_id).map(|indices| &self.file_keys[indices[0]])
    }
}

fn is_lang_enabled(filename: &str, langs: &HashSet<Lang>) -> bool {
    Lang::from_filename(filename).map(|l| langs.contains(&l)).is_some()
}

fn to_content_id(oid: Oid) -> ContentId {
    unsafe { std::mem::transmute(oid) }
}
