use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::thread::available_parallelism;
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use itertools::Itertools;
use log::LevelFilter;

use crate::loading::FileSystem;
use crate::resolution::resolve;
use crate::resolution::StackGraphCtx;
use crate::storage::LoadResponse;
use crate::storage::Store;

mod core;
mod loading;
mod resolution;
mod storage;

const DEFAULT_INDEX_FILE: &str = ".neodepends.db";

/// Scan a project and extract structural dependency information
///
/// If the project is a git repository, rather than pulling files from disk,
/// Neodepends can optionally scan the project as it existed in a previous
/// revision with the `--revision` option.
///
/// Neodepends relies on an index file to store already scanned files. Only
/// files that are new or that have been modified since the last scan need to be
/// processed. This provides signifigant performance benifits when scanning the
/// project many times (for instance, at different revisions or after a small
/// change).
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// The root of the project to scan
    ///
    /// Defaults to the current working directory.
    #[arg(short, long)]
    project_root: Option<PathBuf>,

    /// The index to store and retrieve values while scanning
    ///
    /// Defaults to `.git/.neodepends.db` or `.neodepends.db` if the project
    /// root is not a git repository. Will be created if not found.
    #[arg(short, long)]
    index_file: Option<PathBuf>,

    /// Delete the index before scanning.
    #[arg(short, long)]
    clean: bool,

    /// A commit to scan instead of the files on disk
    ///
    /// If not specified, will scan recursively from the project root. May be
    /// specified in many formats including human-readable and sha-1 hash, e.g.
    /// "main", "origin/main", "refs/heads/main",
    /// "27b5926474deffa7c77df5a3279bad631c6404f0", etc.
    #[arg(short, long)]
    commit: Option<String>,

    /// Number of threads to use when processing files
    ///
    /// If 0, Neodepends will set this automatically (typically as the number of
    /// CPU cores)
    #[arg(short, long, default_value_t = 0)]
    num_threads: usize,
}

fn project_root(project_root: Option<PathBuf>) -> Result<PathBuf> {
    Ok(project_root.unwrap_or(std::env::current_dir()?))
}

fn index_file<P: AsRef<Path>>(index_file: Option<PathBuf>, project_root: P) -> Result<PathBuf> {
    Ok(index_file.unwrap_or_else(|| {
        let git_dir = project_root.as_ref().join(".git");
        let preferred = git_dir.join(DEFAULT_INDEX_FILE);
        let fallback = project_root.as_ref().join(DEFAULT_INDEX_FILE);

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

fn num_threads(num_threads: usize) -> Result<NonZeroUsize> {
    Ok(match NonZeroUsize::new(num_threads) {
        Some(n) => n,
        _ => available_parallelism()?,
    })
}

struct ProcessedArgs {
    project_root: PathBuf,
    index_file: PathBuf,
    clean: bool,
    commit: Option<String>,
    num_threads: NonZeroUsize,
}

impl ProcessedArgs {
    fn process(args: Args) -> Result<Self> {
        let project_root = project_root(args.project_root)?;
        let index_file = index_file(args.index_file, &project_root)?;
        let num_threads = num_threads(args.num_threads)?;

        Ok(Self {
            project_root,
            index_file,
            clean: args.clean,
            commit: args.commit,
            num_threads,
        })
    }

    fn num_per_thread(&self, total: usize) -> usize {
        (total + self.num_threads.get() - 1) / self.num_threads
    }
}

fn delete_file<P: AsRef<Path>>(path: P) -> Result<()> {
    let path = path.as_ref();

    if path.exists() {
        std::fs::remove_file(path)?;

        if path.exists() {
            bail!("failed to remove {}", path.to_string_lossy());
        }
    }

    Ok(())
}

fn clean<P: AsRef<Path>>(index_file: P) -> Result<()> {
    let index_file = index_file.as_ref();

    if index_file.exists() {
        log::info!("Removing existing index file...");
        delete_file(index_file)?;
        delete_file(index_file.to_str().unwrap().to_owned() + "-shm")?;
        delete_file(index_file.to_str().unwrap().to_owned() + "-wal")?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .init();

    let cli = ProcessedArgs::process(Args::parse())?;

    if cli.clean {
        clean(&cli.index_file)?;
    }

    let start = Instant::now();

    let fs = FileSystem::open(&cli.project_root, &cli.commit)?;
    let keys = fs.ls()?;
    log::info!("Found {} file(s) at the selected commit.", keys.len());

    let store = Store::open(&cli.index_file)?;
    let missing = store.find_missing(&keys)?;
    log::info!(
        "Processing {} file(s) which were not found in index...",
        missing.len()
    );

    let mut handles: Vec<JoinHandle<Result<()>>> = Vec::new();
    let num_per_thread = cli.num_per_thread(missing.len());

    for chunk in &missing.into_iter().chunks(num_per_thread) {
        let chunk = chunk.collect::<Vec<_>>();
        let project_root = cli.project_root.clone();
        let index_file = cli.index_file.clone();
        let commit = cli.commit.clone();

        handles.push(thread::spawn(move || {
            let fs = FileSystem::open(project_root, &commit)?;
            let store = Store::open(index_file)?;

            for key in &chunk {
                log::info!("Processing {}...", key);
                let content = fs.load_file(key)?;
                let content = std::str::from_utf8(&content)?;
                let res = StackGraphCtx::build(&content, &key.filename);

                if res.is_err() {
                    log::warn!("Failed to build stack graph for {}", key);
                }

                store.save(&key, res.map_err(|err| err.to_string()))?;
            }

            Ok(())
        }));
    }

    for handle in handles {
        handle
            .join()
            .expect("could not join thread")
            .context("error in thread")?;
    }

    log::info!("Loading resolution context for all {} files...", keys.len());
    let LoadResponse { mut ctx, failures } = store.load(&keys)?;

    if failures.len() > 0 {
        log::warn!(
            "The following {} files have failed to be built into stack graphs and therefore will \
             not be considered during dependency resolution:\n{}",
            failures.len(),
            failures.keys().sorted().map(|k| k.to_string()).join("\n")
        );
    }

    log::info!("Resolving all references...");
    let deps = resolve(&mut ctx).into_iter().collect::<HashSet<_>>();

    log::info!("Writing output...");
    for (src, tgt) in deps.iter().sorted() {
        if src != tgt {
            println!("{} -> {}", src, tgt);
        }
    }

    log::info!("Finished in {}ms", start.elapsed().as_millis());
    Ok(())
}
