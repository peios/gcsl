---
title: Workspaces
type: concept
description: Understand directory scopes, imports, and checking across files.
---

A GCSL workspace is a directory tree rooted at `workspace.gcsl`. Files in
one directory share public declarations. Child directories inherit the
public declarations above them, so common types and helpers can live at
the root without repeated imports.

The marker is an ordinary GCSL source file. It can be empty or declare
shared values and functions. The disk loader finds the nearest enclosing
marker when you run a file, reads the workspace, and checks every source
before executing the selected file and its dependencies.

## Public and private names

Fields, `def` functions, and declared types contribute to a directory's
public scope. `let`, `let mut`, and `fn` bindings remain private to their
file. Dotted public paths merge when they define distinct leaves; a
conflicting leaf is an error unless an earlier declaration explicitly
permits shadowing.

Sibling and child directories are not automatically in scope. Imports
name paths from the workspace root, with `/` between directories, as the
file system does. `import lib/helpers` names `lib/helpers.gcsl` or a
directory of that path. Use `as` for a local alias, or import a named
member with `from`.

A directory or file name in a path is written as it is, dots and all, so
`import pkgs/net.busybox.busybox as busybox` names the directory
`pkgs/net.busybox.busybox`. An import without `as` is named after the
path's last part, so that part must be a plain name; give a name such as
`net.busybox.busybox` one with `as`.

## Libraries

A library is a module that lives apart from any workspace, named by a
path in quotes:

```text
import "github.com/gcslib/gcc"
import compile, link from "github.com/gcslib/gcc"
```

The `gcsl` program looks for the library's directory in each directory
of `$GCSL_LIBRARY_PATH`, then in `.gcsl/libraries/` at the workspace's
root, and uses the first it finds. A library is the `.gcsl` files directly
in its directory, which share their public declarations as a workspace
directory's files do.

A library sees only its own files, the standard library, and the other
libraries it imports: never anything of the workspace that uses it. So a
library means the same in every workspace, and a name it uses can't be
picked up from yours by accident.

A library can keep other files beside its `.gcsl` files, sources to build
or tools to run. A relative path in a library is relative to its own
directory, so `file("tools/gen.py")` names the library's tool wherever
it's used.

GCSL doesn't pin the libraries a workspace imports; a host such as pekit
does. `import libraries` gives the context `pinned_libraries`, a
`Pinned` for each library the host pinned: its path, its git repository,
the directory in the repository that holds it, and the commit. A library
that takes what one of those repositories gives another way, such as a
crate from the same repository, can check that both are at one commit.

## Boundaries and evaluation order

Hidden names and nested directories containing their own `workspace.gcsl`
are excluded from an enclosing workspace's source discovery. A nested
workspace is independent, not an importable child module of the outer
workspace. Symbolic-link directory traversal is not a mechanism for
extending the source tree.

A `.gcslignore` file at the root also excludes paths from source discovery.
Use it for GCSL files that are a tool's data, such as test fixtures. Each
line names one file or directory, relative to the root. Blank lines and lines
that start with `#` have no effect, and there are no wildcards. A line that
names nothing is ignored:

```text
# Highlighting fixtures, which aren't build files.
test/highlight
```

Dependencies determine file order; path order breaks ties. A file runs
as a whole before a file that depends on it. Cyclic file dependencies are
errors, including cycles introduced through function bodies or imports.

A standalone file outside any workspace can import standard modules but
cannot import arbitrary neighboring source files. Create a workspace
marker when several files need to cooperate.

## Deliberate overrides

`#[shadowable]` marks a public declaration or import as replaceable by a
later declaration in the workspace's defined order. The implicit prelude
import is shadowable, so your own declarations can use its names.

Use this attribute for intentional defaults. It is not an escape from
all conflicts: an ordinary public leaf still cannot be defined twice.
Whole-value overrides replace the old value, while distinct dotted
leaves can merge. See `gcsl(5)` for ordering and import syntax.
