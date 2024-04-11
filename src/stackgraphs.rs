//! Used to interface with Stack Graphs
//! 
//! See https://github.com/github/stack-graphs

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::RwLock;

use anyhow::bail;
use stack_graphs::arena::Handle;
use stack_graphs::graph::Node;
use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPath;
use stack_graphs::partial::PartialPaths;
use stack_graphs::stitching::Database;
use stack_graphs::stitching::DatabaseCandidates;
use stack_graphs::stitching::ForwardPartialPathStitcher;
use stack_graphs::stitching::StitcherConfig;
use tree_sitter_graph::Variables;
use tree_sitter_stack_graphs::NoCancellation;
use tree_sitter_stack_graphs::StackGraphLanguage;

use crate::core::Dep;
use crate::core::DepKind;
use crate::core::FileDep;
use crate::core::FileEndpoint;
use crate::core::FileKey;
use crate::core::PartialPosition;
use crate::core::PseudoCommitId;
use crate::core::Span;
use crate::languages::Lang;
use crate::resolution::Resolver;
use crate::resolution::ResolverFactory;

/// A Stack Graphs resolver.
///
/// See [Resolver].
pub struct StackGraphsResolver {
    commit_id: PseudoCommitId,
    sgl: Arc<StackGraphLanguage>,
    cache: Arc<SgCache>,
    files: RwLock<HashSet<FileKey>>,
}

impl StackGraphsResolver {
    fn new(commit_id: PseudoCommitId, sgl: Arc<StackGraphLanguage>, cache: Arc<SgCache>) -> Self {
        Self { commit_id, sgl, cache, files: Default::default() }
    }
}

impl Resolver for StackGraphsResolver {
    fn add_file(&self, filename: &str, content: &str) {
        let file = FileKey::from_content(filename.to_string(), content);

        if !self.cache.contains(&file) {
            self.cache.insert(file.clone(), build(&self.sgl, filename, content));
        }

        self.files.write().unwrap().insert(file);
    }

    fn resolve(&self) -> Vec<FileDep> {
        let files = self.files.read().unwrap();
        let data = files.iter().filter_map(|f| self.cache.get(f).unwrap());
        resolve(data, self.commit_id)
    }
}

impl Debug for StackGraphsResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StackGraphsResolver")
            .field("commit_id", &self.commit_id)
            .field("tsg_path", &self.sgl.tsg_path())
            .field("cache", &self.cache)
            .field("files", &self.files)
            .finish()
    }
}

/// A Stack Graphs resolver factory.
///
/// See [ResolverFactory].
#[derive(Debug)]
pub struct StackGraphsResolverFactory {
    cache: Arc<SgCache>,
}

impl StackGraphsResolverFactory {
    pub fn new() -> Self {
        Self { cache: Arc::new(SgCache::new()) }
    }
}

impl ResolverFactory for StackGraphsResolverFactory {
    fn try_create(&self, commit_id: PseudoCommitId, lang: Lang) -> Option<Box<dyn Resolver>> {
        lang.sgl().map(|sgl| {
            Box::new(StackGraphsResolver::new(commit_id, sgl, self.cache.clone()))
                as Box<dyn Resolver>
        })
    }
}

/// Used to avoid duplicate stack graph calculations.
#[derive(Debug)]
struct SgCache {
    map: RwLock<HashMap<FileKey, Option<StackGraphData>>>,
}

impl SgCache {
    fn new() -> Self {
        Self { map: Default::default() }
    }

    fn contains(&self, key: &FileKey) -> bool {
        self.map.read().unwrap().contains_key(key)
    }

    fn get(&self, key: &FileKey) -> Option<Option<StackGraphData>> {
        self.map.read().unwrap().get(key).cloned()
    }

    fn insert(&self, key: FileKey, value: Option<StackGraphData>) {
        self.map.write().unwrap().insert(key, value);
    }
}

/// A stack graph representation that is able to be cloned and moved around for
/// caching purposes.
///
/// Intended to contain the stack graph of a single file.
#[derive(Debug, Clone)]
struct StackGraphData {
    file_key: FileKey,
    graph: stack_graphs::serde::StackGraph,
    paths: Vec<stack_graphs::serde::PartialPath>,
}

