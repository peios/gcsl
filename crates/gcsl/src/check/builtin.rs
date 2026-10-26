//! The built-in functions, the members of `builtins`: their names, parameters,
//! types and effects (see the checker's [Built-in
//! functions](crate::check::calls#built-in-functions)).

use crate::ast::Effect;
use crate::names::{Name, Names};
use crate::types::{TyId, TypeTable};

use super::BuiltinTypes;

/// A built-in function: a member of the `builtins` module, implemented in
/// Rust, whose value is a [`crate::Value::Builtin`]. See the module
/// documentation, [Built-in functions](crate::check::calls#built-in-functions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `len(list)`: the number of elements of a list, an `Int`. Its typing
    /// is special.
    Len,
    /// `int(x: Float): Int`: `x` truncated toward zero.
    Int,
    /// `float(x: Int): Float`: the `Float` nearest `x`.
    Float,
    /// `exec(argv: List<Revealed<String>>, options: {cwd, env, interactive,
    /// panic, until}) -> {status: Int, stderr: String, stdout: String}`:
    /// runs the program `argv[0]` with the arguments `argv[1..]`, as the
    /// options say, and reports what it did (see [`crate::eval`]). Impure;
    /// the prelude's `exec` gives the options as named parameters.
    Exec,
    /// `panic(message: String): Never`: ends the run with the runtime error
    /// `message`, at the call.
    Panic,
    /// `split(s: String, sep: String): List<String>`: the pieces of `s`
    /// between occurrences of `sep`, in order, as Rust's `str::split` gives
    /// them; an empty `sep` is a runtime error.
    Split,
    /// `join(parts: List<String>, sep: String): String`: the parts with
    /// `sep` between each pair.
    Join,
    /// `starts_with(s: String, prefix: String): Bool`: whether `s` begins
    /// with the bytes of `prefix`.
    StartsWith,
    /// `ends_with(s: String, suffix: String): Bool`: whether `s` ends with
    /// the bytes of `suffix`.
    EndsWith,
    /// `trim(s: String): String`: `s` without its leading and trailing
    /// Unicode whitespace.
    Trim,
    /// `strip_prefix(s: String, prefix: String): String`: `s` without the
    /// bytes of `prefix` if it begins with them, else `s` unchanged.
    StripPrefix,
    /// `strip_suffix(s: String, suffix: String): String`: `s` without the
    /// bytes of `suffix` if it ends with them, else `s` unchanged.
    StripSuffix,
    /// `lines(s: String): List<String>`: the lines of `s`, as Rust's
    /// `str::lines` gives them.
    Lines,
    /// `fields(s: String): List<String>`: the pieces of `s` between runs of
    /// Unicode whitespace, as Rust's `str::split_whitespace` gives them.
    Fields,
    /// `compare_versions(a: String, b: String): Int`: `-1`, `0` or `1` as
    /// `a` sorts before, equal to or after `b` in version order.
    CompareVersions,
    /// `fs_exists(p: String) -> Bool`, `fs.exists`: whether something is at
    /// the absolute path `p`, symbolic links followed. Impure.
    FsExists,
    /// `fs_read(p: String) -> String`, `fs.read`: the contents of the file
    /// at the absolute path `p`, which must be UTF-8 text. Impure.
    FsRead,
    /// `fs_mkdir(p: String) -> {}`, `fs.mkdir`: makes the directory at the
    /// absolute path `p` and any missing parents, as `mkdir -p`. Impure.
    FsMkdir,
    /// `fs_copy(from: String, to: String) -> {}`, `fs.copy`: copies the
    /// file at the absolute path `from` to the absolute path `to`. Impure.
    FsCopy,
    /// `fs_set_executable(p: String, executable: Bool) -> {}`: makes the file
    /// at the absolute path `p` executable, or not, the `fs` module's
    /// `set_executable`. Impure.
    FsSetExecutable,
    /// `fs_temp_dir() -> String`, `fs.temp_dir`: makes a new empty
    /// directory, removed with everything in it when the run that made it
    /// is dropped, and gives its absolute path. Impure.
    FsTempDir,
    /// `fs_write(p: String, contents: String) -> {}`, `fs.write`: writes
    /// `contents` to the file at the absolute path `p`, made or replaced
    /// whole, atomically, in a directory that must exist. Impure.
    FsWrite,
    /// `sort_versions(vs: List<String>): List<String>`: `vs` sorted in
    /// version order, as `compare_versions` orders two, a stable sort.
    SortVersions,
    /// `host_arch(): String`: the architecture of the machine GCSL runs on,
    /// as Rust names it, `x86_64` or `aarch64`, the prelude's default of
    /// `host_platform.arch`.
    HostArch,
    /// `host_os(): String`: the operating system of the machine GCSL runs
    /// on, as Rust names it, `linux`, the prelude's default of
    /// `host_platform.os`.
    HostOs,
    /// `show(value)`: the GCSL source of a value, as text, which reads back
    /// as an equal value. Its typing is special.
    Show,
    /// `exec_live(argv: List<Revealed<String>>, options) -> {status: Int,
    /// stderr: String, stdout: String}`: `exec`, but the program's standard
    /// output and
    /// standard error are also copied, line by line as they arrive, to this
    /// process's standard error (see [`crate::eval`]). Impure.
    ExecLive,
    /// `variant_matches<E, T>(kind: Variant<E, T>, value: E): Bool`:
    /// whether `value`, a value of an enum, is of the variant `kind` (see
    /// [Enums](crate::check::enums#enums)), the prelude's `kind.matches(value)`.
    VariantMatches,
    /// `variant_payload<E, T>(kind: Variant<E, T>, value: E): T`: the
    /// payload of `value`, of the variant `kind`, and a runtime error if it
    /// is another's, the prelude's `kind.payload(value)`.
    VariantPayload,
    /// `partial_matches<T>(partial: Partial<T>, full: T): Bool`: whether
    /// every field `partial` holds equals `full`'s of its name (see [Partial
    /// records](crate::check::enums#partial-records)), the prelude's `partial.matches(full)`.
    PartialMatches,
    /// `secret<T>(value: T, identity: String): Secret<T>`: `value` as a
    /// secret, of the public identity `identity` (see [Secrets](crate::check::secrets#secrets)).
    Secret,
    /// `secret_value<T>(s: Secret<T>): T`: the value of the secret `s`, the
    /// prelude's `s.unsafe_value()`, the deliberate escape (see
    /// [Secrets](crate::check::secrets#secrets)).
    SecretValue,
    /// `secret_reveal<T>(s: Secret<T>): Revealed<T>`: the secret `s`
    /// revealed, which a program may be given, the prelude's `s.reveal()`
    /// (see [Secrets](crate::check::secrets#secrets)).
    SecretReveal,
    /// `file(path: String) -> File`: the file at `path`, made absolute
    /// against the working directory (see [Files](crate::check::files#files)),
    /// the prelude's `file(path)`. Impure: what a relative path means
    /// depends on where it runs.
    File,
    /// `file_read(f: File) -> String`: the contents of the file `f`, which
    /// must be UTF-8, the prelude's `f.read()`. Impure.
    FileRead,
    /// `fetch(url: String, sha256: String) -> File`: the file at `url`,
    /// downloaded, which must have the SHA-256 digest `sha256`, in hex;
    /// with `""`, downloaded only to fail with the digest it has, for its
    /// pin. A host may answer it from a store of files by digest. Impure.
    Fetch,
    /// `fetch_git(url: String, commit: String) -> Dir`: a new directory of
    /// the files of the git repository at `url` as the commit `commit`,
    /// its 40 hex digits, has them, without the repository's own `.git`. A
    /// host may answer it from a cache of repositories. Impure.
    FetchGit,
    /// `archive_unpack(archive: File, strip: Int) -> Dir`: a new directory
    /// of what the tar archive `archive` holds, compressed or not, the first
    /// `strip` components of each path left off, the `archive` module's
    /// `unpack`. A host may answer it itself. Impure.
    ArchiveUnpack,
    /// `which(name: String) -> File`: the program `name` on this process's
    /// `PATH`, the first directory's that has a file of the name its user
    /// may run, the `toolchain` module's way to the host's own tools.
    /// Impure.
    Which,
    /// `keychain_string(name: String, from: {profile: String, file: String})
    /// -> Secret<String>`: the secret string of the name `name` for the
    /// profile, from its configured file or the environment, the `keychain`
    /// module's `string` (see [Keychains](crate::eval::keychain#keychains)).
    /// Impure.
    KeychainString,
    /// `keychain_file(name: String, from: {profile: String, file: String})
    /// -> Secret<File>`: the secret file of the name, as `keychain_string`
    /// finds it, the `keychain` module's `file`. Impure.
    KeychainFile,
    /// `exec_secret(argv: List<String>, options: {cwd, env, panic, secrets,
    /// until}) -> Secret<{status: Int, stderr: String, stdout: String}>`:
    /// runs a program as `exec` does, given the revealed secrets of
    /// `secrets` as environment variables, and reports what it did as a
    /// secret (see [Secrets](crate::check::secrets#secrets)). Impure.
    ExecSecret,
    /// `dir(path: String) -> Dir`: the directory at `path`, made absolute
    /// as `file` makes a file's, the prelude's `dir(path)`. Impure.
    Dir,
    /// `dir_files(d: Dir) -> List<File>`: every file of the tree `d`, by
    /// path, the prelude's `d.files()`. Impure: it reads the tree.
    DirFiles,
    /// `dir_file(d: Dir, path: String): File`: the file at `path`, relative,
    /// in `d`, the prelude's `d.file(path)`; a path outside `d` is a runtime
    /// error.
    DirFile,
    /// `dir_dir(d: Dir, path: String): Dir`: the directory at `path`,
    /// relative, in `d`, the prelude's `d.dir(path)`, as `dir_file`.
    DirDir,
    /// `fs_compose_dir(entries: {*: Dir | File | List<File>}) -> Dir`: a new
    /// directory of `entries`, each placed at its key, `fs.compose_dir`.
    /// Impure.
    FsComposeDir,
    /// `fs_compose(entries: List<{at: String, place: List<File> | Dir |
    /// File}>) -> Dir`: `fs_compose_dir`, but its entries a list, each placed
    /// at a path computed as any string is, the `fs` module's `compose`.
    /// Impure.
    FsCompose,
    /// `dir_sandbox(d: Dir, options: {cwd: String, env: {*: String}}) ->
    /// Sandbox`: a sandbox of a copy of `d`, its commands run in `cwd` with
    /// the environment `env` by default, the prelude's `d.sandbox(...)`.
    /// Impure: it copies `d`.
    DirSandbox,
    /// `sandbox_exec(s: Sandbox, request: {argv, cwd, env, interactive,
    /// panic, until}) -> {status: Int, stderr: String, stdout: String}`:
    /// runs a program in `s`, as `exec` does but with `s` as its `/`, the
    /// prelude's `s.exec(...)`. Impure.
    SandboxExec,
    /// `sandbox_file(s: Sandbox, path: String): File`: the file at `path`
    /// in `s`, absolute in it or relative to its working directory, the
    /// prelude's `s.file(path)`.
    SandboxFile,
    /// `sandbox_dir(s: Sandbox, path: String): Dir`: the directory at
    /// `path` in `s`, as `sandbox_file`, the prelude's `s.dir(path)`.
    SandboxDir,
    /// `contains(s: String, part: String): Bool`: whether `part` is in `s`,
    /// byte-wise, the prelude's `s.contains(part)`.
    Contains,
    /// `list_unique<T>(xs: List<T>): List<T>`: `xs` with each element but
    /// the first of those equal to it, by total equality, left out, in
    /// order, the prelude's `xs.unique()`; an element holding a function or
    /// a future is a runtime error.
    ListUnique,
    /// `list_contains<T>(xs: List<T>, x: T): Bool`: whether an element of
    /// `xs` equals `x`, by total equality, the prelude's `xs.contains(x)`;
    /// one holding a function or a future is a runtime error.
    ListContains,
    /// `json_decode(text: String)`: the JSON document `text` decoded as a
    /// value of the type expected of the call, the `json` module's
    /// `decode`. Its typing is special: the type comes from where the call
    /// stands, as an empty list's element type does (see [JSON
    /// decoding](crate::check::json#json-decoding)).
    JsonDecode,
}

