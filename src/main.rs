use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::bail;
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

    let repo = Repository::open(project_root)
        .context("current directory must be inside a git repository")?;

    // This is a necessary config for Windows. Even though we never touch the actual
    // filesystem, because libgit2 emulates the behavior of the real git, it will
    // still crash on Windows when encountering especially long paths.
    repo.config()?.set_bool("core.longpaths", true)?;

    let start = Instant::now();

    let revision = cli
        .revision
        .context("reading from filesystem not yet supported")?;
    let file_source = GitCommit::from_str(&repo, revision)?;
    let keys = file_source.discover()?;

    log::info!("Found {} file(s) at the selected commit.", keys.len());

    let mut store = Store::open(&index_file)?;
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

fn main2() -> anyhow::Result<()> {
    let project_dir = "/Users/jtl86/source/java/depends2";
    let file_path = "src/main/java/depends/generator/DependencyGenerator.java";
    let blob_oid = "b1bc4a8366988c0de8baa260114cee5d73374266";

    let repo = Repository::discover(project_dir).unwrap();
    let oid = Oid::from_str(blob_oid)?;
    let blob = repo.find_blob(oid)?;

    println!(
        "{:?}",
        Oid::hash_object(git2::ObjectType::Blob, blob.content())?
    );
    // println!("{}", std::str::from_utf8(blob.content())?);

    let mut buf = Vec::new();
    File::open(Path::new(project_dir).join(file_path))?.read_to_end(&mut buf)?;

    println!("{:?}", Oid::hash_object(git2::ObjectType::Blob, &buf)?);
    // println!("{}", std::str::from_utf8(&buf)?);

    let arr: [u8; 20] = unsafe { std::mem::transmute(oid) };

    println!("{:x?}", arr);

    Ok(())
}
