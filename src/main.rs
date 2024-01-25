use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use git2::Oid;
use git2::Repository;
use log::LevelFilter;

use crate::core::FileSource;
use crate::git::GitCommit;
use crate::resolution::ResolutionCtx;
use crate::storage::Store;

mod core;
mod git;
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

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .init();

    let cli = Cli::parse();

    let start = Instant::now();

    let project_root = cli.project_root.unwrap_or(std::env::current_dir()?);

    let repo = Repository::open(project_root)
        .context("current directory must be inside a git repository")?;

    // This is a necessary config for Windows. Even though we never touch the actual
    // filesystem, because libgit2 emulates the behavior of the real git, it will
    // still crash on Windows when encountering especially long paths.
    repo.config()?.set_bool("core.longpaths", true)?;

    let revision = cli
        .revision
        .context("reading from filesystem not yet supported")?;
    let file_source = GitCommit::from_str(&repo, revision)?;
    let keys = file_source.discover()?;

    log::info!("Found {} file(s) at the selected commit.", keys.len());

    let mut store = Store::open(repo.path().join(".neodepends.idx"))?;
    let missing = &store.find_missing(&keys)?;
    log::info!(
        "Processing {} file(s) which were not found in index...",
        missing.len()
    );

    for (i, key) in missing.iter().enumerate() {
        log::info!("[{}/{}] Processing {}...", i + 1, missing.len(), key);
        let content = file_source.load(key)?;
        let content = std::str::from_utf8(&content)?;

        match ResolutionCtx::from_source(&content, &key.filename) {
            Ok(mut ctx) => store.save(key, &mut ctx)?,
            Err(err) => {
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
