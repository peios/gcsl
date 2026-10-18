---
title: File and Process Effects
type: concept
description: Understand paths, process execution, and temporary-file lifetime.
---

GCSL provides standard functions for files and subprocesses. Their effects
use the host process's authority. Absolute paths and explicit command
arguments make those effects easier to inspect, but don't restrict what
a program can access.

## Locating files

Import `fileroot` to locate source directories and `path` to compose paths.
`fileroot.this` is the directory of the file containing the expression;
`fileroot.workspace` is the workspace root. `fileroot.caller` identifies
the source directory of the running function's caller.

`path.join`, `path.normalize`, `path.dirname`, and `path.basename` operate
lexically. They don't resolve symbolic links or prove that a path stays
inside a directory. Use them for path construction, not access control.

File operations under `fs` require absolute paths. The following program
writes and reads a temporary file, then removes its temporary directory
when the run ends:

```gcsl
import fs
import path
let directory = fs.temp_dir()
let file = path.join(directory, "message.txt")
fs.write(file, "GCSL")
fs.read(file)
```

Temporary directories last for the owning run, workspace, or REPL session.
Normal cleanup also runs after an evaluation error. Killing the process
can leave temporary directories behind. Atomic replacement by `fs.write`
protects readers from partial contents; it does not preserve a replaced
file's permissions or provide a durable transaction across several files.

## Running a process

`exec` accepts a list whose first element names the program. Later elements
are passed as arguments without shell interpretation. It returns a record
with the exit status and captured output. A nonzero exit status is data,
not a GCSL runtime error.

`sh` instead executes a string through `/bin/sh` and fails the GCSL run
when the command fails. Don't insert untrusted text into that string.
Prefer `exec` when values need to become arguments rather than shell code.

`exec_live` and `sh_live` also copy child output to GCSL's standard error
as lines arrive. They still capture output in memory. All these commands
inherit the host environment and working directory, and give the child
null standard input. A source-relative `fileroot` value does not change
a command's working directory.

Consult `gcsl-stdlib(3)` for exact function contracts and error behavior.
