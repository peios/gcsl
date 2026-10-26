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
| `cwd` | `""` | The working directory. `""` keeps GCSL's own; a relative path is relative to it, or in a definition to the definition's directory. |
| `env` | `{}` | Variables set over the environment, as a record of strings. |
| `panic` | `true` | A nonzero exit status fails the call with its standard error. When false, the status is data. |
| `until` | `""` | When set, the program is stopped at the first line of output containing this text. |
| `interactive` | `false` | When true, the program uses GCSL's terminal, and nothing is captured. |

`until` suits a program that keeps running once it's ready, such as a
server or an emulator. The result holds the output read so far, and its
status is 0 because the condition was met. A line counts as soon as the
text is written, before its newline, so a prompt that waits on the line
it asks is met. The program and anything it started are stopped
together. `until` can't be combined with
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

`f.path` is the file's path, always absolute. Outside a definition, a
relative path given to `file` is resolved against the working directory,
the same directory `exec` runs a program in by default. So once a command
has written a file, the same relative path names it:

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
are; a build tool can also compare their contents. Under a build tool, the
tool chooses the working directory. Pekit gives each step a directory of
its own, which both `file()` and `exec` use.

### Relative paths in a definition

Inside a definition, such as `def build lib() { ... }`, a relative path is
resolved against the directory of the definition's own file, whoever calls
the definition. That covers `file`, `dir`, and a relative `cwd`, in the
definition's body and in every function it calls. So `build lib()` reads
the same `lib.c` whether you build it on its own or another definition, in
another directory, builds it. You don't need `fileroot.this` to name a
definition's own files.

A program that `exec` runs with no `cwd` still runs in GCSL's own working
directory, which stays the same for the whole run. That way, a memoised
function that starts something once, such as a test machine, is one call
for every definition that uses it. Pass `cwd = "."` to run a program in
the definition's directory instead.

## Fetching

`fetch(url, sha256)` downloads a file and checks it against a SHA-256
digest, so it's the same file wherever and whenever it's fetched:

```text
let zlib = fetch(
    "https://zlib.net/fossils/zlib-1.3.1.tar.gz",
    sha256 = "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23",
)
```

If the file behind the URL changes, its digest doesn't match the pin, and
`fetch` fails rather than give you different bytes. To find the digest in
the first place, leave it out: `fetch` downloads the file and fails with
the digest it found, ready to paste in.

Plain GCSL runs `curl` to download and `sha256sum` to check, so both must
be installed. A build tool such as pekit keeps fetched files by their
digests, and downloads each one only once.

A fetched file is data, never executable. To run a downloaded program,
copy it and make the copy executable with `fs.set_executable`:

```text
import fs
let tool = file("busybox")
fs.copy(fetched.path, tool.path)
fs.set_executable(tool.path, true)
```

`fs.set_executable` refuses a file with several hard links, since they
share one mode: changing it through one name would change the file
everywhere else too. That's why the example copies first.

### From a git repository

`fetch_git(url, commit)` gives the files of a git repository as one
commit has them, as a directory:

```text
let authd = fetch_git(
    "https://github.com/peios/authd.git",
    "a014fa4c74f17ce4f67ba964eefce33a1a3dbdfa",
)
```

The pin is the commit's whole name, 40 hexadecimal digits. A commit's
name is a digest of its files, so it pins them as `sha256` pins a file. A
branch or a tag can move, so `fetch_git` refuses one. The directory holds
the repository's files without its `.git` directory, and without its
submodules.

Plain GCSL runs `git` to fetch, so it must be installed. A build tool such
as pekit keeps the repositories it fetches, and fetches each commit only
once.

## Unpacking archives

The `archive` module's `unpack(archive, strip)` makes a directory of what
a tar archive holds, compressed with gzip, xz, bzip2, or zstd, or not at
all, or what a zip archive holds, such as a Python wheel. It tells which
from the archive's first bytes, whatever the file is called. `strip`
leaves off the first components of each path, as
`tar --strip-components` does, which removes the single top-level
directory most source archives have:

```text
import archive
let zlib = archive.unpack(fetch("https://zlib.net/fossils/zlib-1.3.1.tar.gz",
    sha256 = "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"), strip = 1)
zlib.file("zlib.h")
```

