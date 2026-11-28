---
title: Giving Your Files a Prelude
type: how-to
description: Give the files of your host their own prelude, so they use your names with no imports, and install it so that gcsl check and editors check them as you run them.
related:
  - gcsl/embedding
  - gcsl/definition-types
  - gcsl/editor-integration
---

A host can give its own files a prelude: a library whose names every one
of those files sees with no import. A test harness does this so that a
test file declares `test` and calls `step` directly. Your files are told
apart by their extension, such as `.prvm` for Provium's tests.

GCSL itself doesn't know what an extension means. Your host gives the
prelude to the files it chooses when it loads a workspace. The `gcsl`
tools and the language server, which don't run your host, find your
prelude on disk by the extension, so they check your files as your host
runs them.

## Before you start

You need the following:

- A host that embeds GCSL and reads workspaces with `gcsl-disk`
- A library of the names your files should see, declared with `def`
- An extension for your files that isn't `gcsl`

A name the files see through the prelude must be declared with `def`. A
`fn` is private to its own file, so the prelude's import doesn't bring it.

## Give the prelude to your files

1. Give each of your files the library as its prelude with
   `SourceFile::with_prelude`:

   ```rust
   let source = SourceFile::new("tests/boot.prvm", text)?.with_prelude("provium");
   ```

   The file is checked as if it began with `#[shadowable] import * from
   "provium"`. A name the library declares replaces the standard
   prelude's name of the same spelling, and anything the file declares or
   imports itself replaces both.

2. Give the library's files with the workspace's, named
   `<lib>/provium/FILE.gcsl`, as you would any library's.
   `gcsl::workspace::library_imports` returns a file's prelude first, so
   code that loads the libraries a file imports loads the prelude too.

3. Check the workspace with `check_with_standard` as usual.

A definition type your library declares replaces the standard one of the
same path. So `def_types` lists your `test`, and `prepare_call(("test",
name), ...)` invokes it, even though the standard prelude has a `test` of
its own.

If the prelude's library isn't among the files, each file given it has
one error at its start: "the prelude `provium` given for this file is not
found". The file's uses of the missing names aren't reported one by one.

## Read your files with gcsl-disk

`gcsl_disk::read_workspace_for` reads the workspace's `.gcsl` files and
the files of the hosts you pass it, gives each its host's prelude, and
looks for libraries in each host's directory too:

```rust
let host = gcsl_disk::Host {
    extension: "prvm".to_owned(),
    place: gcsl_disk::Place::Workspace,
    dir: Some(root.join(gcsl_disk::WORKSPACE_HOSTS).join("prvm")),
    prelude: Some("provium".to_owned()),
};
let sources = gcsl_disk::read_workspace_for(&root, None, &[host])?;
```

`read_workspace` still reads `.gcsl` files alone, so a host that doesn't
ask for your extension never reads your files.

## Install the prelude for the tools

Write your host's directory into the workspace whenever your host loads
one, so that `gcsl check` and editors use the prelude your host runs:

```text
.gcsl/hosts/
  .gitignore            *
  prvm/
    prelude             provium
    provium/core.gcsl
```

- The directory's name is your files' extension.
- It is a library root: your libraries go in it, each in a directory of
  its path.
- The file `prelude` holds one line, the path of the library that is the
  prelude.
- The `.gitignore` keeps the directory out of version control, so a
  workspace carries nothing for your host.

Write it only when its content differs, so editors don't reload for
nothing. A package of your host can also install the same directory as
`/usr/share/gcsl/hosts/prvm/`, which the tools use when a workspace has
none of its own.

## Check the result

Run `gcsl hosts` in the workspace. Your host is listed with the place it
was found and its prelude:

```text
pekit  built in   no prelude
prvm   workspace  provium     .gcsl/hosts/prvm
```

Then run `gcsl check`. Your files are checked with their prelude, and a
name your library doesn't declare is reported where it is used.

The tools know pekit and Provium by name, so a `.pekit` or `.prvm` file is
read even when its host isn't installed, and gets one error that names the
host to install. A file of any other extension is read only when its host
is installed. Users of a third-party host also associate its extension
with GCSL in their editors; see [Configuring an
Editor](~gcsl/editor-integration).
