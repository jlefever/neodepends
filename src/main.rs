#[macro_use]
extern crate derive_builder;

use core::PseudoCommitId;
use std::collections::HashMap;
use std::env::current_dir;
use std::fs::File;
use std::io::Read;
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
use clap::Subcommand;
use clap_verbosity_flag::InfoLevel;
use clap_verbosity_flag::Verbosity;
use depends::DependsConfig;
use itertools::Itertools;
use languages::Lang;
use spec::Pathspec;
use resolution::ResolverManager;

use crate::depends::DependsResolverFactory;
use crate::extraction::Extractor;
use crate::spec::Filespec;
use crate::filesystem::FileSystem;
use crate::resolution::ResolverFactory;
use crate::stackgraphs::StackGraphsResolverFactory;

mod core;
mod depends;
mod extraction;
mod languages;
mod spec;
mod filesystem;
mod resolution;
mod sparse_vec;
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
/// If the project is a git repository, rather than pulling files from disk,
/// Neodepends can scan the project as it existed in previous commit(s).
///
/// Dependency resolution can be done with Stack Graphs ('--stackgraphs'),
/// Depends ('--depends'), or both. If both are enabled, Neodepends will
/// determine which one to use for a particular language by using whichever one
/// is specified first on the command-line. This is only relevant when a
/// language is supported by both Stack Graphs and Depends.
#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Opts {
    #[clap(flatten, next_help_heading = "I/O Options")]
    io_opts: IoOpts,

    #[clap(flatten, next_help_heading = "Logging Options")]
    logging_opts: LoggingOpts,

    #[clap(flatten, next_help_heading = "Depends Options")]
    depends_opts: DependsOpts,

    #[command(subcommand)]
    command: SubCommandOpts,
}

impl Opts {
    fn pathspec_opts(&self) -> &PathspecOpts {
        match &self.command {
            SubCommandOpts::Matrix(c) => &c.pathspec_opts,
            SubCommandOpts::Dump(c) => &c.pathspec_opts,
            SubCommandOpts::Entities(c) => &c.pathspec_opts,
            SubCommandOpts::Deps(c) => &c.pathspec_opts,
            SubCommandOpts::Changes(c) => &c.pathspec_opts,
        }
    }
}

#[derive(Debug, Args)]
struct IoOpts {
    /// The root of the project/repository to scan.
    ///
    /// If not specified, will use the current working directory. If no git
    /// repository is found, then Neodepends is placed in "disk-only" mode
    /// and will read directly from the file system.
    #[arg(short, long, global = true)]
    input: Option<PathBuf>,

    /// The path of the output file.
    ///
    /// If not provided, will write to stdout.
    #[arg(short, long, global = true)]
    output: Option<PathBuf>,

    /// Overwrite the output file if it already exists.
    #[arg(short, long, global = true)]
    force: bool,
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

#[derive(Debug, Subcommand)]
enum SubCommandOpts {
    Matrix(ExportMatrixOpts),
    Dump(DumpOpts),
    Entities(ListEntitiesOpts),
    Deps(ListDepsOpts),
    Changes(ListChangesOpts),
}

/// Export project data as a design structure matrix.
///
/// A design structure matrix (DSM) has a list of `variables` (entities) and a
/// list of `cells` that indicate relations between pairs of variables. At
/// minimum, these cells indicate syntactic dependencies between pairs of
/// entities. Optionally, these cells may also indicate the number of times a
/// pair of entities have changed together in the same commit (co-change).
///
/// Any number of commits may be specified. If at least two are specified then
/// the resulting matrix may also include co-change cells (if any co-changes are
/// found). The fist commit will always be used to collect entities and
/// syntactic dependencies. If there is no first commit, then entities and
/// dependencies will be collected from WORKDIR.
#[derive(Debug, Args)]
struct ExportMatrixOpts {
    /// Format of DSM output
    #[arg(long, default_value_t, value_parser = strum_parser!(MatrixFormat))]
    format: MatrixFormat,

