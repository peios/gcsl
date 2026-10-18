# Building and Checking GCSL

Build on x86-64 Linux with Python 3.11 or newer, a C compiler, binutils,
curl, Git, and Rust 1.98.1 with rustfmt and Clippy. The `bazel` launcher
verifies and caches Bazel 8.4.2. Bazel builds the Rust crates directly from
locked dependencies. GCSL does not need a Peios development SDK or private
repository credentials.

The source declares a minimum Rust version of 1.96. The standardized
formatting and contributor checks use 1.98.1; this does not claim that a
minimum-version qualification has been run.

## Build and test

Build the executable and run the test graph:

```sh
./bazel build //:artifacts
./bazel test //...
```

The executable includes the language server and the standard modules under
`stdlib/`. There is no separate language-server executable or installed
standard-library source requirement.

Prepare pinned style tools outside the repository, then run the contributor
checks:

```sh
python3 tools/prepare-style-tools.py /tmp/gcsl-style
export PEIOS_STYLE_BIN=/tmp/gcsl-style/bin
tools/check
```

The check covers source formatting, Bazel build definitions, the Bazel test
graph, strict Cargo Clippy diagnostics, and Rust documentation tests. Cargo
is a supplemental development interface; package inputs come from Bazel.

## Build packages

Build the complete package set:

```sh
./bazel build //:packages
```

The content-addressed store at `bazel-bin/packages.store` contains canonical
manifests, compressed SHA-256 blobs, and an index for four packages:

- `dev.peios.gcsl`: the executable and manuals in sections 1, 3, and 5.
- `dev.peios.gcsl-debuginfo`: matching executable debug information.
- `dev.peios.gcsl-debugsource`: source inputs for debugging.
- `dev.peios.gcsl-source`: source, standard modules, and build machinery.

The package version is `0.1.0-1`. Runtime, debuginfo, and debugsource use
`x86_64`; source uses `noarch`. Package assembly validates payload policy,
debug correspondence, licenses, declared dependencies, and blob identities.
The build does not install or sign packages. Outputs are unsigned and
marked `release_ready: false`.

Historical design notes are intentionally excluded from the repository and
package inputs. Their exact archive and source inventory are recorded in
`docs/migration-source-map.json`.

## Build in native Peios userspace

Acquire a dependency snapshot on a networked host:

```sh
tools/offline-build acquire /absolute/path/gcsl-inputs
```

Use the same source and `MODULE.bazel.lock` in an isolated Peios userspace
with Rust 1.98.1, its standard library, a C toolchain, Python, and binutils.
The Rust compiler and standard library need the Peios control-flow
enforcement technology (CET) support required by the native build policy.
Mount the dependency snapshot read-only and provide a private work
directory. Inside that environment, run:

```sh
export PEIOS_RUSTC=/usr/bin/rustc
tools/offline-build test /gcsl-inputs /gcsl-work
```

Use `build` instead of `test` when you only need native artifacts. The
wrapper verifies the module lock and disables dependency downloads. Native
checks require CET, full read-only relocations (RELRO), immediate binding,
and packed relocations.

Native userspace tests run on the caller's kernel. Record host tests,
native userspace tests, and installed-package qualification separately.
Neither successful package assembly nor a host test establishes that a
package has been installed and exercised on a target Peios system.

## Check documentation

Build `gcsl` and select a Trail binary with the site's supported hierarchy.
CI pins Trail revision `1d662e98cbcbdd5c51c01533afcab7f5723a2eff`.
Run the documentation gate with absolute executable paths:

```sh
TRAIL=/absolute/path/to/trail \
    GCSL=/absolute/path/to/gcsl tools/check-docs
```

The gate lints the authored Learn tree, builds it strictly in a temporary
directory, and executes its GCSL examples. It checks the multifile tutorial
as a workspace and verifies the documented outputs. Temporary output is
removed afterward. Panorama merges the standalone `gcsl.product` directly
from `docs/learn/`.

The GitHub Actions workflow runs the contributor and documentation checks
with pinned tool versions and read-only repository permissions. It does
not publish package artifacts as an approved release.
