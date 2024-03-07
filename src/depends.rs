use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use itertools::Itertools;
use serde::Deserialize;
use subprocess::Exec;
use subprocess::Redirection;
use tempfile::TempDir;

use crate::core::DepKind;
use crate::core::FileDep;
use crate::core::FileEndpoint;
use crate::core::PartialPosition;
use crate::loading::FileSystem;

pub struct Depends {
    jar: Option<PathBuf>,
    java: Option<PathBuf>,
    xmx: Option<String>,
}

impl Depends {
    pub fn new(jar: Option<PathBuf>, java: Option<PathBuf>, xmx: Option<String>) -> Self {
        Self { jar, java, xmx }
    }

    pub fn resolve(&self, fs: FileSystem) -> Result<Vec<FileDep>> {
        log::info!("Copying relevent source files to a temp directory for Depends...");
        let work_dir = TempDir::new()?;
        copy_to_dir(fs, &work_dir)?;
        log::info!("Running Depends...");
        self.run(&work_dir)?;
        log::info!("Collecting Depends output and removing temp directory...");
        let deps = load_depends_output(&work_dir)?.iter_file_deps().collect_vec();
        Ok(deps)
    }

    fn run<P: AsRef<Path>>(&self, dir: P) -> Result<()> {
        let mut cmd = Exec::cmd(self.java.clone().unwrap_or("java".into()));

        if let Some(xmx) = &self.xmx {
            cmd = cmd.arg(format!("-Xmx{xmx}"));
        }

        let status = cmd
            .arg("-jar")
            .arg(&get_depends_jar(self.jar.clone())?)
            .arg("java")
            .arg(".")
            .arg("deps")
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
}

fn copy_to_dir<P: AsRef<Path>>(fs: FileSystem, dir: P) -> Result<()> {
    let mut buffer = Vec::new();

    for key in fs.list() {
        fs.load_into_buf(key, &mut buffer)?;
        let path = dir.as_ref().join(&key.filename);
        std::fs::create_dir_all(path.parent().unwrap())?;
        File::create_new(path)?.write_all(&buffer)?;
        buffer.clear();
    }

    Ok(())
}

fn load_depends_output<P: AsRef<Path>>(dir: P) -> Result<DependsOutput> {
    let mut buffer = Vec::new();
    File::open(dir.as_ref().join("deps-structure.json"))?.read_to_end(&mut buffer)?;
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
    fn iter_file_deps(self) -> impl Iterator<Item = FileDep> {
        self.cells.into_iter().flat_map(|c| c.iter_file_deps())
    }
}

#[derive(Debug, Deserialize)]
struct DependsCell {
    #[serde(rename = "details")]
    details: Option<Vec<DependsDetail>>,
}

impl DependsCell {
    fn iter_file_deps(self) -> impl Iterator<Item = FileDep> {
        self.details.into_iter().flat_map(|d| d.into_iter().map(|d| d.into_file_dep()))
    }
}

#[derive(Debug, Deserialize)]
struct DependsDetail {
    #[serde(rename = "src")]
    src: DependsEndpoint,

    #[serde(rename = "dest")]
    tgt: DependsEndpoint,

    #[serde(rename = "type")]
    kind: DepKind,
}

impl DependsDetail {
    fn into_file_dep(self) -> FileDep {
        let src = self.src.into_file_endoint();
        let tgt = self.tgt.into_file_endoint();
        let position = src.position;
        FileDep::new(src, tgt, self.kind, position)
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
    fn into_file_endoint(self) -> FileEndpoint {
        FileEndpoint::new(self.filename, PartialPosition::Row(self.line - 1))
    }
}