/// How a built-in function is typed (see [Built-in
/// functions](crate::check::calls#built-in-functions)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    /// By a function type the type table can hold, of any parameters, named
    /// as [`Builtin::params`] says, any result and any effect, which
    /// [`Builtin::ty`] interns, the table in hand: its value is a value of
    /// that type, and a call of it is a call like any other.
    Ordinary,
    /// TEMPORARY (alias rule): call by call, by the checker's rule for it, since no function type
    /// the table can hold is its type (`len`: a `List<T>` for any `T` to `Int`; `show`: any `T`
    /// without a function in it to `String`); its value has an opaque type of its own,
    /// [`TyKind::Builtin`](super::TyKind::Builtin). To be replaced by a real polymorphic type (see
    /// the [inference limits](https://learn.peios.org/gcsl/language/inference-limits/)).
    Special,
}

impl Builtin {
    /// Every built-in function, in the order of the `builtins` module, which
    /// is also the order of their cells, the first slots of a workspace's
    /// frame (see [`crate::workspace`]).
    pub const ALL: [Builtin; 56] = [
        Builtin::Len,
        Builtin::Int,
        Builtin::Float,
        Builtin::Exec,
        Builtin::Panic,
        Builtin::Split,
        Builtin::Join,
        Builtin::StartsWith,
        Builtin::EndsWith,
        Builtin::Trim,
        Builtin::StripPrefix,
        Builtin::StripSuffix,
        Builtin::Lines,
        Builtin::Fields,
        Builtin::CompareVersions,
        Builtin::FsExists,
        Builtin::FsRead,
        Builtin::FsMkdir,
        Builtin::FsCopy,
        Builtin::FsSetExecutable,
        Builtin::FsTempDir,
        Builtin::FsWrite,
        Builtin::SortVersions,
        Builtin::HostArch,
        Builtin::HostOs,
        Builtin::Show,
        Builtin::ExecLive,
        Builtin::VariantMatches,
        Builtin::VariantPayload,
        Builtin::PartialMatches,
        Builtin::Secret,
        Builtin::SecretValue,
        Builtin::SecretReveal,
        Builtin::File,
        Builtin::FileRead,
        Builtin::Fetch,
        Builtin::FetchGit,
        Builtin::ArchiveUnpack,
        Builtin::Which,
        Builtin::KeychainString,
        Builtin::KeychainFile,
        Builtin::ExecSecret,
        Builtin::Dir,
        Builtin::DirFiles,
        Builtin::DirFile,
        Builtin::DirDir,
        Builtin::FsComposeDir,
        Builtin::FsCompose,
        Builtin::DirSandbox,
        Builtin::SandboxExec,
        Builtin::SandboxFile,
        Builtin::SandboxDir,
        Builtin::Contains,
        Builtin::ListUnique,
        Builtin::ListContains,
        Builtin::JsonDecode,
    ];

