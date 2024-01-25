use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use anyhow::bail;
use anyhow::Result;
use git2::Commit;
use git2::ObjectType;
use git2::Oid;
use git2::Repository;
use git2::TreeWalkMode;
use git2::TreeWalkResult;
use walkdir::WalkDir;

use crate::core::FileKey;

pub trait FileLoader {
    /// Discover all files in this file loader.
    fn discover(&self) -> Result<Vec<FileKey>>;

    /// Load the content of a file given a file key. The key must be from the
    /// `discover` method on the same file loader.
    fn load(&self, key: &FileKey) -> Result<Vec<u8>>;
}

pub struct GitFileLoader<'a> {
    repo: &'a Repository,
    commit: Commit<'a>,
}

impl<'a> GitFileLoader<'a> {
    pub fn new(repo: &'a Repository, commit: Commit<'a>) -> Self {
        Self { repo, commit }
    }

    pub fn from_str<S: AsRef<str>>(repo: &'a Repository, commit: S) -> Result<Self> {
        Ok(Self::new(repo, parse_rev(&repo, commit)?))
    }
}

impl<'a> FileLoader for GitFileLoader<'a> {
    fn discover(&self) -> Result<Vec<FileKey>> {
        list_files_git(&self.commit)
    }

    fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        load_file_git(&self.repo, key)
    }
}

pub struct DiskFileLoader {
    project_root: PathBuf,
}

impl DiskFileLoader {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

impl FileLoader for DiskFileLoader {
    fn discover(&self) -> Result<Vec<FileKey>> {
        list_files_nogit(&self.project_root)
    }

    fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        load_file_nogit(&self.project_root, key)
    }
}

fn list_files_git(commit: &Commit<'_>) -> Result<Vec<FileKey>> {
    let mut keys = Vec::new();

    commit.tree()?.walk(TreeWalkMode::PreOrder, |dir, entry| {
        let path = dir.to_string() + entry.name().unwrap();

        if path.ends_with(".java") {
            keys.push(FileKey::new(path, entry.id()));
        }

        TreeWalkResult::Ok
    })?;

    Ok(keys)
}

fn load_file_git(repo: &Repository, key: &FileKey) -> Result<Vec<u8>> {
    Ok(repo.find_blob(key.content_hash)?.content().to_owned())
}

fn list_files_nogit<P: AsRef<Path>>(root: P) -> Result<Vec<FileKey>> {
    let mut keys = Vec::new();

    for entry in WalkDir::new(&root).follow_links(true) {
        match entry {
            Ok(entry) => {
                let path = entry
                    .path()
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .to_string();

                if path.ends_with(".java") {
                    let content_hash = Oid::hash_file(ObjectType::Blob, &path)?;
                    keys.push(FileKey::new(path, content_hash));
                }
            }
            Err(err) => {
                log::warn!("Failed to read directory entry: {}. Skipping...", err);
            }
        }
    }

    Ok(keys)
}

fn load_file_nogit<P: AsRef<Path>>(root: P, key: &FileKey) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let path = root.as_ref().join(&key.filename);
    File::open(path)?.read_to_end(&mut buf)?;
    Ok(buf)
}

fn parse_rev<'a, S: AsRef<str>>(repo: &'a Repository, rev: S) -> Result<Commit<'a>> {
    if let Ok(rev) = repo.resolve_reference_from_short_name(rev.as_ref()) {
        Ok(rev.peel_to_commit()?)
    } else if let Ok(oid) = Oid::from_str(rev.as_ref()) {
        Ok(repo.find_commit(oid)?)
    } else {
        bail!(
            "the given revision ('{}') was not found in this repository",
            rev.as_ref()
        );
    }
}
