#![feature(map_try_insert)]
#[macro_use]
extern crate derive_builder;

use core::CommitId;
use core::Diff;
use core::FileDep;
use core::FileKey;
use core::Tag;
use core::TagDep;
use core::TagId;
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

use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use clap::ValueEnum;
use clap_verbosity_flag::InfoLevel;
use clap_verbosity_flag::Verbosity;
use counter::Counter;
use entities::extract_tag_set;
use entities::TagSet;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use indicatif_log_bridge::LogWrapper;
use itertools::Itertools;
use languages::Lang;

use crate::changes::DiffCalculator;
use crate::core::Change;
use crate::core::EntityId;
use crate::depends::Depends;
use crate::loading::FileFilter;
use crate::loading::FileSystem;
use crate::output::OutputV1;
use crate::output::OutputV2;
use crate::stackgraphs::build_stack_graph;
use crate::stackgraphs::resolve;
use crate::storage::LoadResponse;
use crate::storage::Store;

mod changes;
mod core;
mod depends;
mod entities;
mod languages;
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

    /// The directory to act as the cache. Will be created if not found.
    ///
    /// Defaults to `{project_root}/.neodepends`.
    #[arg(short, long)]
    cache_dir: Option<PathBuf>,

    /// Delete the cache before scanning
    #[arg(long)]
    clean: bool,

    /// Enable the provided langauges
    #[arg(short, long, value_delimiter = ' ', default_values_t = Lang::all())]
    langs: Vec<Lang>,

    /// Method to use to resolve dependencies between files or entities when
    /// needed
    #[arg(long, default_value_t = Resolver::StackGraphs)]
    resolver: Resolver,

    /// Extract entities and dependencies from this commit instead of the files
    /// on disk
    ///
    /// If not specified, will scan recursively from the project root. Can be a
    /// reference (e.g. "main", "origin/main", etc.) or a SHA-1 hash.
    #[arg(long)]
    commit: Option<String>,

    /// Enable history extraction
    ///
    /// May optionally provide a newline delimited list of commits (specified as
    /// SHA-1 hashes) where change information will be extracted from. If not
    /// provided, will scan all commits reachable from `--commit`. If `--commit`
    /// has not been specified, will scan all commits reachable from HEAD.
    ///
    /// This option is intended to work with git rev-list. For instance
    /// `--history="$(git rev-list --since=1year HEAD)"` will include all
    /// commits from the past year that are reachable from HEAD.
    ///
    /// Can also load a list of commits from a file, e.g.,
    /// `--history=history.txt`
    #[arg(long)]
    history: Option<Option<String>>,

    /// Method to use to resolve dependencies between files or entities
    #[arg(long, default_value_t = DumpFormat::JsonV2)]
    format: DumpFormat,

    /// Name field in JSONv1 output
    ///
    /// Defaults to the last component of the project root.
    #[arg(long)]
    name: Option<String>,

    /// Number of threads to use when processing files
    ///
    /// If 0, this will be set automatically (typically as the number of CPU
    /// cores)
    #[arg(short, long, default_value_t = 0)]
    num_threads: usize,

    /// Path to the depends.jar that is used for Depends dependency resolution
    ///
    /// If not provided, will look for depends.jar in the same directory as this
    /// executable.
    #[arg(long)]
    depends_jar: Option<PathBuf>,

    /// Java executable used for running depends.jar
    ///
    /// If not provided, will assume "java" is on the system path
    #[arg(long)]
    depends_java: Option<PathBuf>,

    /// Maximum size of the Java memory allocation pool when running Depends.
    /// Passed with "-Xmx" to the Java executable. Useful for large projects
    /// that cause Depends to run out of memory. For example, "12G" for a twelve
    /// gigabyte memory allocation pool.
    #[arg(long, default_value = "4G")]
    depends_xmx: String,

    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
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
}

impl Display for DumpFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JsonV1 => write!(f, "jsonv1"),
            Self::JsonV2 => write!(f, "jsonv2"),
        }
    }
}

