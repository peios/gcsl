---
title: Checking and Formatting Programs
type: how-to
description: Check types and apply the standard GCSL formatter.
---

You can check a GCSL program without running its effects, then format its
source consistently. The checker and formatter deliberately have different
scopes: checking needs workspace relationships; formatting only needs
source text that parses.

## Before you start

You need `gcsl` and an existing source file or workspace. Commit or save
your changes before rewriting files so you can review the formatting diff.

## Check and format

1. Check a file, including its enclosing workspace if it has one:

   ```sh
   gcsl check main.gcsl
   ```

2. Preview whether formatting would change that file:

   ```sh
   gcsl fmt --check main.gcsl
   ```

3. Apply the formatter and repeat the check:

   ```sh
   gcsl fmt main.gcsl
   gcsl fmt --check main.gcsl
   ```

## Check the result

The checker reports its summary on standard error. Warnings alone do not
make it fail. The final formatting check succeeds without listing the
file when it is already formatted.

A file with lexical or syntax errors is left untouched by the formatter;
fix those diagnostics and retry. Formatting does not require a successful
type check. It uses one language style rather than editor-specific indent
settings.

## Checking a tree

With no paths, both commands use the enclosing workspace and exclude
nested workspaces. An explicit directory makes them discover nested
workspaces too. However, `gcsl check DIRECTORY` checks workspaces, not
unattached `.gcsl` files under that directory. Pass each standalone file
explicitly when it also needs checking.

For automation, `gcsl check --message-format json main.gcsl` writes one
JSON diagnostic per line to standard output and keeps the summary on
standard error. See `gcsl(1)` for the schema and exit statuses.
