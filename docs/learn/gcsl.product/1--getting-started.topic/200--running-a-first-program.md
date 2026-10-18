---
title: Running a First Program
type: how-to
description: Create, check, and run a small GCSL file.
---

You can run a single GCSL file without creating a workspace. This example
checks a small configuration and prints its computed description.

## Before you start

You need the following:

- The `gcsl` executable on your command search path.
- A writable directory for the example file.

## Create and run the file

1. Save the following text as `hello.gcsl`:

   ```gcsl
   let component = {name = "gcsl"; version = "0.1.0"}
   "{component.name} {component.version}"
   ```

2. Check the file without running it:

   ```sh
   gcsl check hello.gcsl
   ```

3. Run the file:

   ```sh
   gcsl run hello.gcsl
   ```

## Check the result

The check reports `no problems in 1 file checked` on standard error. The
run prints the string contents on standard output:

```text
gcsl 0.1.0
```

A file whose result is `{}`, the empty record, prints nothing. Diagnostics
go to standard error. See `gcsl(1)` for exit statuses and output modes.

## Trying expressions interactively

Run `gcsl` without arguments to open the REPL. Enter `let n = 21`, then
`n * 2` on the next line. The second line prints `42 : Int`. Each input is
one line; separate several items on that line with semicolons. Press
**Ctrl+D** to leave the session.
