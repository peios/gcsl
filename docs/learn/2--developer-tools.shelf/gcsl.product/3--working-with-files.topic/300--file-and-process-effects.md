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
with the exit status and captured output. By default, a nonzero exit status
is data, not a GCSL runtime error.

`sh` instead executes a string through `/bin/sh` and fails the GCSL run
when the command fails. Don't insert untrusted text into that string.
Prefer `exec` when values need to become arguments rather than shell code.

`exec_live` and `sh_live` also copy child output to GCSL's standard error
as lines arrive. They still capture output in memory. Each gives the child
null standard input. A failure of any of them, such as a program that
can't start, is reported at your call, not inside the standard library.

### The environment

A command inherits GCSL's environment. The `env` option sets variables
over it, replacing any of the same name:

```gcsl
exec(["sh", "-c", "echo $GREETING"], env = {GREETING = "hello"}).stdout
```

The program named first is found through the resulting `PATH`, so setting
`PATH` in `env` also changes where it's looked for. A value you pass can be
a revealed secret: the program receives its value, and GCSL never shows it.

A build tool that runs commands in isolation, such as pekit, gives them an
environment of its own instead of GCSL's.

### Options

`exec` and `exec_live` take these options by name:

| Option | Default | Effect |
| --- | --- | --- |
| `cwd` | `""` | The working directory. `""` keeps GCSL's own; a relative path is relative to it. |
| `env` | `{}` | Variables set over the environment, as a record of strings or revealed secrets. |
| `panic` | `false` | When true, a nonzero exit status fails the call with its standard error. |
| `until` | `""` | When set, the program is stopped at the first line of output containing this text. |
| `interactive` | `false` | When true, the program uses GCSL's terminal, and nothing is captured. |

`until` suits a program that keeps running once it's ready, such as a
server or an emulator. The result holds the output read so far, and its
status is 0 because the condition was met. The program and anything it
started are stopped together. `until` can't be combined with
`interactive`, because an interactive program's output isn't captured.

```gcsl
let started = exec(["sh", "-c", "echo booting; echo ready; sleep 60"], until = "ready")
started.stdout
```

A source-relative `fileroot` value doesn't change a command's working
directory; pass it as `cwd` instead.

Consult `gcsl-stdlib(3)` for exact function contracts and error behavior.
