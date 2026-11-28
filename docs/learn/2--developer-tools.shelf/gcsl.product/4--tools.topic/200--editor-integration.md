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

1. Associate files ending in `.gcsl`, `.pekit`, and `.prvm` with a GCSL
   language entry in your editor. pekit's build files and Provium's tests
   are GCSL with a prelude of their own.
2. Set the server command to `gcsl` and its argument list to `["lsp"]`.
   Use standard input and standard output for transport.
3. Open a GCSL file. For a multifile project, open the workspace directory
   so the editor can supply file changes and watch notifications.

## Add a third-party host's files

A host other than pekit and Provium can give its own files a prelude, and
the server checks them once the host is installed (see [Giving Your Files
a Prelude](~gcsl/giving-a-prelude)). Your editor still needs to send them
to the server:

1. Run `gcsl hosts` in the workspace to list the installed hosts and the
   extension of each.
2. Add the extension to your editor's GCSL file association.

The server takes a file's host from its extension, and watches the host
directories, so it rechecks when a host writes a new prelude.

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