fn project_root(project_root: Option<PathBuf>) -> Result<PathBuf> {
    Ok(project_root.unwrap_or(std::env::current_dir()?))
}

fn cache_dir<P: AsRef<Path>>(cache_dir: Option<PathBuf>, project_root: P) -> Result<PathBuf> {
    Ok(cache_dir.unwrap_or_else(|| project_root.as_ref().join(DEFAULT_CACHE_DIR)))
}

fn history(history: Option<Option<String>>) -> Option<Vec<CommitId>> {
    history.map(|x| x.iter().flat_map(|x| x.lines().flat_map(CommitId::from_str)).collect())
}

fn num_threads(num_threads: usize) -> Result<NonZeroUsize> {
    Ok(match NonZeroUsize::new(num_threads) {
        Some(n) => n,
        _ => available_parallelism()?,
    })
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

struct ProcessedCli {
    project_root: PathBuf,
    cache_dir: PathBuf,
    clean: bool,
    langs: HashSet<Lang>,
    resolver: Resolver,
    commit: Option<String>,
    history: Option<Vec<CommitId>>,
    format: DumpFormat,
    name: String,
    num_threads: NonZeroUsize,
    depends_jar: Option<PathBuf>,
    depends_java: Option<PathBuf>,
    depends_xmx: Option<String>,
}

impl ProcessedCli {
    fn from(cli: &Cli) -> Result<Self> {
        let project_root = project_root(cli.project_root.clone())?;
        let cache_dir = cache_dir(cli.cache_dir.clone(), &project_root)?;
        let langs = cli.langs.iter().map(|&x| x).collect();
        let name = name(cli.name.clone(), &project_root);
        let num_threads = num_threads(cli.num_threads)?;

        Ok(Self {
            project_root,
            cache_dir,
            clean: cli.clean,
            langs: langs,
            resolver: cli.resolver,
            commit: cli.commit.clone(),
            history: history(cli.history.clone()),
            format: cli.format,
            name,
            num_threads,
            depends_jar: cli.depends_jar.clone(),
            depends_java: cli.depends_java.clone(),
            depends_xmx: Some(cli.depends_xmx.clone()),
        })
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let logger = env_logger::Builder::new().filter_level(cli.verbose.log_level_filter()).build();
    let args = ProcessedCli::from(&cli)?;
    let multi_progress = MultiProgress::new();
    LogWrapper::new(multi_progress.clone(), logger).try_init().unwrap();

    if args.clean && args.cache_dir.exists() {
        log::info!("Deleting existing cache...");
        remove_dir_all(&args.cache_dir)?;
    }

    let start = Instant::now();

    let filter = FileFilter::from_langs(args.langs.clone());
    let fs = FileSystem::open(&args.project_root, &args.commit, &filter)?;
    let deps = collect_file_deps(&args, fs.clone(), multi_progress)?;

    let mut sets = collect_tag_sets(fs.clone());
    let mut changes = Vec::new();

    if let Some(history) = args.history {
        let repo = fs.repo().expect("must be a git repository to use `--history` flag");
        let dc = DiffCalculator::new(repo);
        let filter = FileFilter::from_filenames(fs.list().iter().map(|f| f.filename.clone()));
        let diffs = history.into_iter().flat_map(|c| dc.diff(c, &filter).unwrap()).collect_vec();
        changes.extend(collect_changes(&fs, &mut sets, diffs));
    }

    let text = match args.format {
        DumpFormat::JsonV1 => {
            let filenames = fs.list().iter().map(|k| k.filename.clone()).collect();
            let output = OutputV1::build(&args.name, filenames, deps)?;
            serde_json::to_string_pretty(&output)?
        }
        DumpFormat::JsonV2 => {
            let deps = to_tag_deps(&deps, &sets);
            let tags = flatten_tag_sets(&sets);
            let changes = to_tag_changes(&tags, &changes);
            let output = OutputV2::build(tags, deps, changes)?;
            serde_json::to_string_pretty(&output)?
        }
    };

    log::info!("Writing output...");
    println!("{}", text);
    log::info!("Finished in {}ms.", start.elapsed().as_millis());
    Ok(())
}

fn to_tag_changes(tags: &[Tag], changes: &[Change<EntityId>]) -> Vec<Change<TagId>> {
    let mut tag_changes = Vec::new();
    let lookup = tags.iter().map(|t| (t.entity.id, t.id)).into_group_map();

    for change in changes {
        if let Some(tag_ids) = lookup.get(&change.target_id) {
            for tag_id in tag_ids {
                tag_changes.push(change.with_id(*tag_id));
            }
        }
    }

    tag_changes
}

fn collect_changes(
    fs: &FileSystem,
    sets: &mut HashMap<FileKey, TagSet>,
    diffs: Vec<Diff>,
) -> Vec<Change<EntityId>> {
    let mut changes = Vec::new();

    for diff in diffs {
        let change_kind = diff.change_kind();

        let old_counts: Counter<EntityId> = diff
            .old
            .map(|k| sets.entry(k.clone()).or_insert_with_key(|k| extract_tag_set(&fs, k)))
            .into_iter()
            .flat_map(|s| diff.hunks.iter().flat_map(|h| s.find_entity_ids(h.old)))
            .collect();

        let new_counts: Counter<EntityId> = diff
            .new
            .map(|k| sets.entry(k.clone()).or_insert_with_key(|k| extract_tag_set(&fs, k)))
            .into_iter()
            .flat_map(|s| diff.hunks.iter().flat_map(|h| s.find_entity_ids(h.new)))
            .collect();

        for id in old_counts.keys().chain(new_counts.keys()).unique() {
            let dels = old_counts[id];
            let adds = new_counts[id];
            changes.push(Change::new(*id, diff.commit_id, change_kind, adds, dels));
        }
    }

    changes
}

fn flatten_tag_sets(tag_sets: &HashMap<FileKey, TagSet>) -> Vec<Tag> {
    let mut tags = Vec::new();

    for file_key in tag_sets.keys().sorted() {
        for tag in tag_sets[file_key].tags() {
            tags.push(tag.clone());
        }
    }

    tags
}

fn collect_tag_sets(fs: FileSystem) -> HashMap<FileKey, TagSet> {
    let mut map = HashMap::with_capacity(fs.list().len());

    for key in fs.list() {
        map.insert(key.clone(), extract_tag_set(&fs, key));
    }

    map
}

fn to_tag_deps(deps: &[FileDep], entity_sets: &HashMap<FileKey, TagSet>) -> Vec<TagDep> {
    let mut entity_deps = Vec::new();

    for dep in deps {
        let src_set = entity_sets.get(&dep.src.file_key);
        let tgt_set = entity_sets.get(&dep.tgt.file_key);

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
    args: &ProcessedCli,
    fs: FileSystem,
    progress: MultiProgress,
) -> Result<Vec<FileDep>> {
    Ok(match args.resolver {
        Resolver::Depends => Depends::new(
            args.depends_jar.clone(),
            args.depends_java.clone(),
            args.depends_xmx.clone(),
        )
        .resolve(&fs)?,
        Resolver::StackGraphs => {
            let (deps, _) =
                collect_file_deps_sg(fs, &args.cache_dir, args.num_threads.into(), progress)?;
            deps
        }
    })
}

fn collect_file_deps_sg(
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
                    store.lock().unwrap().save(&key, res)?;
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

    // if failures.len() > 0 {
    //     log::warn!(
    //         "The following {} files have failed to be built into stack graphs and therefore will \
    //          not be considered during dependency resolution:\n{}",
    //         failures.len(),
    //         failures.iter().sorted().map(|k| &k.filename).join("\n")
    //     );
    // }

    log::info!("Resolving all references...");
    Ok((resolve(&mut ctx, &fs), failures))
}

fn num_per_thread(num_threads: usize, total: usize) -> usize {
    (total + num_threads - 1) / num_threads
}
