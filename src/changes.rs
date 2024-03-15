use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use git2::Repository;
use itertools::Itertools;

use crate::core::CommitId;
use crate::core::Diff;
use crate::core::FileKey;
use crate::core::Hunk;
use crate::loading::FileFilter;

#[derive(Clone)]
pub struct DiffCalculator {
    repo: Arc<Mutex<Repository>>,
}

impl DiffCalculator {
    pub fn new(repo: Arc<Mutex<Repository>>) -> Self {
        Self { repo }
    }

    pub fn diff(&self, commit_id: CommitId, filter: &FileFilter) -> Result<Vec<Diff>> {
        diff_with_parent(self.repo.lock().unwrap(), commit_id, filter)
    }
}

pub fn diff_with_parent<R: Deref<Target = Repository>>(
    repo: R,
    commit_id: CommitId,
    filter: &FileFilter,
) -> Result<Vec<Diff>> {
    let commit = repo.find_commit(commit_id.to_oid())?;
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

    let mut diffs = Vec::new();
    let mut modified = HashMap::new();

    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |delta, hunk| {
            let filename = get_diff_delta_filename(&delta);

            if !filter.includes(filename) {
                return true;
            }

            let old = delta.old_file().id().into();
            let new = delta.new_file().id().into();

            match delta.status() {
                git2::Delta::Added => {
                    diffs.push(Diff::added(commit_id, FileKey::new(filename.to_string(), new)));
                }
                git2::Delta::Deleted => {
                    diffs.push(Diff::deleted(commit_id, FileKey::new(filename.to_string(), old)));
                }
                git2::Delta::Modified => {
                    let old = FileKey::new(filename.to_string(), old);
                    let new = FileKey::new(filename.to_string(), new);
                    modified.entry((old, new)).or_insert(Vec::new()).push(Hunk::from_git(&hunk));
                }
                _ => panic!("unsupported diff status: {:?}", &delta.status()),
            };

            true
        }),
        None,
    )?;

    diffs.extend(modified.into_iter().map(|((x, y), z)| Diff::modified(commit_id, x, y, z)));
    diffs.sort();
    Ok(diffs)
}

fn get_diff_delta_filename<'a>(diff_delta: &'a git2::DiffDelta) -> &'a str {
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
