# Neodepends

Quickly resolve syntactical dependencies between source files.

## Build

```bash
cargo build --release
```

## Usage

```bash
cargo run -- --help
```

or

```bash
./install_symlink.sh  # only needed to be run once
neodepends --help
```

## Help

```
Scan a project and extract structural dependency information

If the project is a git repository, rather than pulling files from disk, Neodepends can optionally scan the project as it existed in a previous commit with the `--commit` option.

Neodepends caches files on disk as they are scanned. Only files that are new or that have been modified since the last scan need to be processed. This provides signifigant performance benifits when scanning the project many times (for instance, at different commits or after a small change).

Usage: neodepends [OPTIONS]

Options:
  -p, --project-root <PROJECT_ROOT>
          The root of the project to scan
          
          Defaults to the current working directory.

  -c, --cache-dir <CACHE_DIR>
          The directory to act as the cache. Will be created if not found.
          
          Defaults to `{project_root}/.neodepends`.

      --clean
          Delete the cache before scanning

  -l, --langs <LANGS>
          Enable the provided langauges
          
          [default: c cpp go java javascript kotlin python ruby typescript]
          [possible values: c, cpp, go, java, javascript, kotlin, python, ruby, typescript]

      --resolver <RESOLVER>
          Method to use to resolve dependencies between files or entities when needed
          
          [default: stackgraphs]
          [possible values: depends, stackgraphs]

      --commit <COMMIT>
          Extract entities and dependencies from this commit instead of the files on disk
          
          If not specified, will scan recursively from the project root. Can be a reference (e.g. "main", "origin/main", etc.) or a SHA-1 hash.

      --history [<HISTORY>]
          Enable history extraction
          
          May optionally provide a newline delimited list of commits (specified as SHA-1 hashes) where change information will be extracted from. If not provided, will scan all commits reachable from `--commit`. If `--commit` has not been specified, will scan all commits reachable from HEAD.
          
          This option is intended to work with git rev-list. For instance `--history="$(git rev-list --since=1year HEAD)"` will include all commits from the past year that are reachable from HEAD.
          
          Can also load a list of commits from a file, e.g., `--history=history.txt`

      --format <FORMAT>
          Method to use to resolve dependencies between files or entities
          
          [default: jsonv2]
          [possible values: jsonv1, jsonv2]

      --name <NAME>
          Name field in JSONv1 output
          
          Defaults to the last component of the project root.

  -n, --num-threads <NUM_THREADS>
          Number of threads to use when processing files
          
          If 0, this will be set automatically (typically as the number of CPU cores)
          
          [default: 0]

      --depends-jar <DEPENDS_JAR>
          Path to the depends.jar that is used for Depends dependency resolution
          
          If not provided, will look for depends.jar in the same directory as this executable.

      --depends-java <DEPENDS_JAVA>
          Java executable used for running depends.jar
          
          If not provided, will assume "java" is on the system path

      --depends-xmx <DEPENDS_XMX>
          Maximum size of the Java memory allocation pool when running Depends. Passed with "-Xmx" to the Java executable. Useful for large projects that cause Depends to run out of memory. For example, "12G" for a twelve gigabyte memory allocation pool
          
          [default: 4G]

  -v, --verbose...
          Increase logging verbosity

  -q, --quiet...
          Decrease logging verbosity

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

```