//! GCSL: a statically typed scripting language.
//!
//! A program is a block body: a sequence of items (`let` declarations,
//! `let mut` declarations of mutable variables, named functions, `:=`
//! rebindings, fields, `type` declarations, a `= EXPR` result and
//! expression statements)
//! whose value is its result, or the record of its fields. It passes
//! through these stages, each exposed as its own module so that tools can
//! stop after any of them:
//!
//! 1. [`lexer::tokenize`]: source text to tokens, interning every name into
//!    a [`names::Names`] table and storing the processed text of every
//!    string literal in a [`strings::Strings`] table.
//! 2. [`parser::parse`]: tokens to an arena-based [`ast::Ast`], which owns
//!    both tables from then on. Both stages go on past their errors
//!    ([`lexer::lex`], [`parser::parse_all`]), reporting each and leaving
//!    an error node where it is, so that a tool sees every error and what
//!    parsed around them; nothing with an error runs.
//! 3. [`check::check`]: types for every expression, interned into a
//!    [`TypeTable`] the caller owns, and a storage slot for every variable,
//!    in the side tables of a [`check::Checked`]; and any warnings. Alone,
//!    with no standard library, it knows no name a file does not declare.
//! 4. [`lower::lower`]: AST and tables to a type-specialised [`ir::Ir`],
//!    which holds its own copies of the string literals and record layouts
//!    it uses, and the code of every function and `lazy`, with an explicit
//!    await wherever a value that may be a future is used as its bits.
//! 5. [`eval::eval`]: IR, shared by reference count with the closures and
//!    futures made from it, to a [`Value`].
//!
//! A language server stops after the check, and indexes what it found, in
//! files with errors too: what each name refers to, where that is
//! declared, and what it is ([`index`]). A formatter stops after the
//! parse: [`fmt::format`] lays a file that parses out in the one style,
//! from its tokens, its comments and its tree.
//!
//! A file may also declare types, `type PackageDef = {...}`, structural
//! aliases in a namespace of their own, used bare, qualified through an
//! import (`t.PackageDef`) or spread into a record type (`{...PackageDef,
//! name: String}`); and a field may be annotated, `x: Int = 1`, as a `let`
//! may. Before a file is checked, its types are *expanded*: each name of a
//! declared type made what its definition is, as if written there
//! ([`ast::Ast::expand_types`], [`workspace`], "Types").
//!
//! A `def` marked `#[def_type]` is a *def-type*, which other `def`s are
//! made with: `def package dev.peios.testpkg() { ... }` defines, in
//! `package`'s namespace, a function whose type `package` gives, binding
//! nothing a variable or a field can name, and `package
//! dev.peios.testpkg()` calls `package` with the definition's path, the
//! definition and its own arguments. A namespace is a root of the field
//! trees under a key no source can write, so definitions merge, conflict
//! and are shadowed as dotted `def`s are, through directories and `*`
//! imports (see [`check`], "Def-types", and [`workspace`], "Def-types").
//! A checked workspace lists its def-types with their definitions, and
//! invokes a definition by the def-type's name and its path, given as text,
//! with the def-type's invocation arguments, given by name, each a string,
//! an integer, a float or a `Bool`, read from text by the parameter's type,
//! as if `DT PATH(NAME = VALUE, ...)` were called from the definition's
//! directory, for a program that turns def-types into commands, as `pekit`
//! does (see
//! [`workspace`], "Def-types by name"); finding and reading a workspace's
//! files for it is the `gcsl-disk` crate's.
//!
//! A function type may have a *meta*, a record attached to the function
//! value, written after its result with a marker of its own, `fn(v:
//! Version): PackageDef META-> PackageMeta`, whose fields are read as the
//! function's, `func.versions`, purely (`META:`), running its body at each
//! read (`META->`), or once per run (`META->>`). A def-type declares the
//! meta of its definitions by its definition parameter's type, and `defmeta
//! package dev.peios.testpkg { ... }` declares the meta of the definition of
//! its path in its directory, which a closure of the definition then
//! carries (see [`check`], "Metas", and [`workspace`], "Def-types"). So does
//! a reference to the definition: `(package dev.peios.testpkg).versions`
//! reads it, and so does `package dev.peios.testpkg.versions`, a reference
//! meaning the longest prefix of its path that names a definition, the rest
//! read from its meta.
//!
//! [`run`] performs all of them on one file. A [`Session`] performs them on
//! a sequence of inputs that share their variables, as the REPL needs. A
//! [`workspace::Workspace`] performs them on the files of a directory tree,
//! each directory's files sharing one scope of fields and `def`s, and each
//! file reaching the rest of the tree by `import`, its dotted paths merged
//! with its directory's, the directories' above it and what it imports
//! with `*` or by member into one view, checking every file and running
//! the one asked for after the files it needs.
//!
//! Every one of them has the standard library: the `builtins` module, the
//! built-in functions implemented in Rust (`len`, `int`, `float`, `exec`,
//! `panic`, on strings `split`, `join`, `starts_with`, `ends_with`, `trim`,
//! `strip_prefix`, `strip_suffix`, `lines` and `fields`,
//! `compare_versions`, which orders two strings in version order, and
//! `sort_versions`, which sorts a list of them so, `show`, which gives the
//! GCSL source of a value, as text that reads back as an equal value, and
//! the file system's `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
//! `fs_temp_dir` and `fs_write`, and `exec_live`, which runs a program as
//! `exec` does, copying its output to standard error as it runs), and the
//! standard files, the `.gcsl` files
//! of the repository's
//! `stdlib/` directory compiled in ([`STANDARD`]), each a module importable
//! by its bare name from any file and each a scope of its own; and every
//! file and input begins as if with `#[shadowable] import * from prelude`,
//! whose names its own declarations shadow silently, unless the file is
//! marked `#![no_prelude]` (see [`workspace`], "The standard library" and
//! "Shadowable declarations"). A declaration or an import marked
//! `#[shadowable]` may be shadowed so by a later one of its name, in one
//! static order of the workspace. The prelude binds the built-in
//! functions but the file system's, and `sh`, written
//! in GCSL on `exec` and `panic`, which runs a shell command and panics
//! unless it succeeds, and `sh_live`, the same on `exec_live`; `path` (`import path`) joins,
//! normalises and takes
//! apart paths, lexically, in GCSL on the string built-ins; and `fs`
//! (`import fs`) is the file system's built-ins as its members, impure and
//! taking only absolute paths: `fs.exists`, `fs.read`, `fs.mkdir`,
//! `fs.copy`, `fs.temp_dir`, which makes a new directory that is removed,
//! with everything in it, as the workspace or [`Session`] whose run made it
//! is dropped, and `fs.write`, which writes a file whole, atomically,
//! through a temporary file beside it (see [`eval`], "The built-in
//! functions"). The
//! `fileroot` module (`import fileroot`), implemented in Rust, tells code
//! where files
//! are, as absolute paths: `fileroot.this`, the directory of the file the
//! code is written in, `fileroot.workspace`, the workspace's root, and
//! `fileroot.caller`, the directory of the file whose code called the
//! running function, which `fileroot.helper()` makes a function pass on to
//! the functions it calls; a memoised function that reads its caller is
//! memoised by it too (see [`eval`], "Callers and helpers"). A workspace's
//! root, or a lone file's directory, is given by whoever runs it (the CLI
//! canonicalises it), and a [`Session`]'s, and that of [`run`] and the
//! entry points told none, is the working directory. A workspace or
//! a lone file loads only the standard files it needs, the prelude and
//! those its files import; a [`Session`] loads them all. A file run
//! alone ([`run`]) is a workspace of one directory beside it, and an
//! `import` of anything else there, or in a [`Session`], is an error. Every
//! failure is a [`Diagnostic`] that can be rendered against the
//! [`SourceFile`], and so is every warning, which does not stop a program
//! from running and is returned with its value in an [`Outcome`]; a call of
//! `panic` is a runtime error of the program's own wording. The library
//! never prints, but the copy `exec_live` makes to the process's standard
//! error, and reads no source file; the things it does outside itself
//! are what a program asks of `exec`, which runs a program, its output
//! captured, its standard input the null device, of `exec_live`, which does
//! the same, copying the program's output to standard error, line by line,
//! as it runs, and of the file system's
//! built-ins, which read, make, copy and write files and directories at the
//! absolute paths the program gives (see [`eval`], "The built-in
//! functions"), and removing the temporary directories `fs.temp_dir` made,
//! besides asking the process for its working directory, canonical, where
//! it is given no root.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro
)]

