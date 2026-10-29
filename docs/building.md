# Building and Checking GCSL

Build on x86-64 Linux with Python 3.11 or newer, a C compiler, Git, and Rust
1.98.1 with rustfmt and Clippy. Cargo builds the crates from the locked
dependencies. GCSL does not need a Peios development SDK or private
repository credentials.

GCSL builds with Cargo rather than pekit. pekit is built from GCSL, so a
GCSL built by pekit would make each depend on the other.

The source declares a minimum Rust version of 1.96. The formatting and
contributor checks use 1.98.1; this does not claim that a minimum-version
qualification has been run.

## Build and test

Build the executable and run the tests:

```sh
cargo +1.98.1 build --release --locked -p gcsl-cli
cargo +1.98.1 test --workspace --locked
```

The executable is `target/release/gcsl`. It includes the language server and
the standard modules under `stdlib/`. There is no separate language-server
executable or installed standard-library source requirement.

Prepare pinned style tools outside the repository, then run the contributor
checks:

```sh
python3 tools/prepare-style-tools.py /tmp/gcsl-style
export PEIOS_STYLE_BIN=/tmp/gcsl-style/bin
tools/check
```

The check covers source formatting, strict Clippy diagnostics, and every
test, documentation tests included. It then lints and runs the GCSL tests
again with the `machine-everywhere` feature, which runs plain programs on
the machine, as graph mode runs them. The tests run unoptimized, as the
stack-depth tests of `STACK_SIZE` assume.

## Packages

The repository does not build packages. ppk builds Peios packages from the
source.

Historical design notes are intentionally excluded from the repository.
Their exact archive and source inventory are recorded in
`docs/migration-source-map.json`.

## Check documentation

Build `gcsl` and select a Trail binary with the site's supported hierarchy.
CI builds Trail revision `64517260afd150283bc1e2309d7c745f2728d467` with
Cargo. Run the documentation gate with absolute executable paths:

```sh
TRAIL=/absolute/path/to/trail \
    GCSL=/absolute/path/to/gcsl tools/check-docs
```

The gate lints the authored Learn tree, builds it strictly in a temporary
directory, and executes its GCSL examples. It checks the multifile tutorial
as a workspace and verifies the documented outputs. It also checks the man
pages with `groff`. Temporary output is removed afterward. Panorama merges
the standalone `gcsl.product` directly from `docs/learn/`.

The GitHub Actions workflow runs the contributor and documentation checks
with pinned tool versions and read-only repository permissions.
