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
name paths from the workspace root. `import lib.helpers` names
`lib/helpers.gcsl` or a directory of that path. Use `as` for a local alias,
or import a named member with `from`.

## Boundaries and evaluation order

Hidden names and nested directories containing their own `workspace.gcsl`
are excluded from an enclosing workspace's source discovery. A nested
workspace is independent, not an importable child module of the outer
workspace. Symbolic-link directory traversal is not a mechanism for
extending the source tree.

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
