use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPath;
use stack_graphs::partial::PartialPaths;
use stack_graphs::serde;
use stack_graphs::stitching::Database;
use stack_graphs::stitching::DatabaseCandidates;
use stack_graphs::stitching::ForwardPartialPathStitcher;
use stack_graphs::stitching::StitcherConfig;
use tree_sitter_graph::Variables;
use tree_sitter_stack_graphs::BuildError;
use tree_sitter_stack_graphs::NoCancellation;

static BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub struct StackGraphCtx {
    graph: StackGraph,
    partials: PartialPaths,
    paths: Vec<PartialPath>,
}

impl StackGraphCtx {
    fn new(graph: StackGraph, partials: PartialPaths, paths: Vec<PartialPath>) -> Self {
        Self {
            graph,
            partials,
            paths,
        }
    }

    pub fn build(source: &str, filename: &str) -> Result<StackGraphCtx, BuildError> {
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        let file = graph.get_or_create_file(filename);

        // TODO: Select the config using Loader from tree_sitter_stack_graphs
        let config = tree_sitter_stack_graphs_java::language_configuration(&NoCancellation);

        config.sgl.build_stack_graph_into(
            &mut graph,
            file,
            source,
            &Variables::new(),
            &NoCancellation,
        )?;

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

        Ok(Self::new(graph, partials, paths))
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

pub fn resolve<'a>(ctx: &'a mut StackGraphCtx) -> Vec<(&'a str, &'a str)> {
    let mut references = Vec::new();

    let mut db = Database::new();

    for path in &ctx.paths {
        db.add_partial_path(&ctx.graph, &mut ctx.partials, path.clone());
    }

    let _stitching_res = ForwardPartialPathStitcher::find_all_complete_partial_paths(
        &mut DatabaseCandidates::new(&ctx.graph, &mut ctx.partials, &mut db),
        ctx.graph
            .iter_nodes()
            .filter(|&n| ctx.graph[n].is_reference()),
        StitcherConfig::default(),
        &stack_graphs::NoCancellation,
        |_, _, p| {
            references.push(p.clone());
        },
    );

    references
        .iter()
        .map(|r| {
            let src_file = ctx.graph[ctx.graph[r.start_node].file().unwrap()].name();
            let tgt_file = ctx.graph[ctx.graph[r.end_node].file().unwrap()].name();
            (src_file, tgt_file)
        })
        .collect()
}
