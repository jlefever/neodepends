use std::collections::BTreeSet;
use std::fmt::Debug;
use std::path::Path;

use crate::core::PseudoCommitId;

/// A wrapper around [git2::Pathspec].
///
/// The inner Pathspec does not implement [Debug], [Default], [Clone], [Send] or
/// [Sync]. This wrapper provides implementations for these traits.
pub struct Pathspec {
    patterns: Vec<String>,
    pathspec: git2::Pathspec,
}

impl Pathspec {
    /// Create a Pathspec from an iterator of patterns, possibly panicking.
    ///
    /// See https://git-scm.com/docs/gitglossary#def_pathspec
    pub fn new<S: Into<String>, I: IntoIterator<Item = S>>(patterns: I) -> Self {
        let patterns = patterns.into_iter().map(|s| s.into()).collect();
        Self::from_vec(patterns)
    }

    /// Create a Pathspec from a vec of patterns, possibly panicking.
    ///
    /// See https://git-scm.com/docs/gitglossary#def_pathspec
    pub fn from_vec(patterns: Vec<String>) -> Self {
        Self::try_from_vec(patterns).unwrap()
    }

    /// Attempt to create a Pathspec from a vec of patterns.
    ///
    /// See https://git-scm.com/docs/gitglossary#def_pathspec
    pub fn try_from_vec(patterns: Vec<String>) -> Result<Pathspec, git2::Error> {
        let pathspec = git2::Pathspec::new(&patterns)?;
        Ok(Self { patterns, pathspec })
    }

    /// Create a new Pathspec by merging two together.
    pub fn merge(&self, other: &Pathspec) -> Pathspec {
        let mut patterns = self.patterns.clone();
        patterns.extend(other.patterns.clone());
        Self::from_vec(patterns)
    }

    /// Check if a path matches the Pathspec.
    ///
    /// Always case-insensitive regardless of the platform. Will panic if the
    /// empty path ("") is given.
    pub fn matches<P: AsRef<Path>>(&self, path: P) -> bool {
        self.pathspec.matches_path(path.as_ref(), git2::PathspecFlags::IGNORE_CASE)
    }
}

impl Debug for Pathspec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Pathspec").field(&self.patterns).finish()
    }
}

impl Default for Pathspec {
    fn default() -> Self {
        Self::from_vec(Vec::default())
    }
}

impl Clone for Pathspec {
    fn clone(&self) -> Self {
        Self::from_vec(self.patterns.clone())
    }
}

unsafe impl Send for Pathspec {}

unsafe impl Sync for Pathspec {}

/// Specify a collection of [crate::core::FileKey]s.
///
/// Any file that both matches the [Self::pathspec] and is reachable from at
/// least one of [Self::commits] is included.
#[derive(Debug, Clone)]
pub struct Filespec {
    pub commits: BTreeSet<PseudoCommitId>,
    pub pathspec: Pathspec,
}

impl Filespec {
    /// Create a Filespec from an iterator of commits and a [Pathspec].
    pub fn new<I: IntoIterator<Item = PseudoCommitId>>(commits: I, pathspec: Pathspec) -> Self {
        Self { commits: commits.into_iter().collect(), pathspec }
    }
}
