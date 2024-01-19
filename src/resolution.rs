use anyhow::Result;
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

static BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub struct ResolutionCtx {
    pub graph: StackGraph,
    partials: PartialPaths,
    paths: Vec<PartialPath>,
}

impl ResolutionCtx {
    fn new(graph: StackGraph, partials: PartialPaths, paths: Vec<PartialPath>) -> Self {
        Self {
            graph,
            partials,
            paths,
        }
    }

    pub fn from_source(source: &str, filename: &str) -> Result<Self> {
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
        )?;

        Ok(Self::new(graph, partials, paths))
    }

    pub fn dummy(filename: &str) -> Result<Self> {
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        let file = graph.get_or_create_file(filename);

        // TODO: Select the config using Loader from tree_sitter_stack_graphs
        let config = tree_sitter_stack_graphs_java::language_configuration(&NoCancellation);

        config.sgl.build_stack_graph_into(
            &mut graph,
            file,
            "",
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
        )?;

        Ok(Self::new(graph, partials, paths))
    }

    pub fn resolve<'a>(&'a mut self) -> Vec<(&'a str, &'a str)> {
        let mut references = Vec::new();

        let mut db = Database::new();

        for path in &self.paths {
            db.add_partial_path(&self.graph, &mut self.partials, path.clone());
        }

        let _stitching_res = ForwardPartialPathStitcher::find_all_complete_partial_paths(
            &mut DatabaseCandidates::new(&self.graph, &mut self.partials, &mut db),
            self.graph
                .iter_nodes()
                .filter(|&n| self.graph[n].is_reference()),
            StitcherConfig::default(),
            &stack_graphs::NoCancellation,
            |_, _, p| {
                references.push(p.clone());
            },
        );

        references
            .iter()
            .map(|r| {
                let src_file = self.graph[self.graph[r.start_node].file().unwrap()].name();
                let tgt_file = self.graph[self.graph[r.end_node].file().unwrap()].name();
                (src_file, tgt_file)
            })
            .collect()
    }
}

#[derive(bincode::Encode, bincode::Decode)]
struct PortableResolutionCtx {
    graph: serde::StackGraph,
    paths: Vec<serde::PartialPath>,
}

impl ResolutionCtx {
    fn to_portable(&mut self) -> PortableResolutionCtx {
        let paths = self
            .paths
            .iter()
            .map(|p| serde::PartialPath::from_partial_path(&self.graph, &mut self.partials, p))
            .collect::<Vec<_>>();
        let graph = serde::StackGraph::from_graph(&self.graph);
        PortableResolutionCtx { graph, paths }
    }

    #[allow(dead_code)]
    fn from_portable(portable: &PortableResolutionCtx) -> Result<Self> {
        Self::from_portables(std::iter::once(portable))
    }

    fn from_portables<'a, C>(portables: C) -> Result<Self>
    where
        C: IntoIterator<Item = &'a PortableResolutionCtx>,
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

        Ok(ResolutionCtx::new(graph, partials, paths))
    }

    pub fn encode(&mut self) -> Result<Vec<u8>> {
        Ok(bincode::encode_to_vec(self.to_portable(), BINCODE_CONFIG)?)
    }

    #[allow(dead_code)]
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::decode_many(std::iter::once(bytes))
    }

    pub fn decode_many<'a, B>(bytes: B) -> Result<Self>
    where
        B: IntoIterator<Item = &'a [u8]>,
    {
        let mut portables = Vec::new();

        for b in bytes {
            let portable: PortableResolutionCtx = bincode::decode_from_slice(&b, BINCODE_CONFIG)?.0;
            portables.push(portable);
        }

        Self::from_portables(&portables)
    }
}
