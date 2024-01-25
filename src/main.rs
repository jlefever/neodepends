use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::bail;
use anyhow::Result;
use clap::Parser;
use git2::Repository;
use log::LevelFilter;

use crate::loading::DiskFileLoader;
use crate::loading::FileLoader;
use crate::loading::GitFileLoader;
use crate::resolution::ResolutionCtx;
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
    if cli.clean {
        if index_file.exists() {
            log::info!("Removing existing index file...");
            std::fs::remove_file(&index_file)?;
        }

        if index_file.exists() {
            bail!("failed to remove {}", index_file.to_string_lossy());
        }
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

    let mut keys = file_loader.discover()?;
    keys.sort();
    log::info!("Found {} file(s) at the selected commit.", keys.len());

    let mut store = Store::open(&index_file)?;
    let missing = &store.find_missing(&keys)?;
    let mut missing = missing.into_iter().collect::<Vec<_>>();
    missing.sort();
    log::info!(
        "Processing {} file(s) which were not found in index...",
        missing.len()
    );

    for (i, key) in missing.iter().enumerate() {
        log::info!("[{}/{}] Processing {}...", i + 1, missing.len(), key);
        let content = file_loader.load(key)?;
        let content = std::str::from_utf8(&content)?;

        match ResolutionCtx::from_source(&content, &key.filename) {
            Ok(mut ctx) => store.save(key, &mut ctx)?,
            Err(_) => {
                // log::warn!("Failed to process {} [{}]", key, err);
                store.save(key, &mut ResolutionCtx::dummy(&key.filename)?)?;
            }
        };
    }

    log::info!("Loading resolution context for all {} files...", keys.len());
    let mut ctx = store.load(&keys)?;

    log::info!("Resolving all references...");
    let deps = ctx.resolve().into_iter().collect::<HashSet<_>>();

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