pub mod ast;
pub mod check;
pub mod diagnostic;
pub mod eval;
pub mod fmt;
pub mod index;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod mentions;
pub mod names;
pub mod parser;
pub mod session;
pub mod source;
pub mod strings;
pub mod suggest;
pub mod types;
pub mod value;
pub mod workspace;

pub use diagnostic::{Diagnostic, Severity, Suggestion};
pub use session::Session;
pub use source::{SourceFile, Span};
pub use types::{TyId, TypeTable};
pub use value::{Value, ValueKind};

/// The deepest nesting accepted, measured as the height of the syntax tree:
/// a leaf is one level, and every operator (`await` included), parenthesis,
/// ascription, field access, block, `if`, loop, interpolated string,
/// function expression, `lazy`, call, list literal and index adds one to
/// its tallest child (a block's children being its items' expressions, a
/// `break`'s, `continue`'s or `return`'s value among them, and annotated types, an
/// ascription's its expression and type, a function's
/// its parameter types, return type and body, a `lazy`'s its operand, a
/// call's its callee and arguments, a list literal's its elements'
/// expressions, spread or not, an index's its list and its position, a
/// `for`'s its iterable, its guard and its body, a condition loop's the
/// items of its header, as items, its condition and its body, and a `do`
/// loop's its body and its condition). A type written
/// as a name is one level, a record type one
/// more than its tallest field's type, a function type one more than the
/// tallest of its parameter and return types, an applied type such as
/// `Future<Int>` one more than its argument, `T[]` one more than `T`, and a
/// union one more than the
/// taller of its sides; a type written as the name of a declared type is,
/// once the file's types are expanded, as tall as its definition, and a
/// type expanded taller than this limit is rejected where it is written,
/// with the same error (see [`ast::Ast::expand_types`]). A `let`, field or
/// `:=` with a dotted path is two levels taller than
/// the taller of its value and the positions of its path's index segments
/// for each segment of the path after the first, since later stages build
/// a record, or a list, for each segment and read the old one's field or
/// element through all those before it. Every stage recurses once per
/// level, so the parser rejects anything deeper with an "expression nested
/// too deeply" error rather than letting a later stage overflow the stack.
/// The lexer, which recurses through string literals nested in each other's
/// interpolations, rejects those nested more than this many deep with the
/// same error.
///
/// The limit counts left-nested chains too: `1 + 1 + ... + 1` with ten
/// thousand terms is ten thousand levels deep, because each `+` is the left
/// operand of the next. Constructs that nest by recursion in the parser are
/// also limited by its recursion depth, where a nested block counts two
/// levels (its body and the expressions in it), so blocks nest at most
/// about `MAX_DEPTH / 2` deep.
///
/// It is also the budget for nesting at runtime, which calls share with
/// expressions, since calls nest on the Rust stack as expressions do and
/// GCSL does not eliminate tail calls. There, a level is a node of the IR:
/// every node on the path from a frame's body to the node being evaluated
/// is one level, the update of a dotted `let` or `:=` counting two more for
/// each segment of its path after the first, as the parser counts it,
/// although it walks the path without recursing, and each call in progress
/// costs [`CALL_LEVELS`] more on top of the nesting around it. The iterations of a loop do not
/// nest:
/// its body is one level below it, as an `if`'s branch is, however many
/// times it runs. A call is made only if the nesting it
/// would reach, the whole height of its callee's body included, stays
/// within this budget; otherwise it fails with "call nested too deeply".
/// An await that runs a future's body is charged as a call of a
/// parameterless function is, and a deep await one more level for each
/// level of record or list nesting it has descended. Nesting built by calls and
/// awaits therefore never exceeds `MAX_DEPTH` levels, whatever shape the
/// recursion has. (The top level's own nesting is not checked: it is
/// bounded by the parser's limit alone.)
pub const MAX_DEPTH: usize = 10_000;

