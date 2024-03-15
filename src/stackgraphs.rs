use stack_graphs::arena::Handle;
use stack_graphs::graph::Node;
use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPath;
use stack_graphs::partial::PartialPaths;
use stack_graphs::serde;
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
use crate::core::PartialPosition;
use crate::core::Span;
use crate::languages::Lang;
use crate::loading::FileSystem;

static BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub fn build_stack_graph(source: &str, filename: &str) -> Option<StackGraphCtx> {
    if let Some(lang) = Lang::from_filename(filename) {
        if let Some(sgl) = &lang.config().sgl {
            return StackGraphCtx::build(source, filename, sgl);
        }
    }

    None
}

pub struct StackGraphCtx {
    graph: StackGraph,
    partials: PartialPaths,
    paths: Vec<PartialPath>,
}

impl StackGraphCtx {
    fn new(graph: StackGraph, partials: PartialPaths, paths: Vec<PartialPath>) -> Self {
        Self { graph, partials, paths }
    }

    pub fn build(source: &str, filename: &str, sgl: &StackGraphLanguage) -> Option<StackGraphCtx> {
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        let file = graph.get_or_create_file(filename);
        let res = sgl.build_stack_graph_into(
            &mut graph,
            file,
            source,
            &Variables::new(),
            &NoCancellation,
        );

        if res.is_err() {
            log::warn!("stack graphs failed on {}", filename);
            return None;
        }

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
        .unwrap();

        Some(Self::new(graph, partials, paths))
    }
}

#[derive(bincode::Encode, bincode::Decode)]
struct PortableStackGraphCtx {
    graph: serde::StackGraph,
    paths: Vec<serde::PartialPath>,
}

impl StackGraphCtx {
    fn to_portable(&mut self) -> PortableStackGraphCtx {
        let paths = self
            .paths
            .iter()
            .map(|p| serde::PartialPath::from_partial_path(&self.graph, &mut self.partials, p))
            .collect::<Vec<_>>();
        let graph = serde::StackGraph::from_graph(&self.graph);
        PortableStackGraphCtx { graph, paths }
    }

    #[allow(dead_code)]
    fn from_portable(portable: &PortableStackGraphCtx) -> anyhow::Result<Self> {
        Self::from_portables(std::iter::once(portable))
    }

    fn from_portables<'a, C>(portables: C) -> anyhow::Result<Self>
    where
        C: IntoIterator<Item = &'a PortableStackGraphCtx>,
    {
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        for portable in portables {
            portable.graph.load_into(&mut graph)?;

            for path in &portable.paths {
                paths.push(path.to_partial_path(&mut graph, &mut partials)?);
            }
        }

        Ok(StackGraphCtx::new(graph, partials, paths))
    }

    pub fn encode(&mut self) -> anyhow::Result<Vec<u8>> {
        Ok(bincode::encode_to_vec(self.to_portable(), BINCODE_CONFIG)?)
    }

    #[allow(dead_code)]
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        Self::decode_many(std::iter::once(bytes))
    }

    pub fn decode_many<'a, B>(bytes: B) -> anyhow::Result<Self>
    where
        B: IntoIterator<Item = &'a [u8]>,
    {
        let mut portables = Vec::new();

        for b in bytes {
            let portable: PortableStackGraphCtx = bincode::decode_from_slice(&b, BINCODE_CONFIG)?.0;
            portables.push(portable);
        }

        Self::from_portables(&portables)
    }
}

pub fn resolve(ctx: &mut StackGraphCtx, fs: &FileSystem) -> Vec<FileDep> {
    let mut references = Vec::new();

    let mut db = Database::new();

    for path in &ctx.paths {
        db.add_partial_path(&ctx.graph, &mut ctx.partials, path.clone());
    }

    let _stitching_res = ForwardPartialPathStitcher::find_all_complete_partial_paths(
        &mut DatabaseCandidates::new(&ctx.graph, &mut ctx.partials, &mut db),
        ctx.graph.iter_nodes().filter(|&n| ctx.graph[n].is_reference()),
        StitcherConfig::default(),
        &stack_graphs::NoCancellation,
        |_, _, p| {
            references.push(p.clone());
        },
    );

    let filename = |n: Handle<Node>| ctx.graph[ctx.graph[n].file().unwrap()].name().to_string();
    let file_key = |n: Handle<Node>| fs.get_key_for_filename(filename(n)).unwrap().clone();
    let position = |n: Handle<Node>| {
        PartialPosition::Whole(Span::from_lsp(&ctx.graph.source_info(n).unwrap().span).start)
    };

    references
        .iter()
        .map(|r| {
            let start_node_pos = position(r.start_node);
            Dep::new(
                FileEndpoint::new(file_key(r.start_node), start_node_pos),
                FileEndpoint::new(file_key(r.end_node), position(r.end_node)),
                DepKind::Use,
                start_node_pos,
            )
        })
        .collect()
}
