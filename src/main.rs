#[macro_use]
extern crate derive_builder;

use core::Entity;
use core::EntityDep;
use core::EntityId;
use core::FileDep;
use core::FileKey;
use core::Lang;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Display;
use std::fs::remove_dir_all;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::thread::available_parallelism;
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use clap_verbosity_flag::InfoLevel;
use clap_verbosity_flag::Verbosity;
use entities::extract;
use entities::EntitySet;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use indicatif_log_bridge::LogWrapper;
use itertools::Itertools;

use crate::loading::FileSystem;
use crate::output::OutputV1;
use crate::output::OutputV2;
use crate::stackgraphs::build_stack_graph;
use crate::stackgraphs::resolve;
use crate::storage::LoadResponse;
use crate::storage::Store;

mod core;
mod entities;
mod loading;
mod output;
mod sparse_vec;
mod stackgraphs;
mod storage;

const DEFAULT_CACHE_DIR: &str = ".neodepends";

/// Scan a project and extract structural dependency information
///
/// If the project is a git repository, rather than pulling files from disk,
/// Neodepends can optionally scan the project as it existed in a previous
/// commit with the `--commit` option.
///
/// Neodepends caches files on disk as they are scanned. Only files that are new
/// or that have been modified since the last scan need to be processed. This
/// provides signifigant performance benifits when scanning the project many
/// times (for instance, at different commits or after a small change).
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// The root of the project to scan
    ///
    /// Defaults to the current working directory.
    #[arg(short, long)]
    project_root: Option<PathBuf>,

    /// The directory to act as the cache
    ///
    /// Defaults to `.git/.neodepends` or `.neodepends` if the project
    /// root is not a git repository. Will be created if not found.
    #[arg(short, long)]
    cache_dir: Option<PathBuf>,

    /// Delete the cache before scanning
    #[arg(long)]
    clean: bool,

    /// Enable the provided langauges
    #[arg(short, long, value_delimiter = ' ', default_values_t = EnabledLang::all())]
    langs: Vec<EnabledLang>,

    /// Number of threads to use when processing files
    ///
    /// If 0, this will be set automatically (typically as the number of CPU
    /// cores)
    #[arg(short, long, default_value_t = 0)]
    num_threads: usize,

    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Dump(DumpCommand),
    LsEntities(LsEntitiesCommand),
    LsDeps(LsDepsCommand),
}

/// Export files and dependencies as DV8 JSON
#[derive(Debug, Args)]
struct DumpCommand {
    /// A commit to scan instead of the files on disk
    ///
    /// If not specified, will scan recursively from the project root. Can be a
    /// reference (e.g. "main", "origin/main", etc.) or a SHA-1 hash.
    #[arg(long)]
    commit: Option<String>,

    /// Method to use to resolve dependencies between files or entities
    #[arg(long, default_value_t = Resolver::StackGraphs)]
    resolver: Resolver,

    /// Method to use to resolve dependencies between files or entities
    #[arg(long, default_value_t = DumpFormat::JsonV2)]
    format: DumpFormat,

    /// Name field in JSON output
    ///
    /// Defaults to the last component of the project root.
    #[arg(long)]
    name: Option<String>,
}

/// List all entities found
#[derive(Debug, Args)]
struct LsEntitiesCommand {
    /// Read entities from this commit instead of disk
    ///
    /// If not specified, will scan recursively from the project root. Can be a
    /// reference (e.g. "main", "origin/main", etc.) or a SHA-1 hash.
    #[arg(long)]
    commit: Option<String>,
}

/// List all dependencies found
#[derive(Debug, Args)]
struct LsDepsCommand {
    /// Read dependencies from this commit instead of disk
    ///
    /// If not specified, will scan recursively from the project root. Can be a
    /// reference (e.g. "main", "origin/main", etc.) or a SHA-1 hash.
    #[arg(long)]
    commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
enum EnabledLang {
    Java,
    JavaScript,
    Python,
    TypeScript,
}

impl EnabledLang {
    fn all() -> &'static [Self] {
        &[Self::Java, Self::JavaScript, Self::Python, Self::TypeScript]
    }
}

impl Display for EnabledLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Java => write!(f, "java"),
            Self::JavaScript => write!(f, "javascript"),
            Self::Python => write!(f, "python"),
            Self::TypeScript => write!(f, "typescript"),
        }
    }
}

