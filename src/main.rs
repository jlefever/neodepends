#[macro_use]
extern crate derive_builder;

use core::PseudoCommitId;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use clap::arg;
use clap::ArgMatches;
use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;
use clap_verbosity_flag::InfoLevel;
use clap_verbosity_flag::Verbosity;
use depends::DependsConfig;
use itertools::Itertools;
use languages::Lang;
use output::OutputFormat;
use output::Resource;
use rayon::prelude::*;
use resolution::ResolverManager;
use spec::Pathspec;

use crate::depends::DependsResolverFactory;
use crate::extraction::Extractor;
use crate::filesystem::FileSystem;
use crate::resolution::ResolverFactory;
use crate::spec::Filespec;
use crate::stackgraphs::StackGraphsResolverFactory;

mod core;
mod depends;
mod extraction;
mod filesystem;
mod languages;
mod matrix;
mod output;
mod resolution;
mod sparse_vec;
mod spec;
mod stackgraphs;
mod tagging;

/// Allow an enum to be used on the command-line as long as the enum implements
/// [`strum::EnumString`] and [`strum::VariantNames`].
///
/// This cuts down on some duplication and lets us use [`strum`] as our
/// canonical enum serializer and deserializer.
///
/// From https://github.com/clap-rs/clap/discussions/4264#discussioncomment-3737696
#[macro_export]
macro_rules! strum_parser {
    ($e: ty) => {{
        use clap::builder::TypedValueParser;
        use strum::VariantNames;
        clap::builder::PossibleValuesParser::new(<$e>::VARIANTS).map(|s| s.parse::<$e>().unwrap())
    }};
}

/// Scan a project and extract structural and historical information.
///
/// Neodepends can export the following "resources":
///
/// - Entities: Source code entities like classes, methods, etc.
///
/// - Deps: Syntactic dependencies between entities (like method calls)
///
/// - Changes: Records of a particular commit changing a particular entity
///
/// - Contents: Textual content of source files
///
/// Entities, deps, and contents and considered "structural" resources, while
/// changes are considered "historical" resources.
///
/// For examples,
///
/// $ neodepends --output=out.jsonl --format=jsonl --depends WORKDIR
///
/// will create out.jsonl with one resource per line where each resource comes
/// from the working directory (WORKDIR). If the project is a git repository,
/// Neodepends can also extract resources from one or more commits. For example,
///
/// $ neodepends --output=out.jsonl --format=jsonl --depends $(git rev-list HEAD -n 100)
///
/// will scan the most recent 100 commits reachable from HEAD. By default, entities,
/// deps, and contents will only be extracted from the fist commit specified. The
/// remaining commits are used to calculate changes. If this info is desired for
/// more than the first commit, use the --structure argument.
///
/// Instead of providing the commits directly on the command line, Neodepends
/// can also take commits as a text file. For example,
///
/// $ git rev-list HEAD -n 100 > commits.txt
///
/// $ neodepends --output=out.jsonl --format=jsonl --depends commits.txt
///
/// This is useful in some shells where subcommands are not available.
///
/// Dependency resolution can be done with Stack Graphs (--stackgraphs),
/// Depends (--depends), or both. If both are enabled, Neodepends will
/// determine which one to use for a particular language by using whichever one
/// is specified first on the command-line. This is useful when a language is
/// supported by both Stack Graphs and Depends.
///
/// If --format=csvs or --format=parquets, then a directory will be created with
/// a .csv or .parquet file for each table requested. All other formats will
/// result in a single file.
///
/// A design structure matrix (DSM) has a list of `variables` (entities) and a
/// list of `cells` that indicate relations between pairs of variables. At
/// minimum, these cells indicate syntactic dependencies between pairs of
/// entities. Optionally, these cells may also indicate the number of times a
/// pair of entities have changed together in the same commit (co-change).
#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Opts {
    /// The path of the output file or directory.
    #[arg(short, long)]
    output: PathBuf,

    /// Overwrite the output file or directory if it already exists.
    ///
    /// Careful! This will recursively delete everything at --output.
    #[arg(short, long)]
    force: bool,

    /// The root of the project/repository to scan.
    ///
    /// If not specified, will use the current working directory. If no git
    /// repository is found, then Neodepends is placed in "disk-only" mode
    /// and will read directly from the file system.
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Format of tabular output.
    ///
    /// If not specified, will try to infer from the file extension of the
    /// output. If --format=dsm-v1, then --file-level is implied.
    #[arg(long, value_parser = strum_parser!(OutputFormat))]
    format: Option<OutputFormat>,

    /// Extract and export the provided resources.
    ///
    /// If not provided, all supported resources will be exported.
    #[arg(short, long, value_delimiter = ',', value_parser = strum_parser!(Resource))]
    resources: Vec<Resource>,

    /// Extract entities from historical commits in addition to structural.
    #[arg(long)]
    all_entities: bool,

    /// Always report at the file-level, even when more fine-grain info is
    /// available.
    ///
    /// Implied if --format=dsm-v1.
    #[arg(long)]
    file_level: bool,

    /// Scan these commits for structural data (entities, deps, and contents).
    ///
    /// If not provided, these will only be extracted from the first COMMIT
    #[arg(long, value_name = "COMMIT")]
    structure: Vec<String>,

    /// Commits to be scanned for resources.
    ///
    /// Defaults to WORKDIR if not specified. If input is a bare repository,
    /// then it will default to HEAD. Entities, deps, and contents will only be
    /// extracted from the first commit.
    #[arg(value_name = "COMMIT")]
    revspecs: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,

    #[clap(flatten, next_help_heading = "Dependency Options")]
    resolver_opts: ResolverOpts,

    #[clap(flatten, next_help_heading = "Depends Options")]
    depends_opts: DependsOpts,

    #[clap(flatten, next_help_heading = "Logging Options")]
    logging_opts: LoggingOpts,
}

