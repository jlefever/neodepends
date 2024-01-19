use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use git2::Commit;
use git2::Oid;
use git2::Repository;
use git2::TreeWalkMode;
use git2::TreeWalkResult;
use stack_graphs::arena::Handle;
use stack_graphs::graph::Node;
use stack_graphs::graph::StackGraph;

use crate::resolution::ResolutionCtx;
use crate::storage::Store;
use crate::storage::StoreKey;

mod resolution;
mod storage;

fn parse_rev<'a>(repo: &'a Repository, rev: &'a str) -> Result<Commit<'a>> {
    if let Ok(rev) = repo.resolve_reference_from_short_name(rev) {
        Ok(rev.peel_to_commit()?)
    } else if let Ok(oid) = Oid::from_str(rev) {
        Ok(repo.find_commit(oid)?)
    } else {
        bail!(
            "the given revision ('{}') was not found in this repository",
            rev
        );
    }
}

fn main() -> anyhow::Result<()> {
    let repo = Repository::discover(std::env::current_dir()?)
        .context("current directory must be inside a git repository")?;

    // This is a necessary config for Windows. Even though we never touch the actual
    // filesystem, because libgit2 emulates the behavior of the real git, it will
    // still crash on Windows when encountering especially long paths.
    repo.config()?.set_bool("core.longpaths", true)?;

    // Get store keys
    let mut keys = Vec::new();
    parse_rev(&repo, "master")?
        .tree()?
        .walk(TreeWalkMode::PreOrder, |dir, entry| {
            let path = dir.to_string() + entry.name().unwrap();

            if path.ends_with(".java") {
                keys.push(StoreKey::new(entry.id().to_string(), path));
            }

            TreeWalkResult::Ok
        })?;

    // Open database in default directory
    let mut store = Store::open(repo.path().join("neodepends.db"))?;

    for key in &store.find_missing(&keys)? {
        println!("Missing {}", key);
        let oid = Oid::from_str(&key.oid)?;
        let blob = repo.find_blob(oid)?;
        let content = blob.content();
        let content = std::str::from_utf8(content)?;

        if let Ok(mut ctx) = ResolutionCtx::from_source(&content, &key.filename) {
            store.save(key, &mut ctx)?;
        } else {
            store.save(key, &mut ResolutionCtx::dummy(&key.filename)?)?;
        }
    }

    println!("Loading...");
    let mut ctx = store.load(&keys)?;

    println!("Resolving...");
    let deps = ctx.resolve().into_iter().collect::<HashSet<_>>();

    println!("Printing...");
    let mut deps = deps.into_iter().collect::<Vec<_>>();
    deps.sort();

    for (src, tgt) in deps {
        if src != tgt {
            println!("{} -> {}", src, tgt);
        }
    }

    Ok(())
}
