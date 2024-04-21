use std::collections::HashMap;
use std::fmt::Debug;

use rayon::prelude::*;

use crate::core::FileDep;
use crate::core::FileKey;
use crate::core::MultiFileSet;
use crate::core::PseudoCommitId;
use crate::languages::Lang;
use crate::filesystem::FileReader;

/// Used to extract the file-level dependencies between given source
/// files.
pub trait Resolver: Debug + Send + Sync {
    /// Add a source file to the internal state of the resolver.
    ///
    /// May be called from parallel threads. All files added should be from the
    /// same version of the project and written in the same programming
    /// language.
    fn add_file(&self, filename: &str, content: &str);

    /// Resolve file-level dependencies between source files.
    ///
    /// Only considers files added with [Resolver::add_file]. It is undefined
    /// behavior to call [Resolver::add_file] while [Resolver::resolve] is
    /// running. It is undefined behavior to call this function more than once.
    fn resolve(&self) -> Vec<FileDep>;
}

/// Used to create a [Resolver] as a trait object.
pub trait ResolverFactory: Debug + Send + Sync + 'static {
    /// Attempt to create a Resolver for the given commit and lang.
    ///
    /// Will return [None] if the underlying Resolver does not support this
    /// language.
    fn try_create(&self, commit_id: PseudoCommitId, lang: Lang) -> Option<Box<dyn Resolver>>;
}

/// Used to resolve the dependencies of different versions and languages
/// simultaneously
#[derive(Debug)]
pub struct ResolverManager {
    resolvers: Vec<Box<dyn ResolverFactory>>,
}

impl ResolverManager {
    /// Create a [ResolverManager] from a list of [ResolverFactory]s.
    ///
    /// The list should be sorted in order of decreasing priority.
    pub fn new(resolvers: Vec<Box<dyn ResolverFactory>>) -> Self {
        Self { resolvers }
    }

    /// Create a [ResolverManager] without any resolvers.
    pub fn empty() -> Self {
        Self { resolvers: Vec::new() }
    }

    /// Resolve the file-level dependencies for each version contained within
    /// the [MultiFileSet].
    ///
    /// The [crate::core::FileSet] associated with each version is split into
    /// disjoint subsets of files where each subset contains all the files
    /// written in a particular language. Each of these subsets is considered
    /// independently and has its dependencies resolved in parallel.
    pub fn resolve<R: FileReader>(&self, reader: &R, files: &MultiFileSet) -> Vec<FileDep> {
        // Save some work if we know there are no resolvers
        if self.resolvers.is_empty() {
            return Vec::new();
        }
        
        // Create a list of resolvers and an associated list (of lists) of files
        let (resolvers, files): (Vec<_>, Vec<_>) = to_map(files)
            .into_par_iter()
            .filter_map(|((commit_id, lang), files)| {
                self.resolver_for(commit_id, lang).map(|r| (r, files))
            })
            .collect();

        // Organize files and resolvers so we only have to load each file once
        let mut lookup: HashMap<&FileKey, Vec<&Box<dyn Resolver>>> = HashMap::new();
        for (i, inner_files) in files.iter().enumerate() {
            for &file in inner_files {
                lookup.entry(file).or_default().push(&resolvers[i]);
            }
        }

        // Iterate through the files and add each one to their associated resolvers
        lookup.into_par_iter().for_each(|(f, resolvers)| {
            let content = reader.read(f.content_id).unwrap();
            resolvers.into_par_iter().for_each(|r| r.add_file(&f.filename, &content));
        });

        // Resolve everything
        resolvers.into_par_iter().flat_map(|r| r.resolve()).collect()
    }

    /// Try to create a resolver for a particular version and language
    fn resolver_for(&self, commit_id: PseudoCommitId, lang: Lang) -> Option<Box<dyn Resolver>> {
        self.resolvers.iter().filter_map(|f| f.try_create(commit_id, lang)).next()
    }
}

/// Group the given files by their version and language.
fn to_map<'a>(files: &'a MultiFileSet) -> HashMap<(PseudoCommitId, Lang), Vec<&'a FileKey>> {
    let mut map: HashMap<_, Vec<_>> = HashMap::new();

    for (&commit_id, file_set) in files.iter() {
        for file in file_set.iter() {
            if let Some(lang) = Lang::of(&file.filename) {
                map.entry((commit_id, lang)).or_default().push(file);
            }
        }
    }

    map
}
