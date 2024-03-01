use ::serde::Serialize;
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
use strum_macros::AsRefStr;
use strum_macros::EnumString;
use tree_sitter_graph::Variables;
use tree_sitter_stack_graphs::BuildError;
use tree_sitter_stack_graphs::NoCancellation;

use crate::core::EntityId;
use crate::core::Loc;
use crate::entities::EntitySet;
use crate::languages::Lang;

static BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub struct StackGraphCtx {
    graph: StackGraph,
    partials: PartialPaths,
    paths: Vec<PartialPath>,
}

impl StackGraphCtx {
    fn new(graph: StackGraph, partials: PartialPaths, paths: Vec<PartialPath>) -> Self {
        Self { graph, partials, paths }
    }

    pub fn build(source: &str, filename: &str) -> Result<StackGraphCtx, BuildError> {
        let mut graph = StackGraph::new();
        let mut partials = PartialPaths::new();
        let mut paths = Vec::new();

        let file = graph.get_or_create_file(filename);

        let lang = Lang::from_filename(filename).unwrap();

        lang.sg_config().sgl.build_stack_graph_into(
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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, EnumString, AsRefStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum DepKind {
    Use,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileEndpoint {
    pub filename: String,
    pub byte: usize,
}

impl FileEndpoint {
    pub fn new(filename: String, byte: usize) -> Self {
        Self { filename, byte }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Dep<E> {
    pub src: E,
    pub tgt: E,
    pub kind: DepKind,
    pub byte: usize,
}

impl<E> Dep<E> {
    pub fn new(src: E, tgt: E, kind: DepKind, byte: usize) -> Self {
        Self { src, tgt, kind, byte }
    }
}

impl<E: Eq> Dep<E> {
    pub fn is_loop(&self) -> bool {
        self.src == self.tgt
    }
}

pub type FileDep = Dep<FileEndpoint>;

pub type EntityDep = Dep<EntityId>;

impl FileDep {
    pub fn to_entity_dep(&self, src_set: &EntitySet, tgt_set: &EntitySet) -> EntityDep {
        let src = src_set.get_by_byte(self.src.byte);
        let tgt = tgt_set.get_by_byte(self.tgt.byte);
        Dep::new(src, tgt, self.kind, self.byte)
    }
}

pub fn resolve(ctx: &mut StackGraphCtx) -> Vec<FileDep> {
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
    let byte = |n: Handle<Node>| Loc::from_span(&ctx.graph.source_info(n).unwrap().span).start_byte;

    references
        .iter()
        .map(|r| {
            Dep::new(
                FileEndpoint::new(filename(r.start_node), byte(r.start_node)),
                FileEndpoint::new(filename(r.end_node), byte(r.end_node)),
                DepKind::Use,
                byte(r.start_node),
            )
        })
        .collect()
}
