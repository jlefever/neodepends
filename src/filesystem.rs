use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::fs::File;
use std::io::Read;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use itertools::Itertools;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::core::CommitId;
use crate::core::ContentId;
use crate::core::Diff;
use crate::core::FileKey;
use crate::core::FileSet;
use crate::core::Hunk;
use crate::core::MultiFileSet;
use crate::core::PseudoCommitId;
use crate::spec::Filespec;
use crate::spec::Pathspec;

/// The central way to interact with the filesystem inside Neodepends.
#[derive(Debug, Clone)]
pub struct FileSystem {
    disk: Disk,
    repo: Option<Repository>,
}

impl FileSystem {
    /// Attempt to open a project given a root directory.
    ///
    /// Will open a git repository if one exists at this directory. Otherwise,
    /// it will open in "disk-only" mode.
    pub fn open<P: AsRef<Path>>(root: P) -> Result<Self> {
        let mut root = root.as_ref().to_path_buf();
        let repo = Repository::open(&root).ok();

        if let Some(repo) = &repo {
            root = repo.path().to_path_buf();
        } else {
            log::warn!("No git repository found. Opening in disk-only mode.");
        }

        log::info!("Project opened at: {}", root.to_string_lossy());
        Ok(Self { disk: Disk::open(root)?, repo })
    }

    /// Attempt to parse a revspec as a [PseudoCommitId].
    ///
    /// This revspec must refer to a single commit, not a range.
    pub fn parse_as_commit(&self, revspec: &str) -> Result<PseudoCommitId> {
        if revspec == "WORKDIR" {
            Ok(PseudoCommitId::WorkDir)
        } else {
            let repo = self.repo.as_ref().context("cannot parse commit in disk-only mode")?;
            let repo = repo.repo.lock().unwrap();
            let object = repo.revparse_single(revspec)?;
            let commit = object.into_commit().ok().context("not a commit")?;
            Ok(PseudoCommitId::CommitId(commit.id().into()))
        }
    }

    /// Walk the commits and files reachable from the given [Filespec],
    /// returning the results as a [MultiFileSet].
    pub fn list(&self, spec: &Filespec) -> MultiFileSet {
        let mut map = HashMap::new();

        if spec.commits.contains(&PseudoCommitId::WorkDir) {
            map.insert(PseudoCommitId::WorkDir, self.disk.list(&spec.pathspec).unwrap());
        }

        let commits = spec.commits.iter().filter_map(|c| c.try_as_commit_id()).collect_vec();

        if let Some(repo) = &self.repo {
            map.extend(
                commits
                    .into_par_iter()
                    .map(|c| (PseudoCommitId::CommitId(c), repo.list(c, &spec.pathspec).unwrap()))
                    .collect::<Vec<_>>(),
            );
        } else if !commits.is_empty() {
            panic!("attempted to list files of commit while in disk-only mode")
        }

        MultiFileSet::new(map)
    }

    /// Compares the given commit against its parent and produces a vec of
    /// [Diff]s.
    ///
    /// One Diff per touched file.
    pub fn diff(&self, commit_id: CommitId, pathspec: &Pathspec) -> Result<Vec<Diff>> {
        if let Some(repo) = &self.repo {
            repo.diff(commit_id, pathspec)
        } else {
            bail!("attempted to diff while in disk-only mode")
        }
    }

    /// Read the contents of a file as a UTF-8 String.
    fn read_to_string(&self, content_id: ContentId) -> Result<String> {
        String::from_utf8(self.read_to_vec(content_id)?).context("invalid UTF-8")
    }

    /// Read the contents of a file as a vec of bytes.
    fn read_to_vec(&self, content_id: ContentId) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.read_buf(content_id, &mut buf)?;
        Ok(buf)
    }

    /// Read the contents of a file into an existing buffer of bytes.
    ///
    /// Will first try to find the file in the git repository. If it can't be
    /// found, it will look for it on disk. If [FileSystem] is in disk-only
    /// mode, it will only try loading from the disk.
    fn read_buf(&self, content_id: ContentId, buf: &mut Vec<u8>) -> Result<()> {
        match self.repo.as_ref().map(|r| r.read_buf(content_id, buf)) {
            Some(Ok(())) => Ok(()),
            _ => self.disk.read_buf(content_id, buf),
        }
    }
}