    /// The most parameters any built-in function has: what a call of one
    /// collects its arguments into (see [`crate::eval`]).
    pub const MAX_PARAMS: usize = 2;

    /// The names of the fields of what `exec` and `exec_live` return, in the
    /// order of the record each builds: its exit status, its standard output
    /// and its standard error.
    pub const EXEC_FIELDS: [&'static str; 3] = ["status", "stdout", "stderr"];

    /// The names of the fields of the options `exec` and `exec_live` take,
    /// in the order of the record's type (see [Built-in
    /// functions](crate::check::calls#built-in-functions)): the program's
    /// working directory, its environment, whether it has the terminal,
    /// whether a status other than 0 fails the call, and the text whose
    /// line ends it.
    pub const EXEC_OPTIONS: [&'static str; 5] = ["cwd", "env", "interactive", "panic", "until"];

    /// The names of the fields of the options `exec_secret` takes, in the
    /// order of the record's type: `exec`'s but `interactive`, which would
    /// show what the program writes, and the revealed secrets it is given.
    pub const EXEC_SECRET_OPTIONS: [&'static str; 5] = ["cwd", "env", "panic", "secrets", "until"];

    /// The name of its member of the `builtins` module.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Builtin::Len => "len",
            Builtin::Int => "int",
            Builtin::Float => "float",
            Builtin::Exec => "exec",
            Builtin::Panic => "panic",
            Builtin::Split => "split",
            Builtin::Join => "join",
            Builtin::StartsWith => "starts_with",
            Builtin::EndsWith => "ends_with",
            Builtin::Trim => "trim",
            Builtin::StripPrefix => "strip_prefix",
            Builtin::StripSuffix => "strip_suffix",
            Builtin::Lines => "lines",
            Builtin::Fields => "fields",
            Builtin::CompareVersions => "compare_versions",
            Builtin::FsExists => "fs_exists",
            Builtin::FsRead => "fs_read",
            Builtin::FsMkdir => "fs_mkdir",
            Builtin::FsCopy => "fs_copy",
            Builtin::FsSetExecutable => "fs_set_executable",
            Builtin::FsTempDir => "fs_temp_dir",
            Builtin::FsWrite => "fs_write",
            Builtin::SortVersions => "sort_versions",
            Builtin::HostArch => "host_arch",
            Builtin::HostOs => "host_os",
            Builtin::Show => "show",
            Builtin::JsonDecode => "json_decode",
            Builtin::ExecLive => "exec_live",
            Builtin::VariantMatches => "variant_matches",
            Builtin::VariantPayload => "variant_payload",
            Builtin::PartialMatches => "partial_matches",
            Builtin::Secret => "secret",
            Builtin::SecretValue => "secret_value",
            Builtin::SecretReveal => "secret_reveal",
            Builtin::File => "file",
            Builtin::FileRead => "file_read",
            Builtin::Fetch => "fetch",
            Builtin::FetchGit => "fetch_git",
            Builtin::ArchiveUnpack => "archive_unpack",
            Builtin::Which => "which",
            Builtin::KeychainString => "keychain_string",
            Builtin::KeychainFile => "keychain_file",
            Builtin::ExecSecret => "exec_secret",
            Builtin::Dir => "dir",
            Builtin::DirFiles => "dir_files",
            Builtin::DirFile => "dir_file",
            Builtin::DirDir => "dir_dir",
            Builtin::FsComposeDir => "fs_compose_dir",
            Builtin::FsCompose => "fs_compose",
            Builtin::DirSandbox => "dir_sandbox",
            Builtin::SandboxExec => "sandbox_exec",
            Builtin::SandboxFile => "sandbox_file",
            Builtin::SandboxDir => "sandbox_dir",
            Builtin::Contains => "contains",
            Builtin::ListUnique => "list_unique",
            Builtin::ListContains => "list_contains",
        }
    }

    /// The name of the member of the standard `fs` module that is it,
    /// `fs.read`, for one of the file system's, by which its runtime errors
    /// name it whatever name it is called by; `None` for every other
    /// built-in function.
    #[must_use]
    pub const fn fs_name(self) -> Option<&'static str> {
        match self {
            Builtin::FsExists => Some("fs.exists"),
            Builtin::FsRead => Some("fs.read"),
            Builtin::FsMkdir => Some("fs.mkdir"),
            Builtin::FsCopy => Some("fs.copy"),
            Builtin::FsSetExecutable => Some("fs.set_executable"),
            Builtin::FsTempDir => Some("fs.temp_dir"),
            Builtin::FsWrite => Some("fs.write"),
            Builtin::FsComposeDir => Some("fs.compose_dir"),
            Builtin::FsCompose => Some("fs.compose"),
            Builtin::Len
            | Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::SortVersions
            | Builtin::HostArch
            | Builtin::HostOs
            | Builtin::Show
            | Builtin::JsonDecode
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue
            | Builtin::SecretReveal
            | Builtin::File
            | Builtin::FileRead
            | Builtin::Fetch
            | Builtin::FetchGit
            | Builtin::ArchiveUnpack
            | Builtin::Which
            | Builtin::KeychainString
            | Builtin::KeychainFile
            | Builtin::ExecSecret
            | Builtin::Dir
            | Builtin::DirFiles
            | Builtin::DirFile
            | Builtin::DirDir
            | Builtin::DirSandbox
            | Builtin::SandboxExec
            | Builtin::SandboxFile
            | Builtin::SandboxDir
            | Builtin::Contains
            | Builtin::ListUnique
            | Builtin::ListContains => None,
        }
    }

