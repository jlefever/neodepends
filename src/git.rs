use anyhow::bail;
use anyhow::Result;
use git2::Commit;
use git2::Oid;
use git2::Repository;
use git2::TreeWalkMode;
use git2::TreeWalkResult;

use crate::core::FileKey;
use crate::core::FileSource;

pub struct GitCommit<'a> {
    repo: &'a Repository,
    commit: Commit<'a>,
}

impl<'a> GitCommit<'a> {
    pub fn new(repo: &'a Repository, commit: Commit<'a>) -> Self {
        Self { repo, commit }
    }

    pub fn from_str<S: AsRef<str>>(repo: &'a Repository, commit: S) -> Result<Self> {
        Ok(Self::new(repo, parse_rev(repo, commit)?))
    }
}

impl<'a> FileSource for GitCommit<'a> {
    fn discover(&self) -> Result<Vec<FileKey>> {
        let mut keys = Vec::new();

        self.commit
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

    fn load(&self, key: &FileKey) -> Result<Vec<u8>> {
        Ok(self.repo.find_blob(key.content_hash)?.content().to_owned())
    }
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