Plain GCSL runs `tar` to unpack, or `unzip` for a zip archive. A build
tool such as pekit unpacks archives itself and keeps the result, so each
archive is unpacked once.

## Directories

A `Dir` is a tree of files: an unpacked archive, a toolchain, or what a
command leaves in a directory. `dir(path)` makes one, resolved as `file`
resolves a path. Inside it, `d.file(path)` and `d.dir(path)` name a file
and a directory by a path relative to it, and `d.files()` lists every file
in the tree:

```gcsl
import fs
import path
let root = fs.temp_dir()
fs.mkdir(path.join(root, "include"))
fs.write(path.join(root, "include/greet.h"), "void greet(void);")
let tree = dir(root)
header = tree.file("include/greet.h").read()
count = len(tree.files())
```

A path that would leave the directory, such as `d.file("../secret")`, is an
error: a directory's files are its own. `d.exec(...words)` runs a command
with the directory as its working directory.

Use a `Dir` for a tree, and a `FileSet` for loose files such as headers,
sources and objects. A `FileSet` has no root, so it can't say where each of
its files belongs in a tree.

### Composing a directory

`fs.compose_dir` makes a new directory from a record that maps paths to
what goes there:

```gcsl
import fs
import path
let root = fs.temp_dir()
fs.write(path.join(root, "main.c"), "int main(void);")
fs.write(path.join(root, "notes.txt"), "build notes")
let src = dir(root)
let stage = fs.compose_dir({
    "/" = src
    "docs/notes.txt" = src.file("notes.txt")
    "flat" = [src.file("main.c")]
})
listing = stage.exec("ls", "flat").stdout
```

A `Dir`'s tree goes under its key, `/` meaning the new directory itself; a
`File` goes at its key; and each file of a list goes under its key by its
name. Placing two things at one path is an error. Everything is copied, so
a command that writes in the new directory changes none of the originals.
Copying a large tree, such as a toolchain, takes time and space; a build
tool such as pekit mounts the trees instead.

A record's keys are written out, so they can't be computed. When a path
depends on a value, such as a package's name, use `fs.compose`, which
takes a list of entries, each with the path it goes at and what to place
there:

```gcsl
import fs
import path
let root = fs.temp_dir()
fs.write(path.join(root, "zlib.h"), "int deflate(void);")
let name = "zlib"
let stage = fs.compose([
    {at = "include/{name}.h"; place = dir(root).file("zlib.h")},
])
header = stage.file("include/zlib.h").read()
```

Its entries are placed as `fs.compose_dir` places a record's, with the
same errors.

## Sandboxes

An upstream build, such as `./configure && make`, expects its tools at the
usual places: `/bin/sh`, `/usr/bin/make`, a compiler in a fixed directory.
A sandbox runs commands with a directory you compose as their `/`, so those
paths are the ones you put there. `d.sandbox(cwd, env)` makes one from a
directory, with a working directory and an environment for its commands:

```gcsl
import fs
import path
let root = fs.temp_dir()
fs.mkdir(path.join(root, "work/src"))
fs.write(path.join(root, "work/src/config.txt"), "debug = false")
let box = dir(root).sandbox(cwd = "/work/src", env = {MODE = "release"})
config = box.file("config.txt").read()
```

`box.exec(...words)` runs a command in it, as `exec` does: `/` is the
sandbox's directory, the working directory is the sandbox's unless you
pass `cwd`, and the environment is exactly the sandbox's with the call's
`env` set over it, nothing of GCSL's. So a sandbox made with `env = {CC =
"/opt/cc/bin/cc"}` gives every command the compiler that you composed into
`/opt/cc`, never the machine's own. The program is found through that
environment's `PATH`, or `/bin` and `/usr/bin` without one.

A sandbox is live: what one command writes is there for the next, and
`box.file(path)` and `box.dir(path)` read results out, by a path that's
absolute in the sandbox or relative to its working directory. A sandbox is
a copy of its directory, so each sandbox of one directory is its own, and
none changes the directory. Its commands don't reach the network any less
than other commands do; a build tool decides that for the steps it
isolates.

Sandboxes need bubblewrap, `bwrap`. Without it, a sandbox is an error
rather than a command run against your machine's own `/`, which would find
the wrong tools.

Consult `gcsl-stdlib(3)` for exact function contracts and error behavior.