    /// Whether it is one of the built-ins of files and directories: `file`,
    /// `file_read`, `dir`, `dir_files`, `dir_file`, `dir_dir` and
    /// `fs_compose_dir` (see [Files](crate::check::files#files)).
    #[must_use]
    pub const fn is_of_files(self) -> bool {
        matches!(
            self,
            Builtin::File
                | Builtin::FileRead
                | Builtin::Fetch
                | Builtin::FetchGit
                | Builtin::ArchiveUnpack
                | Builtin::Which
                | Builtin::KeychainString
                | Builtin::KeychainFile
                | Builtin::Dir
                | Builtin::DirFiles
                | Builtin::DirFile
                | Builtin::DirDir
                | Builtin::FsComposeDir
                | Builtin::FsCompose
                | Builtin::DirSandbox
                | Builtin::SandboxExec
                | Builtin::SandboxFile
                | Builtin::SandboxDir
        )
    }

    /// Whether it is one of the built-ins that compare a string's bytes or a
    /// list's elements: `contains`, `list_unique` and `list_contains`.
    #[must_use]
    pub const fn is_of_elements(self) -> bool {
        matches!(
            self,
            Builtin::Contains | Builtin::ListUnique | Builtin::ListContains
        )
    }

    /// The names of its parameters, in order, which named arguments match.
    /// None has more than [`Builtin::MAX_PARAMS`].
    #[must_use]
    pub const fn params(self) -> &'static [&'static str] {
        match self {
            Builtin::Len => &["list"],
            Builtin::Int | Builtin::Float => &["x"],
            Builtin::Exec | Builtin::ExecLive | Builtin::ExecSecret => &["argv", "options"],
            Builtin::DirFiles => &["d"],
            Builtin::DirFile | Builtin::DirDir => &["d", "path"],
            Builtin::FsComposeDir | Builtin::FsCompose => &["entries"],
            Builtin::DirSandbox => &["d", "options"],
            Builtin::SandboxExec => &["s", "request"],
            Builtin::SandboxFile | Builtin::SandboxDir => &["s", "path"],
            Builtin::Contains => &["s", "part"],
            Builtin::ListUnique => &["xs"],
            Builtin::ListContains => &["xs", "x"],
            Builtin::Panic => &["message"],
            Builtin::Split => &["s", "sep"],
            Builtin::Join => &["parts", "sep"],
            Builtin::StartsWith | Builtin::StripPrefix => &["s", "prefix"],
            Builtin::EndsWith | Builtin::StripSuffix => &["s", "suffix"],
            Builtin::Trim
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::SecretValue
            | Builtin::SecretReveal => &["s"],
            Builtin::File | Builtin::Dir => &["path"],
            Builtin::FileRead => &["f"],
            Builtin::Fetch => &["url", "sha256"],
            Builtin::FetchGit => &["url", "commit"],
            Builtin::ArchiveUnpack => &["archive", "strip"],
            Builtin::Which => &["name"],
            Builtin::KeychainString | Builtin::KeychainFile => &["name", "from"],
            Builtin::CompareVersions => &["a", "b"],
            Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir => &["p"],
            Builtin::FsCopy => &["from", "to"],
            Builtin::FsSetExecutable => &["p", "executable"],
            Builtin::FsTempDir | Builtin::HostArch | Builtin::HostOs => &[],
            Builtin::FsWrite => &["p", "contents"],
            Builtin::SortVersions => &["vs"],
            Builtin::Show => &["value"],
            Builtin::JsonDecode => &["text"],
            Builtin::VariantMatches | Builtin::VariantPayload => &["kind", "value"],
            Builtin::PartialMatches => &["partial", "full"],
            Builtin::Secret => &["value", "identity"],
        }
    }

    /// What it may do: only a pure one may be called from a pure function.
    /// `exec` and `exec_live`, which run a program, are impure, and so is
    /// each of the file
    /// system's, `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
    /// `fs_temp_dir` and `fs_write`, which touch the world; every other
    /// built-in function is pure, `panic` included, which ends the run as a
    /// division by zero does, `split`, whose empty separator does the same,
    /// and `show`, which fails so on a `Float` with no literal.
    #[must_use]
    pub const fn effect(self) -> Effect {
        match self {
            Builtin::Exec
            | Builtin::ExecLive
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsSetExecutable
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::File
            | Builtin::FileRead
            | Builtin::Fetch
            | Builtin::FetchGit
            | Builtin::ArchiveUnpack
            | Builtin::Which
            | Builtin::KeychainString
            | Builtin::KeychainFile
            | Builtin::ExecSecret
            | Builtin::Dir
            | Builtin::DirFiles
            | Builtin::FsComposeDir
            | Builtin::FsCompose
            | Builtin::DirSandbox
            | Builtin::SandboxExec => Effect::Impure,
            Builtin::Len
            | Builtin::Int
            | Builtin::Float
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::SortVersions
            | Builtin::HostArch
            | Builtin::HostOs
            | Builtin::Show
            | Builtin::JsonDecode
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue
            | Builtin::SecretReveal
            | Builtin::DirFile
            | Builtin::DirDir
            | Builtin::SandboxFile
            | Builtin::SandboxDir
            | Builtin::Contains
            | Builtin::ListUnique
            | Builtin::ListContains => Effect::Pure,
        }
    }

    /// How it is typed.
    #[must_use]
    pub const fn typing(self) -> Typing {
        match self {
            Builtin::Len | Builtin::Show | Builtin::JsonDecode => Typing::Special,
            Builtin::Int
            | Builtin::Float
            | Builtin::Exec
            | Builtin::Panic
            | Builtin::Split
            | Builtin::Join
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::Trim
            | Builtin::StripPrefix
            | Builtin::StripSuffix
            | Builtin::Lines
            | Builtin::Fields
            | Builtin::CompareVersions
            | Builtin::FsExists
            | Builtin::FsRead
            | Builtin::FsMkdir
            | Builtin::FsCopy
            | Builtin::FsSetExecutable
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::SortVersions
            | Builtin::HostArch
            | Builtin::HostOs
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue
            | Builtin::SecretReveal
            | Builtin::File
            | Builtin::FileRead
            | Builtin::Fetch
            | Builtin::FetchGit
            | Builtin::ArchiveUnpack
            | Builtin::Which
            | Builtin::KeychainString
            | Builtin::KeychainFile
            | Builtin::ExecSecret
            | Builtin::Dir
            | Builtin::DirFiles
            | Builtin::DirFile
            | Builtin::DirDir
            | Builtin::FsComposeDir
            | Builtin::FsCompose
            | Builtin::DirSandbox
            | Builtin::SandboxExec
            | Builtin::SandboxFile
            | Builtin::SandboxDir
            | Builtin::Contains
            | Builtin::ListUnique
            | Builtin::ListContains => Typing::Ordinary,
        }
    }

    /// How its value is shown: as its type, `fn(x: Float): Int`, for an
    /// ordinary one, as any function is, and as the type table shows that
    /// type (a record's fields by name, `panic`'s result `Never`, which no
    /// annotation can write); as `builtin len`, `builtin show`, for a
    /// special one.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Builtin::Len => "builtin len",
            Builtin::Show => "builtin show",
            Builtin::JsonDecode => "builtin json_decode",
            Builtin::Int => "fn(x: Float): Int",
            Builtin::Float => "fn(x: Int): Float",
            Builtin::Exec | Builtin::ExecLive => {
                "fn(argv: List<String>, options: {cwd: String, env: {*: String}, interactive: Bool, \
                 panic: Bool, until: String}) -> {status: Int, stderr: String, stdout: String}"
            }
            Builtin::ExecSecret => {
                "fn(argv: List<String>, options: {cwd: String, env: {*: String}, panic: Bool, \
                 secrets: {*: Revealed<String>}, until: String}) -> Secret<{status: Int, stderr: \
                 String, stdout: String}>"
            }
            Builtin::Panic => "fn(message: String): Never",
            Builtin::Split => "fn(s: String, sep: String): List<String>",
            Builtin::Join => "fn(parts: List<String>, sep: String): String",
            Builtin::StartsWith => "fn(s: String, prefix: String): Bool",
            Builtin::EndsWith => "fn(s: String, suffix: String): Bool",
            Builtin::Trim => "fn(s: String): String",
            Builtin::StripPrefix => "fn(s: String, prefix: String): String",
            Builtin::StripSuffix => "fn(s: String, suffix: String): String",
            Builtin::Lines | Builtin::Fields => "fn(s: String): List<String>",
            Builtin::CompareVersions => "fn(a: String, b: String): Int",
            Builtin::FsExists => "fn(p: String) -> Bool",
            Builtin::FsRead => "fn(p: String) -> String",
            Builtin::FsMkdir => "fn(p: String) -> {}",
            Builtin::FsCopy => "fn(from: String, to: String) -> {}",
            Builtin::FsSetExecutable => "fn(p: String, executable: Bool) -> {}",
            Builtin::FsTempDir => "fn() -> String",
            Builtin::FsWrite => "fn(p: String, contents: String) -> {}",
            Builtin::SortVersions => "fn(vs: List<String>): List<String>",
            Builtin::HostArch | Builtin::HostOs => "fn(): String",
            Builtin::VariantMatches => "fn<E, T>(kind: Variant<E, T>, value: E): Bool",
            Builtin::VariantPayload => "fn<E, T>(kind: Variant<E, T>, value: E): T",
            Builtin::PartialMatches => "fn<T>(partial: Partial<T>, full: T): Bool",
            Builtin::Secret => "fn<T>(value: T, identity: String): Secret<T>",
            Builtin::SecretValue => "fn<T>(s: Secret<T>): T",
            Builtin::SecretReveal => "fn<T>(s: Secret<T>): Revealed<T>",
            Builtin::File => "fn(path: String) -> File",
            Builtin::FileRead => "fn(f: File) -> String",
            Builtin::Fetch => "fn(url: String, sha256: String) -> File",
            Builtin::FetchGit => "fn(url: String, commit: String) -> Dir",
            Builtin::ArchiveUnpack => "fn(archive: File, strip: Int) -> Dir",
            Builtin::Which => "fn(name: String) -> File",
            Builtin::KeychainString => {
                "fn(name: String, from: {file: String, profile: String}) -> Secret<String>"
            }
            Builtin::KeychainFile => {
                "fn(name: String, from: {file: String, profile: String}) -> Secret<File>"
            }
            Builtin::Dir => "fn(path: String) -> Dir",
            Builtin::DirFiles => "fn(d: Dir) -> List<File>",
            Builtin::DirFile => "fn(d: Dir, path: String): File",
            Builtin::DirDir => "fn(d: Dir, path: String): Dir",
            Builtin::FsComposeDir => "fn(entries: {*: List<File> | Dir | File}) -> Dir",
            Builtin::FsCompose => {
                "fn(entries: List<{at: String, place: List<File> | Dir | File}>) -> Dir"
            }
            Builtin::DirSandbox => {
                "fn(d: Dir, options: {cwd: String, env: {*: String}}) -> Sandbox"
            }
            Builtin::SandboxExec => {
                "fn(s: Sandbox, request: {argv: List<String>, cwd: String, env: {*: String}, \
                 interactive: Bool, panic: Bool, until: String}) -> {status: Int, stderr: String, \
                 stdout: String}"
            }
            Builtin::SandboxFile => "fn(s: Sandbox, path: String): File",
            Builtin::SandboxDir => "fn(s: Sandbox, path: String): Dir",
            Builtin::Contains => "fn(s: String, part: String): Bool",
            Builtin::ListUnique => "fn<T>(xs: List<T>): List<T>",
            Builtin::ListContains => "fn<T>(xs: List<T>, x: T): Bool",
        }
    }

    /// The type of its value, interned into `types`, the names of its
    /// parameters, and of the fields of its result, into `names`: for an
    /// ordinary one its function type, its parameters' types and its result
    /// built here, with its [`Builtin::effect`]; the opaque type of a
    /// special one.
    pub(crate) fn ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        // What fills the entries of `param_types` past the built-in's
        // parameters, which the `zip` with `params` below never reads.
        const UNUSED: TyId = TyId::ERROR;
        // Each parameter's type, in the order of `params`, and the result's.
        let (param_types, result): ([TyId; Builtin::MAX_PARAMS], TyId) = match self {
            Builtin::Int => ([TyId::FLOAT, UNUSED], TyId::INT),
            Builtin::Float => ([TyId::INT, UNUSED], TyId::FLOAT),
            Builtin::Exec | Builtin::ExecLive | Builtin::ExecSecret => {
                return self.exec_ty(types, names);
            }
            Builtin::Panic => ([TyId::STRING, UNUSED], types.never()),
            Builtin::Split => ([TyId::STRING, TyId::STRING], types.list(TyId::STRING)),
            Builtin::Join => ([types.list(TyId::STRING), TyId::STRING], TyId::STRING),
            Builtin::StartsWith | Builtin::EndsWith | Builtin::Contains => {
                ([TyId::STRING, TyId::STRING], TyId::BOOL)
            }
            Builtin::Trim | Builtin::FsRead => ([TyId::STRING, UNUSED], TyId::STRING),
            Builtin::File | Builtin::Which => ([TyId::STRING, UNUSED], TyId::FILE),
            Builtin::KeychainString | Builtin::KeychainFile => {
                let [profile, file] = ["profile", "file"].map(|text| names.intern(text));
                let from = types.record([(profile, TyId::STRING), (file, TyId::STRING)]);
                let value = if self == Builtin::KeychainString {
                    TyId::STRING
                } else {
                    TyId::FILE
                };
                ([TyId::STRING, from], types.secret(value))
            }
            Builtin::FileRead => ([TyId::FILE, UNUSED], TyId::STRING),
            Builtin::Fetch => ([TyId::STRING, TyId::STRING], TyId::FILE),
            Builtin::FetchGit => ([TyId::STRING, TyId::STRING], TyId::DIR),
            Builtin::ArchiveUnpack => ([TyId::FILE, TyId::INT], TyId::DIR),
            Builtin::Dir => ([TyId::STRING, UNUSED], TyId::DIR),
            Builtin::DirFiles => ([TyId::DIR, UNUSED], types.list(TyId::FILE)),
            Builtin::DirFile => ([TyId::DIR, TyId::STRING], TyId::FILE),
            Builtin::DirDir => ([TyId::DIR, TyId::STRING], TyId::DIR),
            Builtin::DirSandbox => {
                let [cwd, env] = ["cwd", "env"].map(|text| names.intern(text));
                let environment = types.record_with_rest([], Some(TyId::STRING), Vec::new());
                let options = types.record([(cwd, TyId::STRING), (env, environment)]);
                ([TyId::DIR, options], TyId::SANDBOX)
            }
            Builtin::SandboxExec => return self.sandbox_exec_ty(types, names),
            Builtin::SandboxFile => ([TyId::SANDBOX, TyId::STRING], TyId::FILE),
            Builtin::SandboxDir => ([TyId::SANDBOX, TyId::STRING], TyId::DIR),
            Builtin::FsComposeDir => ([compose_entries(types), UNUSED], TyId::DIR),
            Builtin::FsCompose => ([compose_list(types, names), UNUSED], TyId::DIR),
            Builtin::StripPrefix | Builtin::StripSuffix => {
                ([TyId::STRING, TyId::STRING], TyId::STRING)
            }
            Builtin::Lines | Builtin::Fields => ([TyId::STRING, UNUSED], types.list(TyId::STRING)),
            Builtin::CompareVersions => ([TyId::STRING, TyId::STRING], TyId::INT),
            Builtin::FsExists => ([TyId::STRING, UNUSED], TyId::BOOL),
            Builtin::FsMkdir => ([TyId::STRING, UNUSED], TyId::EMPTY),
            Builtin::FsCopy | Builtin::FsWrite => ([TyId::STRING, TyId::STRING], TyId::EMPTY),
            Builtin::FsSetExecutable => ([TyId::STRING, TyId::BOOL], TyId::EMPTY),
            Builtin::FsTempDir | Builtin::HostArch | Builtin::HostOs => {
                ([UNUSED, UNUSED], TyId::STRING)
            }
            Builtin::SortVersions => {
                let list = types.list(TyId::STRING);
                ([list, UNUSED], list)
            }
            // TEMPORARY (alias rule): the opaque type, until a polymorphic one (see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            Builtin::Len | Builtin::Show | Builtin::JsonDecode => return types.builtin(self),
            Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue
            | Builtin::SecretReveal
            | Builtin::ListUnique
            | Builtin::ListContains => {
                return self.generic_ty(types, names);
            }
        };
        let params = self
            .params()
            .iter()
            .zip(param_types)
            .map(|(&text, ty)| (Some(names.intern(text)), ty));
        types.fn_type(params, result, self.effect())
    }

    /// The type of `exec`, `exec_live` or `exec_secret`, which this is, as
    /// [`Builtin::ty`] gives it: its words, its options, and its result,
    /// a secret one for `exec_secret`.
    fn exec_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        // A program's words and environment are plain: a secret is
        // given to one only by `exec_secret`'s `secrets`.
        let argv = types.list(TyId::STRING);
        let [status, stdout, stderr] = Builtin::EXEC_FIELDS.map(|text| names.intern(text));
        let result = types.record([
            (status, TyId::INT),
            (stdout, TyId::STRING),
            (stderr, TyId::STRING),
        ]);
        let environment = types.record_with_rest([], Some(TyId::STRING), Vec::new());
        if self == Builtin::ExecSecret {
            let [cwd, env, panic, secrets, until] =
                Builtin::EXEC_SECRET_OPTIONS.map(|text| names.intern(text));
            let revealed = types.revealed(TyId::STRING);
            let given = types.record_with_rest([], Some(revealed), Vec::new());
            let options = types.record([
                (cwd, TyId::STRING),
                (env, environment),
                (panic, TyId::BOOL),
                (secrets, given),
                (until, TyId::STRING),
            ]);
            let secret = types.secret(result);
            let params = [
                (Some(names.intern("argv")), argv),
                (Some(names.intern("options")), options),
            ];
            return types.fn_type(params, secret, self.effect());
        }
        let [cwd, env, interactive, panic, until] =
            Builtin::EXEC_OPTIONS.map(|text| names.intern(text));
        let options = types.record([
            (cwd, TyId::STRING),
            (env, environment),
            (interactive, TyId::BOOL),
            (panic, TyId::BOOL),
            (until, TyId::STRING),
        ]);
        let params = [
            (Some(names.intern("argv")), argv),
            (Some(names.intern("options")), options),
        ];
        types.fn_type(params, result, self.effect())
    }
}

