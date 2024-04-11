//! Used to interface with Depends
//! 
//! See https://github.com/multilang-depends/depends

use std::collections::HashSet;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::Context;
use anyhow::Result;
use itertools::Itertools;
use serde::Deserialize;
use subprocess::Exec;
use subprocess::Redirection;
use tempfile::TempDir;

use crate::core::FileDep;
use crate::core::FileKey;
use crate::core::FileSet;
use crate::core::FilenameDep;
use crate::core::FilenameEndpoint;
use crate::core::PartialPosition;
use crate::core::PseudoCommitId;
use crate::languages::Lang;
use crate::resolution::Resolver;
use crate::resolution::ResolverFactory;

/// All options needed to run Depends.
#[derive(Debug, Clone)]
pub struct DependsConfig {
    /// The path to depends.jar.
    jar: Option<PathBuf>,

    /// The path to the Java executable to run depends.jar.
    java: Option<PathBuf>,

    /// The "-Xmx" value to be passed to the Java executable.
    xmx: Option<String>,
}

impl DependsConfig {
    pub fn new(jar: Option<PathBuf>, java: Option<PathBuf>, xmx: Option<String>) -> Self {
        Self { jar, java, xmx }
    }
}

/// A Depends resolver.
///
/// Works by using a temporary directory.
///
/// See [Resolver].
#[derive(Debug)]
pub struct DependsResolver {
    commit_id: PseudoCommitId,
    depends_lang: String,
    config: DependsConfig,
    temp_dir: TempDir,
    files: RwLock<HashSet<FileKey>>,
}

impl DependsResolver {
    fn new(commit_id: PseudoCommitId, depends_lang: String, config: DependsConfig) -> Self {
        Self {
            commit_id,
            depends_lang,
            config,
            temp_dir: TempDir::new().unwrap(),
            files: Default::default(),
        }
    }
}

impl Resolver for DependsResolver {
    fn add_file(&self, filename: &str, content: &str) {
        let path = self.temp_dir.as_ref().join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create_new(path).unwrap().write_all(content.as_bytes()).unwrap();
        let file_key = FileKey::from_content(filename.to_string(), content);
        self.files.write().unwrap().insert(file_key);
    }

    fn resolve(&self) -> Vec<FileDep> {
        let file_set = FileSet::new(self.files.read().unwrap().iter().map(|x| x.clone()));
        log::info!("Running Depends on {} file(s)...", &self.depends_lang);
        run(&self.config, &self.temp_dir, &self.depends_lang).unwrap();
        log::info!("Loading Depends {} output...", &self.depends_lang);
        load_depends_output(&self.temp_dir, &self.depends_lang)
            .unwrap()
            .iter_filename_deps(self.commit_id)
            .map(|d| d.into_file_dep(&file_set).unwrap())
            .collect_vec()
    }
}

/// A Depends resolver factory.
///
/// See [ResolverFactory].
#[derive(Debug, Clone)]
pub struct DependsResolverFactory {
    config: DependsConfig,
}

impl DependsResolverFactory {
    pub fn new(config: DependsConfig) -> Self {
        Self { config }
    }
}

impl ResolverFactory for DependsResolverFactory {
    fn try_create(&self, commit_id: PseudoCommitId, lang: Lang) -> Option<Box<dyn Resolver>> {
        lang.depends_lang().map(|l| {
            Box::new(DependsResolver::new(commit_id, l.to_string(), self.config.clone()))
                as Box<dyn Resolver>
        })
    }
}

fn run<P: AsRef<Path>>(config: &DependsConfig, dir: P, depends_lang: &str) -> Result<()> {
    let mut cmd = Exec::cmd(config.java.clone().unwrap_or("java".into()));

    if let Some(xmx) = &config.xmx {
        cmd = cmd.arg(format!("-Xmx{xmx}"));
    }

    let status = cmd
        .arg("-jar")
        .arg(&get_depends_jar(config.jar.clone())?)
        .arg(depends_lang)
        .arg(".")
        .arg(format!("deps-{}", depends_lang))
        .arg("--detail")
        .arg("--output-self-deps")
        .arg("--granularity=structure")
        .arg("--namepattern=unix")
        .arg("--strip-leading-path")
        .stdout(Redirection::Merge)
        .cwd(dir)
        .join()?;

    if !status.success() {
        log::warn!("Depends terminated with a non-zero exit code");
    }

    Ok(())
}

fn load_depends_output<P: AsRef<Path>>(dir: P, depends_lang: &str) -> Result<DependsOutput> {
    let mut buffer = Vec::new();
    let path = format!("deps-{}-structure.json", depends_lang);
    std::fs::File::open(dir.as_ref().join(path))?.read_to_end(&mut buffer)?;
    Ok(serde_json::from_slice(&buffer)?)
}

fn get_depends_jar(jar: Option<PathBuf>) -> Result<PathBuf> {
    Ok(jar.or_else(find_depends_jar).context("could not find depends.jar")?.canonicalize()?)
}

fn find_depends_jar() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        .map(|p| p.join("depends.jar"))
}

#[derive(Debug, Deserialize)]
struct DependsOutput {
    #[serde(rename = "cells")]
    cells: Vec<DependsCell>,
}

impl DependsOutput {
    fn iter_filename_deps(self, commit_id: PseudoCommitId) -> impl Iterator<Item = FilenameDep> {
        self.cells.into_iter().flat_map(move |c| c.iter_filename_deps(commit_id))
    }
}

#[derive(Debug, Deserialize)]
struct DependsCell {
    #[serde(rename = "details")]
    details: Option<Vec<DependsDetail>>,
}

impl DependsCell {
    fn iter_filename_deps(self, commit_id: PseudoCommitId) -> impl Iterator<Item = FilenameDep> {
        self.details
            .into_iter()
            .flat_map(move |d| d.into_iter().map(move |d| d.into_filename_dep(commit_id)))
    }
}

#[derive(Debug, Deserialize)]
struct DependsDetail {
    #[serde(rename = "src")]
    src: DependsEndpoint,

    #[serde(rename = "dest")]
    tgt: DependsEndpoint,

    #[serde(rename = "type")]
    kind: String,
}

impl DependsDetail {
    fn into_filename_dep(self, commit_id: PseudoCommitId) -> FilenameDep {
        let src = self.src.into_filename_endpoint();
        let tgt = self.tgt.into_filename_endpoint();
        let position = src.position;
        let kind = self.kind.strip_suffix("(possible)").unwrap_or(&self.kind);
        FilenameDep::new(src, tgt, kind.try_into().unwrap(), position, commit_id)
    }
}

#[derive(Debug, Deserialize)]
struct DependsEndpoint {
    #[serde(rename = "file")]
    filename: String,

    #[serde(rename = "lineNumber")]
    line: usize,
}

impl DependsEndpoint {
    fn into_filename_endpoint(self) -> FilenameEndpoint {
        FilenameEndpoint::new(self.filename, PartialPosition::Row(self.line - 1))
    }
}