/// A trait for reading the contents of a file into a UTF-8 String.
///
/// Only implementation is [FileSystem]. Mainly useful to allow other code to
/// not explicitly depend on FileSystem.
pub trait FileReader: Send + Sync {
    fn read(&self, content_id: ContentId) -> Result<String>;
}

impl FileReader for FileSystem {
    fn read(&self, content_id: ContentId) -> Result<String> {
        self.read_to_string(content_id)
    }
}

/// A wrapper around [`git2::Repository`].
///
/// This wrapper uses [Arc] and [Mutex] to create a thread-safe Repository. We
/// prefer Mutex over [`std::sync::RwLock`] because there is no guarantee that
/// operations that are ostensibly read-only are actually thread-safe.
#[derive(Clone)]
struct Repository {
    repo: Arc<Mutex<git2::Repository>>,
    path: PathBuf,
}

impl Repository {
    /// Attempt to open a Repository at or above `path`.
    fn open<P: AsRef<Path>>(path: P) -> Result<Repository> {
        let repo = git2::Repository::discover(path)?;

        // This is a necessary config for Windows
        repo.config().unwrap().set_bool("core.longpaths", true).unwrap();

        // Get path of repository for printing to user
        let mut path = repo.path().canonicalize().unwrap();

        if path.ends_with(".git") {
            path = path.parent().unwrap().to_path_buf();
        }

        Ok(Self { repo: Arc::new(Mutex::new(repo)), path })
    }

    /// Root of repository (without .git).
    fn path(&self) -> &Path {
        &self.path
    }

    /// Collect all [FileKey]s that are reachable from the given commit and
    /// pathspec.
    fn list<C>(&self, commit: C, pathspec: &Pathspec) -> Result<FileSet>
    where
        C: Into<git2::Oid>,
    {
        Ok(FileSet::new(walk_commits(self.repo.lock().unwrap(), vec![commit], pathspec)?))
    }

    /// Read the contents of a blob into the provided buffer.
    fn read_buf<B: Into<git2::Oid>>(&self, blob_id: B, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(self.repo.lock().unwrap().find_blob(blob_id.into())?.content());
        Ok(())
    }

    /// Collect all [FileKey]s that changed between this commit and its parent.
    fn diff<C>(&self, commit_id: C, pathspec: &Pathspec) -> Result<Vec<Diff>>
    where
        C: Into<git2::Oid>,
    {
        diff_with_parent(self.repo.lock().unwrap(), commit_id, pathspec)
    }
}

impl Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Repository").field(&self.path).finish()
    }
}

/// Represents a project as it exists on disk (according to the OS).
#[derive(Debug, Clone)]
struct Disk {
    root: PathBuf,
    file_set: FileSet,
}

impl Disk {
    fn open<P: AsRef<Path>>(root: P) -> Result<Self> {
        let file_set = FileSet::new(walk_dir(root.as_ref(), &Pathspec::default())?);
        Ok(Self { root: root.as_ref().to_path_buf(), file_set })
    }

    fn list(&self, pathspec: &Pathspec) -> Result<FileSet> {
        Ok(FileSet::new(walk_dir(&self.root, pathspec)?))
    }

    fn read_buf(&self, content_id: ContentId, buf: &mut Vec<u8>) -> Result<()> {
        let filename = self
            .file_set
            .get_filenames(content_id)
            .next()
            .context("could not find file with matching id")?;
        self.read_buf_by_filename(filename, buf)
    }

    fn read_buf_by_filename<P: AsRef<Path>>(&self, filename: P, buf: &mut Vec<u8>) -> Result<()> {
        File::open(self.root.join(filename))?.read_to_end(buf)?;
        Ok(())
    }
}