impl Builtin {
    /// The type of `sandbox_exec`'s value, interned into `types`, its
    /// parameters' and fields' names into `names`: a sandbox, and a request
    /// of `exec`'s options and the `argv` to run.
    fn sandbox_exec_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        let [status, stdout, stderr] = Builtin::EXEC_FIELDS.map(|text| names.intern(text));
        let result = types.record([
            (status, TyId::INT),
            (stdout, TyId::STRING),
            (stderr, TyId::STRING),
        ]);
        let [cwd, env, interactive, panic, until] =
            Builtin::EXEC_OPTIONS.map(|text| names.intern(text));
        let environment = types.record_with_rest([], Some(TyId::STRING), Vec::new());
        let argv = (names.intern("argv"), types.list(TyId::STRING));
        let request = types.record([
            argv,
            (cwd, TyId::STRING),
            (env, environment),
            (interactive, TyId::BOOL),
            (panic, TyId::BOOL),
            (until, TyId::STRING),
        ]);
        let params = [
            (Some(names.intern("s")), TyId::SANDBOX),
            (Some(names.intern("request")), request),
        ];
        types.fn_type(params, result, self.effect())
    }

    /// The type of a generic built-in's value, interned into `types`, its
    /// type parameters' and parameters' names into `names` (see
    /// [Generics](crate::check::methods#generics)): `variant_matches` and `variant_payload` of
    /// `E` and `T`, `partial_matches`, `secret`, `secret_value` and
    /// `secret_reveal` of `T`;
    /// `Error` for any other.
    pub(super) fn generic_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        if matches!(
            self,
            Builtin::Secret | Builtin::SecretValue | Builtin::SecretReveal
        ) {
            return self.secret_ty(types, names);
        }
        if matches!(self, Builtin::ListUnique | Builtin::ListContains) {
            return self.list_ty(types, names);
        }
        let partial = self == Builtin::PartialMatches;
        let parameters: &[&str] = if partial { &["T"] } else { &["E", "T"] };
        let generic = types.new_generic(parameters.iter().map(|text| names.intern(text)).collect());
        let [first, second] = match self.params() {
            &[first, second] => [first, second].map(|text| names.intern(text)),
            _ => return TyId::ERROR,
        };
        let ty = if partial {
            let full = types.param(generic, 0);
            let record = types.partial(full);
            types.fn_type(
                [(Some(first), record), (Some(second), full)],
                TyId::BOOL,
                Effect::Pure,
            )
        } else {
            let (of, payload) = (types.param(generic, 0), types.param(generic, 1));
            let kind = types.variant(of, payload);
            let result = if self == Builtin::VariantMatches {
                TyId::BOOL
            } else {
                payload
            };
            types.fn_type(
                [(Some(first), kind), (Some(second), of)],
                result,
                Effect::Pure,
            )
        };
        types.generic_fn(ty, generic)
    }
}

