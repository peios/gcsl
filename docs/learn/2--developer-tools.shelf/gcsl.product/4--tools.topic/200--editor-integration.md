---
title: Configuring an Editor
type: how-to
description: Connect an editor to the bundled GCSL language server.
---

The GCSL language server supplies diagnostics and navigation from the same
checker that runs programs. You can use it in an editor that supports the
Language Server Protocol (LSP) over standard input and standard output.

## Before you start

You need the following:

- An editor with configurable LSP support.
- The `gcsl` executable on the editor's command search path.
- A GCSL file or workspace to open.

## Connect the server

1. Associate files ending in `.gcsl` with a GCSL language entry in your
   editor.
2. Set the server command to `gcsl` and its argument list to `["lsp"]`.
   Use standard input and standard output for transport.
3. Open a GCSL file. For a multifile project, open the workspace directory
   so the editor can supply file changes and watch notifications.

## Check the result

Hover over a declared name to see its type. Introduce a misspelled name and
confirm that the editor displays a diagnostic. When an unambiguous
spelling suggestion is available, a quick fix offers the replacement.

The server checks source but does not evaluate programs. It supports
completion, signature help, inlay hints, formatting, symbol search, and
references. Rename checks the proposed edits before returning them and
refuses changes that introduce errors or change name resolution. It does
not rename files, directories, or standard-library declarations.

## Incomplete source

The index retains useful information around many syntax errors. For an
unbalanced block, a query can use a repaired copy to answer navigation or
completion while diagnostics still describe the real text. Rename needs
the real source to be valid enough to verify and can be refused instead.

Editor syntax highlighting is a separate integration. The grammar and
queries in `tree-sitter-gcsl` are maintained in their own repository; they
are not required for the `gcsl lsp` process to check your code.
