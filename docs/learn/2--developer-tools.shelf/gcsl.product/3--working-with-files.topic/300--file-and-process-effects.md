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
with the exit status and captured output. A nonzero exit status fails the
call with the program's standard error; pass `panic = false` to get the
status back as data instead:

```gcsl
let probe = exec(["sh", "-c", "exit 3"], panic = false)
probe.status
```

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
`PATH` in `env` also changes where it's looked for. A secret is never
passed this way: `exec_secret` gives a program secrets, and its result is a
secret too (see Secrets).

A build tool that runs commands in isolation, such as pekit, gives them an
environment of its own instead of GCSL's.

### Options

`exec` and `exec_live` take these options by name:

| Option | Default | Effect |
| --- | --- | --- |
| `cwd` | `""` | The working directory. `""` keeps GCSL's own; a relative path is relative to it. |
| `env` | `{}` | Variables set over the environment, as a record of strings. |
| `panic` | `true` | A nonzero exit status fails the call with its standard error. When false, the status is data. |
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

## Files

A `File` is a value that names a file. `file(path)` makes one, and nothing
is read until you ask:

```gcsl
import fs
import path
let directory = fs.temp_dir()
fs.write(path.join(directory, "notes.txt"), "remember")
let notes = file(path.join(directory, "notes.txt"))
notes.read()
```

`f.path` is the file's path, always absolute. A relative path given to
`file` is resolved against the working directory, the same directory
`exec` runs a program in by default. So once a command has written a file,
the same relative path names it:

```gcsl
let tool = file("/bin/sh")
tool.exec("-c", "echo hello > greeting.txt")
file("greeting.txt").read()
```

`f.exec(...words)` runs the file as a program, with `words` as its
arguments and the same options as `exec`. Like `exec`, it fails on a
nonzero exit status unless you pass `panic = false`.

A list of files is a `FileSet`, an alias of `List<File>`. `files.file(name)`
picks the one file whose path is `name`, or ends with `/name`, and fails if
none or several match. `FilePath` is an alias of `String`, for parameters
that take a path.

A `File` is a type of its own: a record with a `path` field isn't one, and
a `String` path is never taken for one. That lets a build tool such as
pekit find every file a step uses and produces, and move it in and out of
the step's sandbox. Under plain GCSL, two files are equal when their paths
are; a build tool can also compare their contents. When GCSL runs a program
under a build tool, the tool chooses the working directory, so `file()`
and `exec` agree there too.

Consult `gcsl-stdlib(3)` for exact function contracts and error behavior.