impl From<&EnabledLang> for Lang {
    fn from(value: &EnabledLang) -> Self {
        match value {
            EnabledLang::Java => Self::Java,
            EnabledLang::JavaScript => Self::JavaScript,
            EnabledLang::Python => Self::Python,
            EnabledLang::TypeScript => Self::TypeScript,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
enum Resolver {
    Depends,
    StackGraphs,
}

impl Display for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Depends => write!(f, "depends"),
            Self::StackGraphs => write!(f, "stackgraphs"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
enum DumpFormat {
    JsonV1,
    JsonV2,
    Sqlite,
}

impl Display for DumpFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JsonV1 => write!(f, "jsonv1"),
            Self::JsonV2 => write!(f, "jsonv2"),
            Self::Sqlite => write!(f, "sqlite"),
        }
    }
}

fn project_root(project_root: Option<PathBuf>) -> Result<PathBuf> {
    Ok(project_root.unwrap_or(std::env::current_dir()?))
}

fn cache_dir<P: AsRef<Path>>(cache_dir: Option<PathBuf>, project_root: P) -> Result<PathBuf> {
    Ok(cache_dir.unwrap_or_else(|| {
        let git_dir = project_root.as_ref().join(".git");
        let preferred = git_dir.join(DEFAULT_CACHE_DIR);
        let fallback = project_root.as_ref().join(DEFAULT_CACHE_DIR);

        if preferred.exists() {
            preferred
        } else if fallback.exists() {
            fallback
        } else if git_dir.exists() {
            preferred
        } else {
            fallback
        }
    }))
}

fn name<P: AsRef<Path>>(name: Option<String>, project_root: P) -> String {
    name.unwrap_or_else(|| {
        project_root
            .as_ref()
            .components()
            .last()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or("".to_string())
    })
}

fn num_threads(num_threads: usize) -> Result<NonZeroUsize> {
    Ok(match NonZeroUsize::new(num_threads) {
        Some(n) => n,
        _ => available_parallelism()?,
    })
}

struct CommonArgs {
    project_root: PathBuf,
    cache_dir: PathBuf,
    clean: bool,
    langs: HashSet<Lang>,
    num_threads: NonZeroUsize,
}

impl CommonArgs {
    fn from(cli: &Cli) -> Result<Self> {
        let project_root = project_root(cli.project_root.clone())?;
        let cache_dir = cache_dir(cli.cache_dir.clone(), &project_root)?;
        let num_threads = num_threads(cli.num_threads)?;

        Ok(Self {
            project_root,
            cache_dir,
            clean: cli.clean,
            langs: cli.langs.iter().map_into().collect(),
            num_threads,
        })
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let logger = env_logger::Builder::new().filter_level(cli.verbose.log_level_filter()).build();
    let args = CommonArgs::from(&cli)?;
    let multi_progress = MultiProgress::new();
    LogWrapper::new(multi_progress.clone(), logger).try_init().unwrap();

    if args.clean && args.cache_dir.exists() {
        log::info!("Deleting existing cache...");
        remove_dir_all(&args.cache_dir)?;
    }

    let start = Instant::now();

    match cli.command {
        Command::Dump(cmd) => dump(args, cmd, multi_progress),
        Command::LsEntities(cmd) => ls_entities(args, cmd),
        Command::LsDeps(cmd) => ls_deps(args, cmd, multi_progress),
    }?;

    log::info!("Finished in {}ms.", start.elapsed().as_millis());
    Ok(())
}

fn dump(args: CommonArgs, cmd: DumpCommand, progress: MultiProgress) -> anyhow::Result<()> {
    let fs = FileSystem::open(&args.project_root, &cmd.commit, &args.langs)?;

    if matches!(cmd.resolver, Resolver::Depends) {
        bail!("depends resolver not yet supported");
    }

    let name = name(cmd.name, &args.project_root);
    let (deps, _) =
        collect_file_deps(fs.clone(), &args.cache_dir, args.num_threads.into(), progress)?;

    let text = match cmd.format {
        DumpFormat::JsonV1 => {
            let filenames = fs.list().iter().map(|k| k.filename.clone()).collect();
            let output = OutputV1::build(&name, filenames, deps)?;
            serde_json::to_string_pretty(&output)?
        }
        DumpFormat::JsonV2 => {
            let entity_sets = collect_entity_sets(fs);
            let entities = flatten_entity_sets(&entity_sets);
            let deps = to_entity_deps(&deps, &entity_sets);
            let output = OutputV2::build(&name, entities, deps)?;
            serde_json::to_string_pretty(&output)?
        }
        DumpFormat::Sqlite => {
            bail!("sqlite output not yet suppported");
        }
    };

    log::info!("Writing output...");
    println!("{}", text);
    Ok(())
}

fn ls_entities(args: CommonArgs, cmd: LsEntitiesCommand) -> anyhow::Result<()> {
    let fs = FileSystem::open(&args.project_root, &cmd.commit, &args.langs)?;
    let entity_sets = collect_entity_sets(fs);
    let entities = flatten_entity_sets(&entity_sets);

    for entity in &entities {
        println!("{}", serde_json::to_string(entity)?);
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash, serde::Serialize)]
struct DepDto {
    src_id: EntityId,
    tgt_id: EntityId,
    kind: String,
    byte: usize,
}

fn ls_deps(args: CommonArgs, cmd: LsDepsCommand, progress: MultiProgress) -> anyhow::Result<()> {
    let fs = FileSystem::open(&args.project_root, &cmd.commit, &args.langs)?;

    let entity_sets = collect_entity_sets(fs.clone());
    let (deps, _) = collect_file_deps(fs, &args.cache_dir, args.num_threads.into(), progress)?;

    for dep in to_entity_deps(&deps, &entity_sets) {
        if !dep.is_loop() {
            println!("{}", serde_json::to_string(&dep)?);
        }
    }

    Ok(())
}

fn flatten_entity_sets(entity_sets: &HashMap<String, EntitySet>) -> Vec<Entity> {
    let mut entities = Vec::new();

    for filename in entity_sets.keys().sorted() {
        for entity in entity_sets[filename].entities() {
            entities.push(entity.clone());
        }
    }

    entities
}

fn collect_entity_sets(fs: FileSystem) -> HashMap<String, EntitySet> {
    let mut map = HashMap::with_capacity(fs.list().len());

    for key in fs.list() {
        if let Some(entity_set) = extract(fs.clone(), &key.filename) {
            map.insert(key.filename.clone(), entity_set);
        } else {
            log::warn!("Failed to extract entities from {}", key.filename);
        }
    }

    map
}

fn to_entity_deps(deps: &[FileDep], entity_sets: &HashMap<String, EntitySet>) -> Vec<EntityDep> {
    let mut entity_deps = Vec::new();

    for dep in deps {
        let src_set = entity_sets.get(&dep.src.filename);
        let tgt_set = entity_sets.get(&dep.tgt.filename);

        if src_set.is_none() || tgt_set.is_none() {
            continue;
        }

        let dep = dep.to_entity_dep(src_set.unwrap(), tgt_set.unwrap());

        if !dep.is_loop() {
            entity_deps.push(dep);
        }
    }

    entity_deps
}

fn collect_file_deps(
    fs: FileSystem,
    cache_dir: &Path,
    num_threads: usize,
    progress: MultiProgress,
) -> anyhow::Result<(Vec<FileDep>, HashSet<FileKey>)> {
    let store = Arc::new(Mutex::new(Store::open(cache_dir)?));
    let missing = store.lock().unwrap().find_missing(fs.list());

    if missing.len() > 0 {
        log::info!("Processing {} file(s) which were not found in the cache...", missing.len());
        let bar = progress.add(ProgressBar::new(missing.len() as u64)).with_style(
            ProgressStyle::with_template(
                "[{elapsed_precise}] {bar:40} {pos}/{len} (ETA: {eta_precise}) {msg}",
            )?,
        );

        let mut handles: Vec<JoinHandle<Result<()>>> = Vec::new();
        let num_per_thread = num_per_thread(num_threads, missing.len());

        for chunk in &missing.into_iter().chunks(num_per_thread) {
            let chunk = chunk.collect::<Vec<_>>();
            let fs = fs.clone();
            let store = store.clone();
            let bar = bar.clone();

            handles.push(thread::spawn(move || {
                for key in &chunk {
                    let msg = format!("Processing {}...", &key.filename);
                    log::debug!("{}", msg);
                    bar.set_message(msg);
                    let bytes = fs.load(key)?;
                    let content = std::str::from_utf8(&bytes)?;
                    let res = build_stack_graph(&content, &key.filename);

                    // stackgraphs is not supported for this language
                    if res.is_none() {
                        continue;
                    }

                    store.lock().unwrap().save(&key, res.unwrap().ok())?;
                    bar.inc(1);
                }
                Ok(())
            }));
        }

        for handle in handles {
            handle.join().expect("could not join thread").context("error in thread")?;
        }

        bar.finish();
        progress.remove(&bar);
    }

    log::info!("Loading stack graphs for all {} files...", fs.list().len());
    let LoadResponse { mut ctx, failures } = store.lock().unwrap().load(fs.list())?;

    if failures.len() > 0 {
        log::warn!(
            "The following {} files have failed to be built into stack graphs and therefore will \
             not be considered during dependency resolution:\n{}",
            failures.len(),
            failures.iter().sorted().map(|k| &k.filename).join("\n")
        );
    }

    log::info!("Resolving all references...");
    Ok((resolve(&mut ctx), failures))
}

fn num_per_thread(num_threads: usize, total: usize) -> usize {
    (total + num_threads - 1) / num_threads
}
