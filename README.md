# Neodepends

Quickly extract entities, dependencies, and changes from a software project.

## Build

```bash
cargo build --release
```

## Usage

```bash
cargo run -- --help
```

Or download a recent [release](https://github.com/jlefever/neodepends/releases) and run

```bash
neodepends --help
```

## Example

Here is an example of how to generate a design structure matrix from a software repository.

```bash
git clone https://github.com/apache/deltaspike
cd deltaspike
neodepends -o matrix.json --depends HEAD
```

`HEAD` can be replaced with any commit (e.g. branch, tag, short or long hash) or with `WORKDIR` if you want to scan directly from disk and not from the git repository. (Neodepends still works even if the project is not a git repository.)

To get cochange information, simply pass more than one commit. Neodepends is designed to work well with [git rev-list](https://git-scm.com/docs/git-rev-list). (On Windows, you may need to use Powershell.)

```bash
neodepends -o matrix.json --depends $(git rev-list deltaspike-1.9.6 -n 300)
```

This will extract entities and dependencies from `deltaspike-1.9.6` and use the most recent 300 commits that are reachable from `deltaspike-1.9.6` to calculate cochange.

If you prefer the older format, pass `--format=dsm-v1`. If you prefer the newer format but still want only file-level info, pass the `--file-level` flag. To scan a different directory than the working directory, use `--input`. See `neodepends --help` for more.

## Help

```plaintext
Scan a project and extract structural and historical information.

Neodepends can export the following "resources":

- Entities: Source code entities like classes, methods, etc.

- Deps: Syntactic dependencies between entities (like method calls)

- Changes: Records of a particular commit changing a particular entity

- Contents: Textual content of source files

Entities, deps, and contents and considered "structural" resources, while
changes are considered "historical" resources.

For examples,

$ neodepends --output=out.jsonl --format=jsonl --depends WORKDIR

will create out.jsonl with one resource per line where each resource comes from
the working directory (WORKDIR). If the project is a git repository, Neodepends
can also extract resources from one or more commits. For example,

$ neodepends --output=out.jsonl --format=jsonl --depends $(git rev-list HEAD -n 100)

will scan the most recent 100 commits reachable from HEAD. By default, entities,
deps, and contents will only be extracted from the fist commit specified. The
remaining commits are used to calculate changes. If this info is desired for
more than the first commit, use the --structure argument.

Instead of providing the commits directly on the command line, Neodepends can
also take commits as a text file. For example,

$ git rev-list HEAD -n 100 > commits.txt

$ neodepends --output=out.jsonl --format=jsonl --depends commits.txt

This is useful in some shells where subcommands are not available.

Dependency resolution can be done with Stack Graphs (--stackgraphs), Depends
(--depends), or both. If both are enabled, Neodepends will determine which one
to use for a particular language by using whichever one is specified first on
the command-line. This is useful when a language is supported by both Stack
Graphs and Depends.

If --format=csvs or --format=parquets, then a directory will be created with a
.csv or .parquet file for each table requested. All other formats will result in
a single file.

A design structure matrix (DSM) has a list of `variables` (entities) and a list
of `cells` that indicate relations between pairs of variables. At minimum, these
cells indicate syntactic dependencies between pairs of entities. Optionally,
these cells may also indicate the number of times a pair of entities have
changed together in the same commit (co-change).

Usage: neodepends [OPTIONS] --output <OUTPUT> [COMMIT]... [-- <PATH>...]

Arguments:
  [COMMIT]...
          Commits to be scanned for resources.
          
          Entities, deps, and contents will only be extracted from the first
          commit.

  [PATH]...
          Patterns that each path must match to be scanned
          
          See https://git-scm.com/docs/gitglossary#def_pathspec.

Options:
  -o, --output <OUTPUT>
          The path of the output file or directory

  -f, --force
          Overwrite the output file or directory if it already exists.
          
          Careful! This will recursively delete everything at --output.

  -i, --input <INPUT>
          The root of the project/repository to scan.
          
          If not specified, will use the current working directory. If no git
          repository is found, then Neodepends is placed in "disk-only" mode and
          will read directly from the file system.

      --format <FORMAT>
          Format of tabular output.
          
          If not specified, will try to infer from the file extension of the
          output.
          
          [possible values: csvs, jsonl, sqlite, dsm-v1, dsm-v2]

  -r, --resources <RESOURCES>
          Extract and export the provided resources.
          
          If not provided, all supported resources will be exported.
          
          [possible values: entities, deps, changes, contents]

      --file-level
          Always report at the file-level, even when more fine-grain info is
          available

      --structure <COMMIT>
          Scan these commits for structural data (entities, deps, and contents).
          
          If not provided, these will only be extracted from the first COMMIT

  -l, --langs <LANGS>
          Only scan the provided languages
          
          Otherwise, all supported languages will be scanned.
          
          [possible values: c, cpp, go, java, javascript, kotlin, python, ruby,
          typescript]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Dependency Options:
  -S, --stackgraphs
          Enable dependency resolution using Stack Graphs
          
          When a both tools support a language, Stack Graphs will take priority
          over Depends if specified first on the command line.

  -D, --depends
          Enable dependency resolution using Depends
          
          When a both tools support a language, Depends will take priority over
          Stack Graphs if specified first on the command line.

Depends Options:
      --depends-jar <DEPENDS_JAR>
          Path to the depends.jar that is used for Depends dependency
          resolution.
          
          If not provided, will look for depends.jar in the same directory as
          this executable.

      --depends-java <DEPENDS_JAVA>
          Java executable used for running depends.jar.
          
          If not provided, will assume "java" is on the system path.

      --depends-xmx <DEPENDS_XMX>
          Maximum size of the Java memory allocation pool when running Depends.
          
          Passed with "-Xmx" to the Java executable. Useful for large projects
          that cause Depends to run out of memory. For example, "12G" for a
          twelve gigabyte memory allocation pool.

Logging Options:
  -v, --verbose...
          Increase logging verbosity

  -q, --quiet...
          Decrease logging verbosity
```