/// The levels of [`MAX_DEPTH`]'s runtime budget that a call costs on top of
/// the nesting around it: the Rust frames a call itself adds, expressed in
/// levels of expression nesting. Measured in an unoptimised build, a call
/// takes about 9 KB of Rust stack, against 2.4 to 3.3 KB for a level of
/// expression nesting, depending on its kind, so a call is charged as four
/// levels.
pub const CALL_LEVELS: usize = 4;

/// The stack size, in bytes, that the pipeline is run with, even in an
/// unoptimised build. Run the pipeline on a thread with at least this much
/// stack: the `gcsl` binary does, and so should any embedder. Stack memory
/// is reserved, not committed, so an unused reservation costs nothing.
///
/// It must cover every stage at [`MAX_DEPTH`], and evaluation at the
/// runtime budget `MAX_DEPTH` sets: `MAX_DEPTH` levels of the costliest kind
/// of nesting, calls included, which the budget guarantees a program never
/// exceeds. The end-to-end tests check both, unoptimised: every stage on
/// trees `MAX_DEPTH` deep, and recursions that use the whole budget with
/// their calls wrapped in each costly kind of nesting.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;

/// What running a program successfully produces: its value, and the
/// warnings found on the way, in source order.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The value of the program's body.
    pub value: Value,
    /// The warnings found, in source order. The program ran regardless.
    pub warnings: Vec<Diagnostic>,
}

