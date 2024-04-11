use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::RwLock;

use counter::Counter;
use rayon::prelude::*;

use crate::core::Change;
use crate::core::Diff;
use crate::core::Entity;
use crate::core::EntityDep;
use crate::core::FileKey;
use crate::core::SimpleEntityId;
use crate::languages::Lang;
use crate::spec::Filespec;
use crate::filesystem::FileReader;
use crate::filesystem::FileSystem;
use crate::resolution::ResolverManager;
use crate::tagging::EntitySet;

pub struct Extractor {
    fs: FileSystem,
    resolver: ResolverManager,
    entity_sets: RwLock<HashMap<FileKey, EntitySet>>,
}

impl Extractor {
    pub fn new(fs: FileSystem, resolver: ResolverManager) -> Self {
        Self { fs, resolver, entity_sets: Default::default() }
    }

    pub fn extract_entities(&self, spec: &Filespec) -> Vec<Entity> {
        let files = self.fs.list(spec).files().cloned().collect::<HashSet<_>>();
        self.iter_entity_sets(files).flat_map(|(_, e)| e.into_entities_vec()).collect()
    }

    pub fn extract_changes(&self, spec: &Filespec) -> Vec<Change> {
        let diffs: Vec<_> = spec
            .commits
            .par_iter()
            .filter_map(|c| c.try_as_commit_id())
            .flat_map(|c| self.fs.diff(c, &spec.pathspec).unwrap())
            .collect();
        let files = diffs.iter().flat_map(|d| d.iter_file_keys().cloned()).collect();
        let entity_sets = self.collect_entity_sets(files);
        diffs.into_par_iter().flat_map(move |d| calc_changes(&entity_sets, &d)).collect()
    }

    pub fn extract_deps(&self, spec: &Filespec) -> Vec<EntityDep> {
        let files = self.fs.list(spec);
        let deps = self.resolver.resolve(&self.fs, &files);
        let entity_sets = self.collect_entity_sets(files.files().cloned().collect());
        deps.into_par_iter().map(move |d| d.to_entity_dep(&entity_sets).unwrap()).collect()
    }

    fn collect_entity_sets(&self, files: HashSet<FileKey>) -> HashMap<FileKey, EntitySet> {
        self.iter_entity_sets(files).into_par_iter().collect()
    }

    fn iter_entity_sets(
        &self,
        files: HashSet<FileKey>,
    ) -> impl ParallelIterator<Item = (FileKey, EntitySet)> + '_ {
        files.into_par_iter().map(|f| {
            if let Some(entity_set) = self.entity_sets.read().unwrap().get(&f) {
                return (f, entity_set.clone());
            }

            let content = self.fs.read(&f).unwrap();
            let lang = Lang::of(&f.filename).unwrap();
            let entity_set = lang.tagger().tag(&f.filename, &content);
            self.entity_sets.write().unwrap().insert(f.clone(), entity_set.clone());
            (f, entity_set)
        })
    }
}

fn calc_changes(entity_sets: &HashMap<FileKey, EntitySet>, diff: &Diff) -> Vec<Change> {
    let change_kind = diff.change_kind();

    let old_entity_set = diff.old.as_ref().map(|k| entity_sets.get(&k).unwrap());
    let new_entity_set = diff.new.as_ref().map(|k| entity_sets.get(&k).unwrap());

    let old_ids = old_entity_set.map(|s| s.count_simple_ids(diff.iter_old_spans()));
    let new_ids = new_entity_set.map(|s| s.count_simple_ids(diff.iter_new_spans()));

    let mut ids = HashSet::new();
    ids.extend(old_ids.iter().flat_map(|x| x.keys()));
    ids.extend(new_ids.iter().flat_map(|x| x.keys()));

    let old_counts: Counter<SimpleEntityId> = old_ids.into_iter().flatten().collect();
    let new_counts: Counter<SimpleEntityId> = new_ids.into_iter().flatten().collect();

    ids.iter()
        .map(|id| Change::new(*id, diff.commit_id, change_kind, old_counts[id], new_counts[id]))
        .collect()
}