/// Collect [FileKey]s by recursively walking directories starting from `root`.
///
/// Note: Does not respect `.gitignore`, even when `root` refers to a git
/// repository.
fn walk_dir<P: AsRef<Path>>(root: P, pathspec: &Pathspec) -> Result<Vec<FileKey>> {
    let mut keys = Vec::new();

    for entry in WalkDir::new(root.as_ref()).follow_links(true) {
        match entry {
            Ok(entry) => {
                let path = entry.path().strip_prefix(root.as_ref())?;

                if path.is_file() && pathspec.matches(&path) {
                    let content_id = ContentId::from_path(&path);
                    let filename = path.to_string_lossy().to_string();
                    keys.push(FileKey::new(filename, content_id));
                }
            }
            Err(err) => {
                log::warn!("Failed to read directory entry: {}. Skipping...", err);
            }
        }
    }

    Ok(keys)
}

/// Collect [FileKey]s by recursively walking git trees associated with the
/// given commits.
fn walk_commits<R, C, I>(repo: R, commit_ids: I, pathspec: &Pathspec) -> Result<Vec<FileKey>>
where
    R: Deref<Target = git2::Repository>,
    C: Into<git2::Oid>,
    I: IntoIterator<Item = C>,
{
    let mut keys = Vec::new();
    let mut visited = HashSet::new();

    // TODO: Allow None to be passed in for commit_id, then read from the working
    // tree instead. This would let us respect the .gitignore rules. libgit2 doesn't
    // allow us to open the workdir as a tree that can be walk. Instead, we can use
    // `diff_tree_to_workdir` to collect filenames then load these filenames from
    // the disk.
    for id in commit_ids {
        let commit = repo.find_commit(id.into())?;

        commit.tree()?.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if visited.contains(&entry.id()) {
                return git2::TreeWalkResult::Skip;
            }

            visited.insert(entry.id());
            let path = dir.to_string() + entry.name().unwrap();

            // TODO: Consider using `.matches_tree` of `git2::Pathspec` for potential
            // performance gains
            if pathspec.matches(&path) {
                keys.push(FileKey::new(path, entry.id().into()));
            }

            git2::TreeWalkResult::Ok
        })?;
    }

    Ok(keys)
}

fn diff_with_parent<R, C>(repo: R, commit_id: C, pathspec: &Pathspec) -> Result<Vec<Diff>>
where
    R: Deref<Target = git2::Repository>,
    C: Into<git2::Oid>,
{
    let commit_id: git2::Oid = commit_id.into();
    let commit = repo.find_commit(commit_id)?;
    let parents = commit.parents().collect_vec();
    let new_tree = commit.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.ignore_filemode(true);
    opts.context_lines(0);

    let diff = match parents.len() {
        0 => repo.diff_tree_to_tree(None, Some(&new_tree), Some(&mut opts)),
        1 => {
            let parent = parents.get(0).unwrap();
            let old_tree = parent.tree()?;
            repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut opts))
        }
        _ => return Ok(Vec::new()),
    }?;

    let mut diffs = HashMap::new();

    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |delta, hunk| {
            let filename = diff_delta_filename(&delta);

            if !pathspec.matches(filename) {
                return true;
            }

            match delta.status() {
                git2::Delta::Added => (),
                git2::Delta::Deleted => (),
                git2::Delta::Modified => (),
                _ => panic!("unsupported diff status: {:?}", &delta.status()),
            };

            let old = to_file_key(filename, delta.old_file().id());
            let new = to_file_key(filename, delta.new_file().id());
            diffs.entry((old, new)).or_insert(Vec::new()).push(Hunk::from_git(&hunk));
            true
        }),
        None,
    )?;

    Ok(diffs.into_iter().map(|((x, y), z)| Diff::new(commit_id.into(), x, y, z)).sorted().collect())
}

fn diff_delta_filename<'a>(diff_delta: &'a git2::DiffDelta) -> &'a str {
    let old_path = diff_delta.old_file().path();
    let new_path = diff_delta.new_file().path();

    let path = match (old_path, new_path) {
        (None, None) => panic!("expected at least one side of diff to be non-empty"),
        (None, Some(path)) => path,
        (Some(path), None) => path,
        (Some(old_path), Some(new_path)) => {
            if old_path != new_path {
                panic!("expected no renames or moves");
            }
            old_path
        }
    };

    path.to_str().unwrap()
}

fn to_file_key(filename: &str, oid: git2::Oid) -> Option<FileKey> {
    if oid.is_zero() {
        None
    } else {
        Some(FileKey::new(filename.to_string(), oid.into()))
    }
}