impl StackGraphData {
    fn new(
        file_key: FileKey,
        graph: StackGraph,
        mut partials: PartialPaths,
        paths: Vec<PartialPath>,
    ) -> Self {
        let paths = paths
            .iter()
            .map(|p| stack_graphs::serde::PartialPath::from_partial_path(&graph, &mut partials, p))
            .collect::<Vec<_>>();
        let graph = stack_graphs::serde::StackGraph::from_graph(&graph);
        Self { file_key, graph, paths }
    }
}

/// A stack graph representation that can be used to resolve dependencies.
///
/// Intended to contain the stack graphs of many files.
struct StackGraphEval {
    file_keys: HashMap<String, FileKey>,
    graph: StackGraph,
    partials: PartialPaths,
    paths: Vec<PartialPath>,
}

impl StackGraphEval {
    fn from_data<I>(data: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = StackGraphData>,
    {
        let mut file_keys = HashMap::new();
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        for portable in data {
            if file_keys.contains_key(&portable.file_key.filename) {
                bail!("duplicate filenames");
            }

            file_keys.insert(portable.file_key.filename.clone(), portable.file_key.clone());
            portable.graph.load_into(&mut graph)?;

            for path in &portable.paths {
                paths.push(path.to_partial_path(&mut graph, &mut partials)?);
            }
        }

        Ok(StackGraphEval { file_keys, graph, partials, paths })
    }
}

/// Attempt to build a stack graph from a source file.
///
/// Returns None if a stack graph could not be built.
fn build(sgl: &StackGraphLanguage, filename: &str, content: &str) -> Option<StackGraphData> {
    let mut graph = StackGraph::new();
    let mut partials = PartialPaths::new();
    let mut paths = Vec::new();

    let file_key = FileKey::from_content(filename.to_string(), content);

    let file = graph.get_or_create_file(filename);
    let vars = Variables::new();
    sgl.build_stack_graph_into(&mut graph, file, content, &vars, &NoCancellation).ok()?;

    ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
        &graph,
        &mut partials,
        file,
        StitcherConfig::default(),
        &stack_graphs::NoCancellation,
        |_, _, p| {
            paths.push(p.clone());
        },
    )
    .ok()?;

    Some(StackGraphData::new(file_key, graph, partials, paths))
}

/// Resolve file-level dependencies given for a collection of files.
fn resolve<I>(data: I, commit_id: PseudoCommitId) -> Vec<FileDep>
where
    I: IntoIterator<Item = StackGraphData>,
{
    let mut references = Vec::new();
    let mut eval = StackGraphEval::from_data(data).unwrap();

    let mut db = Database::new();

    for path in &eval.paths {
        db.add_partial_path(&eval.graph, &mut eval.partials, path.clone());
    }

    let _stitching_res = ForwardPartialPathStitcher::find_all_complete_partial_paths(
        &mut DatabaseCandidates::new(&eval.graph, &mut eval.partials, &mut db),
        eval.graph.iter_nodes().filter(|&n| eval.graph[n].is_reference()),
        StitcherConfig::default(),
        &stack_graphs::NoCancellation,
        |_, _, p| {
            references.push(p.clone());
        },
    );

    let filename = |n: Handle<Node>| eval.graph[eval.graph[n].file().unwrap()].name().to_string();
    let file_key = |n: Handle<Node>| eval.file_keys.get(&filename(n)).unwrap().clone();
    let position = |n: Handle<Node>| {
        PartialPosition::Whole(Span::from_lsp(&eval.graph.source_info(n).unwrap().span).start)
    };

    references
        .into_iter()
        .map(|r| {
            let start_node_pos = position(r.start_node);
            Dep::new(
                FileEndpoint::new(file_key(r.start_node), start_node_pos),
                FileEndpoint::new(file_key(r.end_node), position(r.end_node)),
                DepKind::Use,
                start_node_pos,
                commit_id,
            )
        })
        .collect()
}