impl Opts {
    fn contains(&self, table: Resource) -> bool {
        if self.resources.is_empty() {
            true
        } else {
            self.resources.contains(&table)
        }
    }

    fn absolute_input(&self) -> PathBuf {
        if let Some(input) = self.input.clone() {
            if input.is_absolute() {
                input
            } else {
                std::env::current_dir().unwrap().join(input)
            }
        } else {
            std::env::current_dir().unwrap()
        }
    }
}

#[derive(Debug, Args)]
struct LoggingOpts {
    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
}

#[derive(Debug, Args)]
struct DependsOpts {
    /// Path to the depends.jar that is used for Depends dependency resolution.
    ///
    /// If not provided, will look for depends.jar in the same directory as this
    /// executable.
    #[arg(long, global = true)]
    depends_jar: Option<PathBuf>,

    /// Java executable used for running depends.jar.
    ///
    /// If not provided, will assume "java" is on the system path.
    #[arg(long, global = true)]
    depends_java: Option<PathBuf>,

    /// Maximum size of the Java memory allocation pool when running Depends.
    ///
    /// Passed with "-Xmx" to the Java executable. Useful for large projects
    /// that cause Depends to run out of memory. For example, "12G" for a twelve
    /// gigabyte memory allocation pool.
    #[arg(long, global = true)]
    depends_xmx: Option<String>,
}

impl DependsOpts {
    fn to_depends_config(&self) -> DependsConfig {
        DependsConfig::new(
            self.depends_jar.clone(),
            self.depends_java.clone(),
            self.depends_xmx.clone(),
        )
    }
}

#[derive(Debug, Args)]
struct PathspecOpts {
    /// Only scan the provided languages
    ///
    /// Otherwise, all supported languages will be scanned.
    #[arg(short, long, value_delimiter = ',', value_parser = strum_parser!(Lang))]
    langs: Vec<Lang>,

    /// Patterns that each path must match to be scanned
    ///
    /// See https://git-scm.com/docs/gitglossary#def_pathspec.
    #[arg(value_name = "PATH", last = true)]
    patterns: Vec<String>,
}

