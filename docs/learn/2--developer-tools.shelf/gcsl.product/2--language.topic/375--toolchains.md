---
title: Toolchains
type: concept
description: Ask for a compiler or another tool the same way under any host.
---

A library that compiles code needs a compiler, and which one depends on
where it runs and what it builds for. The `toolchain` module lets a
library ask for a tool without knowing who answers: plain GCSL answers
with the tools on your `PATH`, and a build tool such as pekit with the
toolchains your workspace registers.

```text
import toolchain
import Toolchain from toolchain

def compile(src: File, cchain: toolchain.CToolchain = toolchain.get(Toolchain::C)) -> File {
    cchain.cc.exec("-c", src.path, "-o", "out.o")
    = file("out.o")
}
```

`toolchain.get(Toolchain::C)` gives the first registered C toolchain that
runs on `^host_platform` and builds for `^platform` (see
~gcsl/context). Resolving it in a parameter default means the toolchain
is an ordinary argument, so a build tool can give it to a step like any
other input.

## Kinds of toolchain

Every kind is a variant of the extensible enum `Toolchain`, whose layout
holds only files and directories. `Toolchain::C`'s layout is a
`CToolchain`:

| Field | What it is |
| --- | --- |
| `root` | the whole install, which its tools need beside them |
| `runtime` | the root its tools run on: `/` for the host's own |
| `cc`, `ar`, `ld` | the compiler, the archiver, and the linker |
| `sysroot` | the headers and libraries it builds against |

A new kind is one line, such as
`enum Toolchain::Python<{root: Dir, python: File}>`.

## Registering a toolchain

A toolchain is a definition of the kind it provides, with the platforms
it runs on and builds for. Each platform is partial: a field it leaves
out matches anything.

```text
def Toolchain::C gcc_x86_64() {
    let files = archive.unpack(fetch("https://example.org/gcc.tar.xz", sha256 = "..."))
    host_platform = {arch = "x86_64"}
    platform = {arch = "x86_64"}
    provides = {
        root = files
        runtime = files.dir("sysroot")
        cc = files.file("bin/gcc")
        ar = files.file("bin/ar")
        ld = files.file("bin/ld")
        sysroot = files.dir("sysroot")
    }
}
```

The context `toolchains` lists the registered toolchains in order, and
the first that fits wins. In plain GCSL it holds only `system_c`, the
host's own C compiler found with `which`. It runs on the host and builds
for the host alone, so a build for another platform needs a toolchain
registered for that platform. Set it with `with` to choose others:

```text
let app = build_app() with { toolchains = [Toolchain::C gcc_x86_64] }
```

A registered toolchain's body runs once, when a `get` of its kind first
reaches it. A `get` of another kind passes over it without running it,
so a toolchain built with another can be registered in any order: a
tool compiled with the Rust toolchain doesn't make every `get` of the
Rust toolchain wait on itself. When nothing fits, `get` fails, naming
the kind and both platforms.