impl Builtin {
    /// The type of `list_unique` or `list_contains`, as
    /// [`Builtin::generic_ty`] makes it: of a list of `T`.
    pub(super) fn list_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        let generic = types.new_generic(Box::new([names.intern("T")]));
        let element = types.param(generic, 0);
        let list = types.list(element);
        let xs = Some(names.intern("xs"));
        let ty = if self == Builtin::ListUnique {
            types.fn_type([(xs, list)], list, Effect::Pure)
        } else {
            let x = Some(names.intern("x"));
            types.fn_type([(xs, list), (x, element)], TyId::BOOL, Effect::Pure)
        };
        types.generic_fn(ty, generic)
    }

    /// The type of `secret`, `secret_value` or `secret_reveal`, as
    /// [`Builtin::generic_ty`]
    /// makes it (see [Secrets](crate::check::secrets#secrets)).
    pub(super) fn secret_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        let generic = types.new_generic(Box::new([names.intern("T")]));
        let value = types.param(generic, 0);
        let secret = types.secret(value);
        let params: Vec<Name> = self
            .params()
            .iter()
            .map(|text| names.intern(text))
            .collect();
        let ty = match (self, params.as_slice()) {
            (Builtin::Secret, &[first, second]) => types.fn_type(
                [(Some(first), value), (Some(second), TyId::STRING)],
                secret,
                Effect::Pure,
            ),
            (Builtin::SecretReveal, &[only]) => {
                let revealed = types.revealed(value);
                types.fn_type([(Some(only), secret)], revealed, Effect::Pure)
            }
            (_, &[only]) => types.fn_type([(Some(only), secret)], value, Effect::Pure),
            _ => return TyId::ERROR,
        };
        types.generic_fn(ty, generic)
    }
}