impl PathspecOpts {
    fn pathspec(&self) -> Result<Pathspec> {
        let lang_pathspec = Lang::pathspec_many(self.langs.clone());
        let user_pathspec = Pathspec::try_from_vec(self.patterns.clone())
            .with_context(|| format!("failed to parse patterns: {:?}", self.patterns))?;
        Ok(lang_pathspec.merge(&user_pathspec))
    }
}

#[derive(Debug, Args)]
struct ResolverOpts {
    /// Enable dependency resolution using Stack Graphs
    ///
    /// When a both tools support a language, Stack Graphs will take priority
    /// over Depends if specified first on the command line.
    #[arg(short = 'S', long)]
    stackgraphs: bool,

    /// Enable dependency resolution using Depends
    ///
    /// When a both tools support a language, Depends will take priority over
    /// Stack Graphs if specified first on the command line.
    #[arg(short = 'D', long)]
    depends: bool,
}

fn main() -> Result<()> {
    let matches = Opts::command().get_matches();
    let opts = Opts::from_arg_matches(&matches)?;
    env_logger::Builder::new().filter_level(opts.logging_opts.verbose.log_level_filter()).init();
    let fs = FileSystem::open(opts.absolute_input())?;
    let pathspec = opts.pathspec_opts.pathspec()?;
    let depends_config = opts.depends_opts.to_depends_config();

    let format = match opts.format {
        Some(format) => format,
        None => infer_format(&opts.output)?,
    };

    let file_level = match format {
        OutputFormat::DsmV1 => true,
        _ => opts.file_level,
    };

    let mut extractor = Extractor::new(fs.clone(), file_level);
    extractor.set_resolver(create_resolver(&matches, depends_config));

    let mut structure_commits = try_parse_revspecs(&fs, &opts.structure)?;
    let history_commits = try_parse_revspecs(&fs, &opts.revspecs)?;

    if structure_commits.is_empty() {
        if history_commits.is_empty() {
            if fs.is_bare_repo() {
                structure_commits.push(fs.head());
            } else {
                structure_commits.push(PseudoCommitId::WorkDir)
            }
        } else {
            structure_commits.push(history_commits[0].clone());
        }
    }

    prepare_output(&opts.output, opts.force)?;
    let mut writer = format.open(&opts.output)?;

    if structure_commits.len() > 1 && writer.is_single_structure() {
        bail!("Selected output format can only take the structural information of a single commit")
    }

    let mut union_commits = structure_commits.clone();
    union_commits.extend(history_commits.clone());
    let union_filespec = Filespec::new(union_commits, pathspec.clone());
    let structure_filespec = Filespec::new(structure_commits, pathspec.clone());
    let history_filespec = Filespec::new(history_commits, pathspec);
    let start = Instant::now();

    let should_extract = |resource: Resource| writer.supports(resource) && opts.contains(resource);

    if should_extract(Resource::Entities) {
        log::info!("Extracting and writing entities...");
        let filespec = match opts.all_entities {
            true => &union_filespec,
            false => &structure_filespec,
        };
        extractor.extract_entities(filespec).for_each(|v| {
            writer.write_entity(v).unwrap();
        });
    }

    if should_extract(Resource::Deps) {
        log::info!("Extracting and writing deps...");
        extractor.extract_deps(&structure_filespec).for_each(|v| {
            writer.write_dep(v).unwrap();
        });
    }

    if should_extract(Resource::Changes) {
        log::info!("Extracting and writing changes...");
        extractor.extract_changes(&history_filespec).for_each(|v| {
            writer.write_change(v).unwrap();
        });
    }

    if should_extract(Resource::Contents) {
        log::info!("Extracting and writing contents...");
        extractor.extract_contents(&structure_filespec).for_each(|v| {
            writer.write_content(v).unwrap();
        });
    }

    writer.finalize()?;
    log::info!("Finished in {}ms", start.elapsed().as_millis());
    Ok(())
}

