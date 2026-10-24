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
    /// `exec(argv: List<String>) -> {status: Int, stderr: String, stdout:
    /// String}`: runs the program `argv[0]` with the arguments `argv[1..]`
    /// and reports what it did (see [`crate::eval`]). Impure.
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
    /// `show(value)`: the GCSL source of a value, as text, which reads back
    /// as an equal value. Its typing is special.
    Show,
    /// `exec_live(argv: List<String>) -> {status: Int, stderr: String,
    /// stdout: String}`: `exec`, but the program's standard output and
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
    pub const ALL: [Builtin; 29] = [
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
        Builtin::FsTempDir,
        Builtin::FsWrite,
        Builtin::SortVersions,
        Builtin::Show,
        Builtin::ExecLive,
        Builtin::VariantMatches,
        Builtin::VariantPayload,
        Builtin::PartialMatches,
        Builtin::Secret,
        Builtin::SecretValue,
    ];

    /// The most parameters any built-in function has: what a call of one
    /// collects its arguments into (see [`crate::eval`]).
    pub const MAX_PARAMS: usize = 2;

    /// The names of the fields of what `exec` and `exec_live` return, in the
    /// order of the record each builds: its exit status, its standard output
    /// and its standard error.
    pub const EXEC_FIELDS: [&'static str; 3] = ["status", "stdout", "stderr"];

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
            Builtin::FsTempDir => "fs_temp_dir",
            Builtin::FsWrite => "fs_write",
            Builtin::SortVersions => "sort_versions",
            Builtin::Show => "show",
            Builtin::ExecLive => "exec_live",
            Builtin::VariantMatches => "variant_matches",
            Builtin::VariantPayload => "variant_payload",
            Builtin::PartialMatches => "partial_matches",
            Builtin::Secret => "secret",
            Builtin::SecretValue => "secret_value",
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
            Builtin::FsTempDir => Some("fs.temp_dir"),
            Builtin::FsWrite => Some("fs.write"),
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
            | Builtin::Show
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => None,
        }
    }

    /// The names of its parameters, in order, which named arguments match.
    /// None has more than [`Builtin::MAX_PARAMS`].
    #[must_use]
    pub const fn params(self) -> &'static [&'static str] {
        match self {
            Builtin::Len => &["list"],
            Builtin::Int | Builtin::Float => &["x"],
            Builtin::Exec | Builtin::ExecLive => &["argv"],
            Builtin::Panic => &["message"],
            Builtin::Split => &["s", "sep"],
            Builtin::Join => &["parts", "sep"],
            Builtin::StartsWith | Builtin::StripPrefix => &["s", "prefix"],
            Builtin::EndsWith | Builtin::StripSuffix => &["s", "suffix"],
            Builtin::Trim | Builtin::Lines | Builtin::Fields | Builtin::SecretValue => &["s"],
            Builtin::CompareVersions => &["a", "b"],
            Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir => &["p"],
            Builtin::FsCopy => &["from", "to"],
            Builtin::FsTempDir => &[],
            Builtin::FsWrite => &["p", "contents"],
            Builtin::SortVersions => &["vs"],
            Builtin::Show => &["value"],
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
            | Builtin::FsTempDir
            | Builtin::FsWrite => Effect::Impure,
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
            | Builtin::Show
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => Effect::Pure,
        }
    }

    /// How it is typed.
    #[must_use]
    pub const fn typing(self) -> Typing {
        match self {
            Builtin::Len | Builtin::Show => Typing::Special,
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
            | Builtin::FsTempDir
            | Builtin::FsWrite
            | Builtin::SortVersions
            | Builtin::ExecLive
            | Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => Typing::Ordinary,
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
            Builtin::Int => "fn(x: Float): Int",
            Builtin::Float => "fn(x: Int): Float",
            Builtin::Exec | Builtin::ExecLive => {
                "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}"
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
            Builtin::FsTempDir => "fn() -> String",
            Builtin::FsWrite => "fn(p: String, contents: String) -> {}",
            Builtin::SortVersions => "fn(vs: List<String>): List<String>",
            Builtin::VariantMatches => "fn<E, T>(kind: Variant<E, T>, value: E): Bool",
            Builtin::VariantPayload => "fn<E, T>(kind: Variant<E, T>, value: E): T",
            Builtin::PartialMatches => "fn<T>(partial: Partial<T>, full: T): Bool",
            Builtin::Secret => "fn<T>(value: T, identity: String): Secret<T>",
            Builtin::SecretValue => "fn<T>(s: Secret<T>): T",
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
            Builtin::Exec | Builtin::ExecLive => {
                let argv = types.list(TyId::STRING);
                let [status, stdout, stderr] = Builtin::EXEC_FIELDS.map(|text| names.intern(text));
                let result = types.record([
                    (status, TyId::INT),
                    (stdout, TyId::STRING),
                    (stderr, TyId::STRING),
                ]);
                ([argv, UNUSED], result)
            }
            Builtin::Panic => ([TyId::STRING, UNUSED], types.never()),
            Builtin::Split => ([TyId::STRING, TyId::STRING], types.list(TyId::STRING)),
            Builtin::Join => ([types.list(TyId::STRING), TyId::STRING], TyId::STRING),
            Builtin::StartsWith | Builtin::EndsWith => ([TyId::STRING, TyId::STRING], TyId::BOOL),
            Builtin::Trim | Builtin::FsRead => ([TyId::STRING, UNUSED], TyId::STRING),
            Builtin::StripPrefix | Builtin::StripSuffix => {
                ([TyId::STRING, TyId::STRING], TyId::STRING)
            }
            Builtin::Lines | Builtin::Fields => ([TyId::STRING, UNUSED], types.list(TyId::STRING)),
            Builtin::CompareVersions => ([TyId::STRING, TyId::STRING], TyId::INT),
            Builtin::FsExists => ([TyId::STRING, UNUSED], TyId::BOOL),
            Builtin::FsMkdir => ([TyId::STRING, UNUSED], TyId::EMPTY),
            Builtin::FsCopy | Builtin::FsWrite => ([TyId::STRING, TyId::STRING], TyId::EMPTY),
            Builtin::FsTempDir => ([UNUSED, UNUSED], TyId::STRING),
            Builtin::SortVersions => {
                let list = types.list(TyId::STRING);
                ([list, UNUSED], list)
            }
            // TEMPORARY (alias rule): the opaque type, until a polymorphic one (see the [inference
            // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
            Builtin::Len | Builtin::Show => return types.builtin(self),
            Builtin::VariantMatches
            | Builtin::VariantPayload
            | Builtin::PartialMatches
            | Builtin::Secret
            | Builtin::SecretValue => {
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
}

impl Builtin {
    /// The type of a generic built-in's value, interned into `types`, its
    /// type parameters' and parameters' names into `names` (see
    /// [Generics](crate::check::methods#generics)): `variant_matches` and `variant_payload` of
    /// `E` and `T`, `partial_matches`, `secret` and `secret_value` of `T`;
    /// `Error` for any other.
    pub(super) fn generic_ty(self, types: &mut TypeTable, names: &mut Names) -> TyId {
        if matches!(self, Builtin::Secret | Builtin::SecretValue) {
            return self.secret_ty(types, names);
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
    /// The type of `secret` or `secret_value`, as [`Builtin::generic_ty`]
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
            (_, &[only]) => types.fn_type([(Some(only), secret)], value, Effect::Pure),
            _ => return TyId::ERROR,
        };
        types.generic_fn(ty, generic)
    }
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
    ]
}
