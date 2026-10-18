---
title: Organizing a Workspace
type: how-to
description: Split shared declarations from an executable entry file.
---

You can share a type and helper across files by putting them in a
workspace. This example gives the entry file access to root declarations
without an import.

## Before you start

You need `gcsl` on your command search path and an empty writable directory.

## Create the workspace

1. Save these declarations as `workspace.gcsl`:

   ```gcsl
   type Component = {name: String, version: String}
   def describe(item: Component): String {
       "{item.name} {item.version}"
   }
   ```

2. Save the entry program as `main.gcsl` beside it:

   ```gcsl
   describe({name = "gcsl"; version = "0.1.0"})
   ```

3. From that directory, check the workspace and run the entry file:

   ```sh
   gcsl check
   gcsl run main.gcsl
   ```

## Check the result

The check reports `no problems in 2 files checked`. The run prints
`gcsl 0.1.0`. The helper is visible because `def` contributes to the
shared directory scope. Replacing `def` with `fn` makes it private to
`workspace.gcsl`, so the entry file cannot resolve it.

When adding a child library directory, import its public fields or
functions by its root-relative path. Keep independent projects behind
their own workspace markers rather than relying on hidden files to
separate their scopes.