fn infer_format<P: AsRef<Path>>(output: P) -> Result<OutputFormat> {
    output
        .as_ref()
        .extension()
        .and_then(|e| match e.to_ascii_lowercase().to_str() {
            Some("db") => Some(OutputFormat::Sqlite),
            Some("json") => Some(OutputFormat::DsmV2),
            Some("jsonl") => Some(OutputFormat::Jsonl),
            _ => None,
        })
        .context("Could not infer file format. Use --format to specify.")
}

fn prepare_output<P: AsRef<Path>>(output: P, force: bool) -> Result<()> {
    let path = output.as_ref();
    let path_str = path.to_string_lossy();

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("failed to create parent directory")?;
        }
        return Ok(());
    }

    if !force {
        bail!("Output path ({}) already exists. Use --force to overwrite it.", &path_str);
    }

    if path.is_file() {
        log::info!("Removing existing file at {}", &path_str);
        std::fs::remove_file(path).context("failed to remove file")?;

        // Kind of hacky but we need to remove the WAL of a SQLite database if it exists
        let filename = path.file_name().unwrap().to_str().unwrap();
        let db_shm = path.with_file_name(format!("{}-shm", filename));
        let db_wal = path.with_file_name(format!("{}-wal", filename));

        if db_shm.exists() {
            std::fs::remove_file(db_shm).context("failed to remove shared-memory file")?;
        }

        if db_wal.exists() {
            std::fs::remove_file(db_wal).context("failed to remove write-ahead log")?;
        }

        return Ok(());
    }

    log::info!("Removing existing directory at {}", &path_str);
    std::fs::remove_dir_all(path).context("failed to remove directory")?;
    Ok(())
}

fn try_parse_revspecs(fs: &FileSystem, revspecs: &[String]) -> Result<Vec<PseudoCommitId>> {
    let mut ids = Vec::with_capacity(revspecs.len());

    for revspec in revspecs {
        if let Ok(id) = fs.parse_as_commit(revspec) {
            ids.push(id);
        } else {
            ids.extend(try_read_file_revspecs(fs, revspec)?);
        }
    }

    let ids = ids.into_iter().unique().collect_vec();

    Ok(ids)
}

fn try_read_file_revspecs(fs: &FileSystem, path: &str) -> Result<Vec<PseudoCommitId>> {
    let mut buf = String::new();

    File::open(path)
        .and_then(|mut f| f.read_to_string(&mut buf))
        .with_context(|| format!("'{}' is not a commit in this repository or a file", path))?;

    let mut ids = Vec::new();

    for (i, line) in buf.lines().enumerate() {
        if let Ok(id) = fs.parse_as_commit(&line) {
            ids.push(id);
        } else {
            let path = std::fs::canonicalize(path).unwrap_or(path.into());
            let path = path.to_string_lossy();
            bail!("'{}' is not a commit in this repository ({}:{})", &line, path, i + 1);
        }
    }

    Ok(ids)
}

fn create_resolver(matches: &ArgMatches, config: DependsConfig) -> ResolverManager {
    let mut map: HashMap<&str, Box<dyn ResolverFactory>> = HashMap::new();
    map.insert("stackgraphs", Box::new(StackGraphsResolverFactory::new()));
    map.insert("depends", Box::new(DependsResolverFactory::new(config)));
    ResolverManager::new(sort_by_flag_index(matches, map))
}

fn sort_by_flag_index<V>(matches: &ArgMatches, map: HashMap<&str, V>) -> Vec<V> {
    map.into_iter()
        .filter_map(|(flag, v)| get_flag_index(matches, flag).map(|i| (i, v)))
        .sorted_by_key(|&(i, _)| i)
        .map(|(_, v)| v)
        .collect()
}

fn get_flag_index(matches: &ArgMatches, flag: &str) -> Option<usize> {
    if matches.get_flag(flag) {
        Some(matches.index_of(flag).unwrap())
    } else {
        None
    }
}
