use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::bail;
use anyhow::Result;
use clap::Parser;
use git2::Repository;
use itertools::Itertools;
use log::LevelFilter;

use crate::loading::DiskFileLoader;
use crate::loading::FileLoader;
use crate::loading::GitFileLoader;
use crate::resolution::resolve;
use crate::resolution::StackGraphCtx;
use crate::storage::LoadResponse;
use crate::storage::Store;

mod core;
mod loading;
mod resolution;
mod storage;

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
struct Cli {
    /// The root of the project to scan
    ///
    /// Defaults to the current working directory.
    #[arg(short, long)]
    project_root: Option<PathBuf>,

    /// The index to store and retrieve values while scanning
    ///
    /// Defaults to `.neodepends.idx`. Will be created if not found.
    #[arg(short, long)]
    index_file: Option<PathBuf>,

    /// Delete the index before scanning.
    #[arg(short, long)]
    clean: bool,

    /// The revision to scan
    ///
    /// If not specified, will scan recursively from the project root.
    #[arg(short, long)]
    revision: Option<String>,
}

impl Cli {
    fn project_root(&self) -> Result<PathBuf> {
        Ok(self
            .project_root
            .clone()
            .unwrap_or(std::env::current_dir()?))
    }

    fn index_file(&self) -> Result<PathBuf> {
        let project_root = self.project_root()?;
        let git_dir = project_root.join(".git");
        let preferred = git_dir.join(".neodepends.idx");
        let fallback = project_root.join(".neodepends.idx");

        if preferred.exists() {
            Ok(preferred)
        } else if fallback.exists() {
            Ok(fallback)
        } else if git_dir.exists() {
            Ok(preferred)
        } else {
            Ok(fallback)
        }
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

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    let project_root = cli.project_root()?;
    let index_file = cli.index_file()?;
    log::info!("Project Root: {}", project_root.to_string_lossy());
    log::info!("Index File: {}", index_file.to_string_lossy());

    // Clean if requested
    if cli.clean && index_file.exists() {
        log::info!("Removing existing index file...");
        delete_file(&index_file)?;
        delete_file(&index_file.with_extension("idx-shm"))?;
        delete_file(&index_file.with_extension("idx-wal"))?;
    }

    let repo = Repository::open(&project_root).ok();

    if repo.is_none() && cli.revision.is_some() {
        bail!("a revision was supplied but the project root does not refer to a git repository")
    }

    let file_loader: Box<dyn FileLoader> = if cli.revision.is_none() {
        Box::new(DiskFileLoader::new(project_root.clone()))
    } else {
        let repo = repo.as_ref().unwrap();

        // This is a necessary config for Windows
        repo.config()?.set_bool("core.longpaths", true)?;

        Box::new(GitFileLoader::from_str(repo, cli.revision.unwrap())?)
    };

    let start = Instant::now();

    let keys = file_loader.discover()?;
    log::info!("Found {} file(s) at the selected commit.", keys.len());

    let mut store = Store::open(&index_file)?;
    let missing = &store.find_missing(&keys)?;
    log::info!(
        "Processing {} file(s) which were not found in index...",
        missing.len()
    );

    for (i, key) in missing.iter().enumerate() {
        log::info!("[{}/{}] Processing {}...", i + 1, missing.len(), key);
        let content = file_loader.load(key)?;
        let content = std::str::from_utf8(&content)?;
        let res = StackGraphCtx::build(&content, &key.filename);

        if res.is_err() {
            log::warn!("Failed to build stack graph for {}", key);
        }

        store.save(key, res.map_err(|err| err.to_string()))?;
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
    let mut deps = deps.into_iter().collect::<Vec<_>>();
    deps.sort();

    for (src, tgt) in deps {
        if src != tgt {
            println!("{} -> {}", src, tgt);
        }
    }

    log::info!("Finished in {}ms", start.elapsed().as_millis());
    Ok(())
}