/// The standard library: the `.gcsl` files of the repository's `stdlib/`
/// directory, compiled in, each with its stem, by which it is imported
/// (`import prelude`, `import path`, `import fs`), in path order. Written
/// by hand, one entry per file; a test checks that it and the directory
/// agree. Each is a
/// scope of its own. A scan begins with those it loads, the prelude always
/// and each another file imports, in this order (see [`workspace`], "The
/// standard library"), named `<stdlib>/STEM.gcsl`
/// ([`workspace::check_with_standard`]); a session loads them all
/// ([`workspace::standard_sources`]).
pub const STANDARD: [(&str, &str); 9] = [
    ("archive", include_str!("../../../stdlib/archive.gcsl")),
    ("fs", include_str!("../../../stdlib/fs.gcsl")),
    ("json", include_str!("../../../stdlib/json.gcsl")),
    ("keychain", include_str!("../../../stdlib/keychain.gcsl")),
    ("libraries", include_str!("../../../stdlib/libraries.gcsl")),
    ("packaging", include_str!("../../../stdlib/packaging.gcsl")),
    ("path", include_str!("../../../stdlib/path.gcsl")),
    ("prelude", include_str!("../../../stdlib/prelude.gcsl")),
    ("toolchain", include_str!("../../../stdlib/toolchain.gcsl")),
];

/// Runs every stage on `source` as a file outside any workspace, beside the
/// standard library ([`workspace::check_lone`]), and returns the value of
/// its body with any warnings.
///
/// # Errors
///
/// Returns the diagnostics of the first stage that fails: every lexical and
/// syntax error, with the type errors of what parsed (see
/// [`workspace`], "Syntax errors"); every type error found by the checker;
/// or one runtime error. Any warnings found before the failure are among
/// them, all in source order, but those of a file with a syntax error.
/// The file's diagnostics name no input; one of a standard file, which is a
/// bug, names its input number, its index among the standard files the run
/// loaded ([`workspace::check_lone_with_standard`]). The file is taken to
/// be in the process's working directory, its `fileroot.this` and
/// `fileroot.workspace` ([`workspace::working_directory`]). The temporary
/// directories its run made with `fs.temp_dir` are removed before it
/// returns, whether the run succeeded or failed.
pub fn run(source: &SourceFile) -> Result<Outcome, Vec<Diagnostic>> {
    let dir = workspace::working_directory();
    let (loaded, checked) = workspace::check_lone_loading(source, &dir);
    let index = u32::try_from(loaded.len()).unwrap_or(u32::MAX);
    let being_run = |diagnostics| workspace::being_run(diagnostics, index);
    let mut workspace = checked.map_err(being_run)?;
    let outcome = workspace.run(index).map_err(being_run)?;
    Ok(Outcome {
        value: outcome.value,
        warnings: being_run(outcome.warnings),
    })
}