/// The type of `fs_compose`'s parameter, interned into `types`, its fields'
/// names into `names`: a list of records of a path, `at`, and what to
/// place there, a directory, a file or a list of files.
fn compose_list(types: &mut TypeTable, names: &mut Names) -> TyId {
    let files = types.list(TyId::FILE);
    let place = types.union([TyId::DIR, TyId::FILE, files]);
    let entry = types.record([
        (names.intern("at"), TyId::STRING),
        (names.intern("place"), place),
    ]);
    types.list(entry)
}

/// The type of `fs_compose_dir`'s parameter, interned into `types`: a record
/// of any fields, each a directory, a file or a list of files.
fn compose_entries(types: &mut TypeTable) -> TyId {
    let files = types.list(TyId::FILE);
    let entry = types.union([TyId::DIR, TyId::FILE, files]);
    types.record_with_rest([], Some(entry), Vec::new())
}

/// The [`BuiltinTypes`] of a program whose names are `names`.
pub(super) fn builtin_types(names: &Names) -> BuiltinTypes {
    [
        (names.lookup("Int"), TyId::INT),
        (names.lookup("Float"), TyId::FLOAT),
        (names.lookup("Bool"), TyId::BOOL),
        (names.lookup("String"), TyId::STRING),
        (names.lookup("Duration"), TyId::DURATION),
        (names.lookup("Size"), TyId::SIZE),
        (names.lookup("File"), TyId::FILE),
        (names.lookup("Dir"), TyId::DIR),
        (names.lookup("Sandbox"), TyId::SANDBOX),
    ]
}