    /// Commits to be scanned
    #[arg(value_name = "COMMIT")]
    revspecs: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,

    #[clap(flatten, next_help_heading = "Dependency Options")]
    resolver_opts: ResolverOpts,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::Display, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "kebab-case")]
enum MatrixFormat {
    DsmV1,
    #[default]
    DsmV2,
}

/// Export project data as a collection of tables
#[derive(Debug, Args)]
struct DumpOpts {
    /// Commits to be scanned for structure (entities and deps)
    #[arg(long, value_name = "COMMIT")]
    structure: Vec<String>,

    /// Commits to be scanned for history (changes)
    #[arg(long, value_name = "COMMIT")]
    history: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,

    #[clap(flatten, next_help_heading = "Dependency Options")]
    resolver_opts: ResolverOpts,
}

/// Export entities as a table
#[derive(Debug, Args)]
struct ListEntitiesOpts {
    /// Commits to be scanned
    #[arg(value_name = "COMMIT")]
    revspecs: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,
}

/// Export deps as a table
#[derive(Debug, Args)]
struct ListDepsOpts {
    #[clap(flatten, next_help_heading = "Dependency Options")]
    resolver_opts: ResolverOpts,

    /// Commits to be scanned
    #[arg(value_name = "COMMIT")]
    revspecs: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,
}

/// Export changes as a table
#[derive(Debug, Args)]
struct ListChangesOpts {
    /// Commits to be scanned
    #[arg(value_name = "COMMIT")]
    revspecs: Vec<String>,

    #[clap(flatten)]
    pathspec_opts: PathspecOpts,
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
    #[arg(short, long, id = "stackgraphs")]
    stackgraphs: bool,

    /// Enable dependency resolution using Depends
    ///
    /// When a both tools support a language, Depends will take priority over
    /// Stack Graphs if specified first on the command line.
    #[arg(short, long, id = "depends")]
    depends: bool,
}

fn main() -> Result<()> {
    let matches = Opts::command().get_matches();
    let opts = Opts::from_arg_matches(&matches)?;
    env_logger::Builder::new().filter_level(opts.logging_opts.verbose.log_level_filter()).init();
    let fs = FileSystem::open(opts.io_opts.input.clone().unwrap_or(current_dir()?));
    let pathspec = opts.pathspec_opts().pathspec()?;
    let depends_config = opts.depends_opts.to_depends_config();
    let start = Instant::now();

    match opts.command {
        SubCommandOpts::Entities(opts) => {
            let commits = try_parse_revspecs(&fs, &opts.revspecs)?;
            let filespec = Filespec::new(commits, pathspec);
            let extractor = Extractor::new(fs.clone(), ResolverManager::empty());

            for entity in extractor.extract_entities(&filespec) {
                println!("{}", serde_json::to_string(&entity)?);
            }
        }
        SubCommandOpts::Changes(opts) => {
            let commits = try_parse_revspecs(&fs, &opts.revspecs)?;
            let filespec = Filespec::new(commits, pathspec);
            let extractor = Extractor::new(fs.clone(), ResolverManager::empty());

            for entity in extractor.extract_changes(&filespec) {
                println!("{}", serde_json::to_string(&entity)?);
            }
        }
        SubCommandOpts::Deps(opts) => {
            let commits = try_parse_revspecs(&fs, &opts.revspecs)?;
            let filespec = Filespec::new(commits, pathspec);
            let resolver = create_resolver(&matches.subcommand().unwrap().1, depends_config);
            let extractor = Extractor::new(fs.clone(), resolver);

            for entity in extractor.extract_deps(&filespec) {
                println!("{}", serde_json::to_string(&entity)?);
            }
        }
        _ => (),
    }

    log::info!("Finished in {}ms", start.elapsed().as_millis());
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

    Ok(ids.into_iter().unique().collect())
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
