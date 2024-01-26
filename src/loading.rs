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

pub enum FileSystem {
    Disk { root: PathBuf },
    Git { repo: Repository, commit: String },
}

impl FileSystem {
    fn disk<P: AsRef<Path>>(root: P) -> Self {
        Self::Disk {
            root: root.as_ref().to_owned(),
        }
    }

    fn git<S: AsRef<str>>(repo: Repository, commit: S) -> Result<Self> {
        // This is a necessary config for Windows
        repo.config()?.set_bool("core.longpaths", true)?;

        Ok(Self::Git {
            repo,
            commit: commit.as_ref().to_owned(),
        })
    }

    pub fn open<P, S>(root: P, commit: &Option<S>) -> Result<Self>
    where
        P: AsRef<Path>,
        S: AsRef<str>,
    {
        let repo = Repository::open(&root).ok();

        Ok(match (repo, commit.as_ref()) {
            (None, None) => Self::disk(root),
            (None, Some(_)) => bail!(
                "a commit was supplied but the project root does not refer to a git repository"
            ),
            (Some(_), None) => Self::disk(root),
            (Some(repo), Some(commit)) => Self::git(repo, commit)?,
        })
    }

    pub fn ls(&self) -> Result<Vec<FileKey>> {
        match &self {
            FileSystem::Disk { root } => ls_disk(root),
            FileSystem::Git { repo, commit } => ls_git(&repo, &commit),
        }
    }

    pub fn load_file(&self, key: &FileKey) -> Result<Vec<u8>> {
        match &self {
            FileSystem::Disk { root } => load_file_disk(root, key),
            FileSystem::Git { repo, .. } => load_file_git(repo, key),
        }
    }
}

fn ls_disk<P: AsRef<Path>>(root: P) -> Result<Vec<FileKey>> {
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

fn ls_git<S: AsRef<str>>(repo: &Repository, commit: S) -> Result<Vec<FileKey>> {
    let mut keys = Vec::new();

    parse_commit(repo, commit)?
        .tree()?
        .walk(TreeWalkMode::PreOrder, |dir, entry| {
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

fn load_file_disk<P: AsRef<Path>>(root: P, key: &FileKey) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let path = root.as_ref().join(&key.filename);
    File::open(path)?.read_to_end(&mut buf)?;
    Ok(buf)
}

fn parse_commit<'a, S: AsRef<str>>(repo: &'a Repository, commit: S) -> Result<Commit<'a>> {
    if let Ok(rev) = repo.resolve_reference_from_short_name(commit.as_ref()) {
        Ok(rev.peel_to_commit()?)
    } else if let Ok(oid) = Oid::from_str(commit.as_ref()) {
        Ok(repo.find_commit(oid)?)
    } else {
        bail!(
            "the given commit ('{}') was not found in this repository",
            commit.as_ref()
        );
    }
}
