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
neodepends matrix --depends HEAD > matrix.json
```

`HEAD` can be replaced with any commit (e.g. branch, tag, short or long hash) or with `WORKDIR` if you want to scan directly from disk and not from the git repository. (Neodepends still works even if the project is not a git repository.)

To get cochange information, simply pass more than one commit. Neodepends is designed to work well with [git rev-list](https://git-scm.com/docs/git-rev-list). (On Windows, you may need to use Powershell.)

```bash
neodepends matrix --depends $(git rev-list deltaspike-1.9.6 -n 300) > matrix.json
```

This will extract entities and dependencies from `deltaspike-1.9.6` and use the most recent 300 commits that are reachable from `deltaspike-1.9.6` to calculate cochange.

If you prefer the older format, pass `--format=dsm-v1`. If you prefer the newer format but still want only file-level info, pass the `--file-level` flag. To scan a different directory than the working directory, use `--input`. See `neodepends matrix --help` for more.

## Help

```plaintext
Scan a project and extract structural and historical information.

If the project is a git repository, rather than pulling files from disk, Neodepends can scan the project as it existed in previous commit(s).

Dependency resolution can be done with Stack Graphs ('--stackgraphs'), Depends ('--depends'), or both. If both are enabled, Neodepends will determine which one to use for a particular language by using whichever one is
specified first on the command-line. This is only relevant when a language is supported by both Stack Graphs and Depends.

Usage: neodepends [OPTIONS] <COMMAND>

Commands:
  matrix    Export project data as a design structure matrix
  dump      Export project data as a collection of tables
  entities  Export entities as a table
  deps      Export deps as a table
  changes   Export changes as a table
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

I/O Options:
  -i, --input <INPUT>
          The root of the project/repository to scan.
          
          If not specified, will use the current working directory. If no git repository is found, then Neodepends is placed in "disk-only" mode and will read directly from the file system.

  -o, --output <OUTPUT>
          The path of the output file.
          
          If not provided, will write to stdout.

  -f, --force
          Overwrite the output file if it already exists

Logging Options:
  -v, --verbose...
          Increase logging verbosity

  -q, --quiet...
          Decrease logging verbosity

Depends Options:
      --depends-jar <DEPENDS_JAR>
          Path to the depends.jar that is used for Depends dependency resolution.
          
          If not provided, will look for depends.jar in the same directory as this executable.

      --depends-java <DEPENDS_JAVA>
          Java executable used for running depends.jar.
          
          If not provided, will assume "java" is on the system path.

      --depends-xmx <DEPENDS_XMX>
          Maximum size of the Java memory allocation pool when running Depends.
          
          Passed with "-Xmx" to the Java executable. Useful for large projects that cause Depends to run out of memory. For example, "12G" for a twelve gigabyte memory allocation pool.
```
