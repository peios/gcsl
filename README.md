# GCSL

GCSL is a statically typed language for configuration and scripting. The
`gcsl` executable runs programs, checks them without execution, formats
source, and provides an editor language server.

Start with the [Learn documentation](docs/learn/gcsl.product/1--getting-started.topic/100--gcsl.md).
The [build guide](docs/building.md) covers development, checks, and package
artifacts. The manuals describe the [command](docs/man/gcsl.1),
[language](docs/man/gcsl.5), and [standard library](docs/man/gcsl-stdlib.3).

The repository has four Rust crates: the language core, workspace disk
loader, language server, and command-line interface. Standard GCSL modules
under `stdlib/` are embedded into the executable. Pekit and the
Tree-sitter grammar are separate integrations maintained in their own
repositories.

GCSL is licensed under the MIT license; see [LICENSE](LICENSE).
