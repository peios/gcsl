//! What a host asks of a workspace: def-types, definitions and their arguments,
//! and the errors of a request.
//!
//! # Def-types by name
//!
//! A program that turns def-types into commands (`pekit package
//! dev.peios.testpkg`) asks a checked workspace for its def-types and
//! invokes one of their definitions by name, as text; nothing here is
//! written in any source, and nothing of it changes what a file, a run or a
//! dump does. [`Workspace::def_types`](super::Workspace::def_types) lists every `def` marked
//! `#[def_type]` of the workspace's own files (not the standard files'),
//! but one whose definitions are made with a variant (see the checker's
//! "Variant def-types"), which are reached by references alone,
//! each by its path as declared ([`DefType`]), with the names of its
//! invocation parameters, as it declares them, and which of them are
//! optional, having a default (`version: String = ""`), and every definition made
//! with it ([`Definition`]): found by a search of every file, whichever
//! views see it, so a definition in a directory the root does not see is
//! found, and one a later one shadows too; grouped by the def-type its `DT`
//! names, its namespace's key (see [Def-types](crate::workspace::declared#def-types)), whatever
//! import or alias names it, so `def pkg x()` after `import package as pkg
//! from lib` is `package`'s `x`; each by its path as it writes it and its
//! directory ([`Definition::dir`]); all in path order of their files, then
//! in source order.
//!
//! A definition is named by its path, and where that is not enough, by its
//! directory too. The language scopes definitions by directory, as it does
//! fields: two sibling directories, or any two neither of which is above
//! the other, see nothing of each other's, and each may define one path,
//! as `pkgs/a` and `pkgs/b` may each define `build main`. So a path is
//! asked for alone, `main`, which names the only definition of it, or
//! qualified by its directory, `pkgs/a/main`, which names the definition of
//! it in that directory, and `/main` the root's ([`Definition::qualified`]);
//! a path alone that several directories define is
//! [`RequestError::Ambiguous`], "`build main` is defined in 2 directories,
//! so it is named with its directory", noting the qualified paths to name
//! instead, "name one of `pkgs/a/main`, `pkgs/b/main`".
//!
//! Within directories that see each other, one or one above another, a name
//! must still mean one thing, so the listing fails, and every invocation
//! with it, if a def-type has two definitions of one path in such
//! directories, "`package dev.peios.x` is defined 2 times in directories
//! that see each other", as a `#[shadowable]` one and the one that shadows
//! it are, or two def-types are declared of one path anywhere in the
//! workspace, "def-type `package` is declared 2 times in the workspace",
//! each at the first in path then source order, labelled "also defined
//! here" (or "also declared here") at each other: one error for each chain
//! of definitions of one path whose directories see each other, one
//! through another, so the root's one and those of two siblings below it
//! are one error of three. A definition whose path lies under another's of
//! its def-type in such a directory, `app.core` under `app`, in one through
//! `#[shadowable]` or under one it shadows from a directory above, is a
//! duplicate too: "`package app.core` lies under another `package`
//! definition", one error for each definition under one or more others, at
//! it, labelled "`app` is defined here" at each definition above it; a
//! definition both defined twice and under another has both errors. None
//! of these is an error of the language: the workspace checks and runs
//! regardless. A path above definitions that is none itself is no
//! duplicate, but names no definition to invoke.
//!
//! A path, as listed and as asked for, after the directory that qualifies
//! it, if one does, and its `/`, is the definition's as it writes it, its
//! names joined by `.`, a name with `+` in it as written
//! (`org.gnu.gmp-c++`), and one starting with a digit too
//! (`org.golang.go-stage0-1.24`, `x.1.83.0`, the float `1.83` being the
//! names `1` and `83`; see the parser's "Def-types"). A path asked for is
//! not parsed or checked against the rule of names: its directory, the text
//! before its last `/`, without a leading `/`, is compared with the
//! definitions' directories, and the rest with their paths, as text, so it
//! names the definition whose path it spells, in its directory if it gives
//! one, or none, the error of a path that names none, whose notes list the
//! definitions by the names that name them: a path alone, and one several
//! directories define qualified by each. One path lies under
//! another only at a `.`, so `x.a+b` is not under `x.a`, and `x.a+b.c` is
//! under `x.a+b`; `x.1.83` is under `x.1`, and `x.10` is not.
//!
//! [`Workspace::invoke_with`](super::Workspace::invoke_with) invokes a
//! definition as [`Workspace::invoke`](super::Workspace::invoke) does, in a
//! context the host sets (see the checker's
//! [Context](crate::check::context#context)): each a context's name and a value
//! of its declared type, data the host has from an earlier run, bound around
//! the call as a `with` binds it. What a file computes as it runs reads the
//! declared defaults, whatever invokes it, so a file's cells never depend on an
//! invocation's context, and a memoised call is found only from a context
//! giving every context it read the same value. Pekit sets `host_platform` so
//! for each target it evaluates.
//!
//! [`Workspace::invoke`](super::Workspace::invoke) calls the definition as `DT
//! PATH(NAME = VALUE, ...)` would be called from a file of the definition's
//! directory, with its arguments BY NAME, each the name of one of the
//! def-type's invocation parameters and a value ([`Given`]), in any order; each
//! invocation parameter given none takes its default (see the checker's
//! "Calls"), if it has one, whichever others are given: so `def lock(name:
//! String, func: fn(version: String) -> List<Version>, version: String = "")`
//! invoked with none gives `version` `""`, and `def build(name: String, f:
//! fn(): String, version: String = "", target: String)` may be given `target`
//! alone. A value is of one of six types, `String`, `Int`, `Float`, `Bool`,
//! `Duration` and `Size`, and is given only for a parameter of that type as it
//! is declared, an alias of it, or the either shape of an impure def-type's
//! bare one (`version: String` of an impure def-type is `String |
//! Future<String>`, of a memoised one `String`); a parameter of any other type,
//! a union, a `Future`, a record or a list, cannot be given one, and so a
//! def-type with such a parameter that has no default cannot be invoked so,
//! while one with a default is left out and takes it.
//! [`Workspace::arguments`](super::Workspace::arguments) reads them from text,
//! as a command line gives them, each by its parameter's type: a `String` the
//! text as it is, read as nothing else (no `{` in it interpolates), an `Int` a
//! decimal integer, with `-` before it if negative, in range, a `Float` a
//! decimal number Rust's `f64` reads, finite, a `Bool` `true` or `false`, a
//! `Duration` or a `Size` one of its literals, `30s` or `2GiB`, with `-` before
//! it if negative; it runs nothing, so a host can check every request before it
//! runs any. What is asked for may name nothing: no def-type of the name, no
//! definition of the path, or a path above definitions; or the arguments may
//! not fit the def-type: an invocation parameter that has no default and cannot
//! be given a value, "`build`'s parameter `n` is `List<Int>`, but only
//! `String`, `Int`, `Float`, `Bool`, `Duration` and `Size` parameters can be
//! given here yet", the first such, its type as it is written, whatever is
//! given; then, argument by argument, a name that is no invocation parameter's,
//! "`package` has no invocation parameter `jbos`", noting those it has, one
//! named twice, "`package`'s parameter `version` is given more than once", one
//! given for a parameter that cannot be given a value (the same error as
//! above), or a text that is not a value of its parameter's type, "`x` is not
//! an `Int`, the type of `jobs` of `package org.foo`", noting how one is
//! written; and last the parameters that have no default and are given none,
//! "`deploy` needs `target` and `mode`, which were not given". Each is a
//! [`RequestError`], an error of no source, in that order. Then the
//! definition's file and the def-type's run as a reference to the definition
//! needs them to, each as a whole, with the files they need (its meta's among
//! them, if it has one), unless they have run, as for
//! [`Workspace::run`](super::Workspace::run), and the call is made: an input of
//! its own, built rather than parsed, `<def-type>("PATH", <definition>, NAME =
//! VALUE, ...)` as its result, each value a literal of its kind (a string
//! literal of the text given, an integer, a float, or `true` or `false`), named
//! by its parameter, whose two names no source can write are bound, in a scope
//! of the workspace's slots, as fields of the def-type's cell and the
//! definition's (the slots of their closures), with their types, and the
//! def-type's with what a call of it needs to know of its parameters, as a
//! session's input knows a function of an earlier one, so that the call is of a
//! known callee and gives each optional parameter given none its default. It is
//! checked as an input of no workspace, so that it looks nothing up in any
//! file's view and needs no alias, placed in the definition's directory, so
//! that the def-type's caller (`fileroot.caller`) is that directory, as for a
//! call written there, and the definition's is what the def-type makes it;
//! lowered with an input number after every file's, and run on the workspace's
//! frame and memo tables. It takes no cell and commits nothing, so it changes
//! nothing any file sees, and invoked again it is made and run again, a call
//! like any other, the files not running again. Every diagnostic it returns
//! names a file of the workspace or a standard file: the call has no source, so
//! a diagnostic or a label located in it ("call nested too deeply" at the call,
//! for a def-type's body nested within a few levels of the limit) is located at
//! the definition's path in its file instead, labelled "invoked here".
//!
//! A value a host gets back may hold a reference, `build = build busybox`,
//! which it can call only by name.
//! [`Workspace::definition_of`](super::Workspace::definition_of) names it: the
//! closure of a reference is of a function the program records as made for one
//! (see [`crate::lower`]), which captures the def-type and then the definition,
//! and the definition is found as the definition whose cell holds the very
//! closure captured, its def-type by its path as declared, however the
//! reference names it. Any other value names none, whatever it captures. It
//! runs nothing.
//!
//! # Top-level functions by name
//!
//! A host may give a file of the workspace a meaning of its own (`pekit` its
//! root's `pekit.gcsl`, whose `environment` it tears down with its `teardown`),
//! which nothing here knows of: it asks for the file's top-level fields and
//! `def`s by name, the fields of the file's own record that another file reads
//! as members of its module. A private name (`let`, `fn`) is none, and nor is a
//! dotted path under the name, or a declaration a later one shadows in its own
//! file. [`Workspace::top_level`](super::Workspace::top_level) tells one's
//! type, as it is written (a `def`'s as `fn(env: Vm) -> {}`), and, for a
//! function, what it may do and how many parameters it takes;
//! [`Workspace::memoised`](super::Workspace::memoised) whether a memoised
//! (`->>`) function has returned for no arguments in the workspace's memo
//! tables, which every run, invocation and call shares for the workspace's
//! life, as a call of it written in the file's directory would find;
//! [`Workspace::call_composed`](super::Workspace::call_composed) calls a
//! composition of them, `teardown(environment())` for `["teardown",
//! "environment"]`, as if written in a file of the file's directory, after the
//! file and those it needs have run, as the call that invokes a definition is
//! made, built rather than parsed, the functions bound by names no source can
//! write, checked, lowered and run on the workspace's frame and memo tables, so
//! a memoised function called before is not run again; and
//! [`Workspace::check_composed`](super::Workspace::check_composed) checks that
//! call without running anything, which says whether each function takes what
//! the next returns, as the checker says it of a call. Every diagnostic of the
//! call located in it is located at the first function's name instead, labelled
//! "invoked here".

use crate::ast::ItemId;
use crate::check::Slot;
use crate::diagnostic::Diagnostic;
use crate::source::Span;
use crate::types::TyId;
use crate::value::{Quantity, Value};

use super::quantity_literal;

/// A def-type of a workspace, as
/// [`Workspace::def_types`](super::Workspace::def_types) lists it: a `def`
/// marked `#[def_type]` of one of the workspace's files, with every definition
/// made with it (see the module documentation, "Def-types by name").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefType {
    /// Its path, as it is declared: `package`, or `t.package` for a `def`
    /// named by a dotted path. It is invoked by it.
    pub path: String,
    /// The input number of its file.
    pub file: u32,
    /// Where its name, its whole path, is written in its file.
    pub span: Span,
    /// The names of its invocation parameters, its parameters after the
    /// definition's path and the definition, as it declares them, in order:
    /// what invoking one of its definitions takes arguments by the names of.
    pub parameters: Vec<String>,
    /// For each of its invocation parameters, in the order of `parameters`,
    /// whether it is optional: whether it has a default, `version: String =
    /// ""`, which invoking one of its definitions without an argument for
    /// it gives it, whichever others are given (see the module
    /// documentation, "Def-types by name").
    pub optional: Vec<bool>,
    /// Every definition made with it, in path order of their files, then in
    /// source order: none if it has none.
    pub definitions: Vec<Definition>,
}

/// A definition made with a def-type, `def package dev.peios.testpkg() {
/// ... }`, as [`Workspace::def_types`](super::Workspace::def_types) lists it (see the module
/// documentation, "Def-types by name").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Its path, as the definition writes it: `dev.peios.testpkg`. It is
    /// invoked by it, alone if no other definition of its def-type has its
    /// path, else qualified by its directory ([`Definition::qualified`]).
    pub path: String,
    /// The path of its file's directory relative to the root, as
    /// [`Dir::path`](super::Dir::path) gives it, `pkgs/net.busybox.busybox`; empty for the
    /// root.
    pub dir: String,
    /// The input number of its file.
    pub file: u32,
    /// Where its path is written in its file.
    pub span: Span,
}

impl Definition {
    /// Its path qualified by its directory, as
    /// [`Workspace::invoke`](super::Workspace::invoke) takes it to name this
    /// definition whatever others have its path:
    /// `pkgs/net.busybox.busybox/busybox`, and for one of the root, `/busybox`
    /// (see the module documentation, "Def-types by name").
    #[must_use]
    pub fn qualified(&self) -> String {
        qualified(&self.dir, &self.path)
    }
}

/// The path `path` of a definition of the directory `dir` qualified by it:
/// `dir/path`, and `/path` for the root (see [`Definition::qualified`]).
pub(super) fn qualified(dir: &str, path: &str) -> String {
    format!("{dir}/{path}")
}

/// The definition a def-type reference names, as
/// [`Workspace::definition_of`](super::Workspace::definition_of) finds it: what
/// [`Workspace::invoke`](super::Workspace::invoke) and
/// [`Workspace::meta_field`](super::Workspace::meta_field) take to invoke it, or read its meta, by
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Referenced {
    /// The def-type's path, as it is declared: `build`.
    pub def_type: String,
    /// The definition's path, as it writes it: `busybox`.
    pub path: String,
    /// The path of the definition's directory relative to the root, empty
    /// for the root, as [`Definition::dir`] gives it.
    pub dir: String,
}

impl Referenced {
    /// The definition's path qualified by its directory, as
    /// [`Definition::qualified`] gives it: what names this very definition to
    /// [`Workspace::invoke`](super::Workspace::invoke) and
    /// [`Workspace::meta_field`](super::Workspace::meta_field), whatever others
    /// have its path.
    #[must_use]
    pub fn qualified(&self) -> String {
        qualified(&self.dir, &self.path)
    }
}

/// Why [`Workspace::invoke`](super::Workspace::invoke) could not invoke a definition, or another
/// call by name could not be made ([`Workspace::call_composed`](super::Workspace::call_composed)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvokeError {
    /// What the workspace's files say: the duplicates that
    /// [`Workspace::def_types`](super::Workspace::def_types) fails with, or the
    /// runtime error of a file that ran or of the call, with the workspace's
    /// warnings, as [`Workspace::run`](super::Workspace::run) returns them.
    /// Each names its file, a standard file or one of the workspace's.
    Diagnostics(Vec<Diagnostic>),
    /// What was asked for names nothing that can be invoked, or its
    /// arguments do not fit the def-type's invocation parameters: an error
    /// of no source, which is shown as a line and its notes.
    Request(RequestError),
}

/// A def-type and a definition asked for by name that cannot be invoked,
/// or not with the arguments given (see the module documentation,
/// "Def-types by name"): an error of no
/// source, so no [`Diagnostic`], which needs a span. Shown, it is its message,
/// [`RequestError::message`], which its notes follow,
/// [`RequestError::notes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// No def-type of the workspace is of the name asked for: "no def-type
    /// `pkg`".
    NoDefType {
        /// The name asked for.
        name: String,
        /// The paths of the workspace's def-types, sorted.
        def_types: Vec<String>,
    },
    /// The def-type has no definition of the path asked for: "`package`
    /// has no definition `dev.peios.nope`".
    NoDefinition {
        /// The def-type's path.
        def_type: String,
        /// The path asked for.
        path: String,
        /// The paths of the def-type's definitions, sorted.
        definitions: Vec<String>,
    },
    /// The path asked for, alone, is the path of definitions of the def-type
    /// in several directories, none of which sees another: "`build main` is
    /// defined in 2 directories, so it is named with its directory".
    Ambiguous {
        /// The def-type's path.
        def_type: String,
        /// The path asked for.
        path: String,
        /// The path of each definition of it qualified by its directory
        /// ([`Definition::qualified`]), sorted.
        candidates: Vec<String>,
    },
    /// The path asked for is a prefix of the paths of definitions of the
    /// def-type, none of which it is: "`dev.peios` is not a `package`
    /// definition".
    NotADefinition {
        /// The def-type's path.
        def_type: String,
        /// The path asked for.
        path: String,
        /// The paths of the def-type's definitions, sorted.
        definitions: Vec<String>,
    },
    /// An invocation parameter of the def-type that is given an argument, or
    /// that must be, having no default, is of a type no argument can be
    /// given for: only a `String`, `Int`, `Float` or `Bool` one can be (see
    /// [`Given`]): "`build`'s parameter `n` is `List<Int>`, but only
    /// `String`, `Int`, `Float`, `Bool`, `Duration` and `Size` parameters can be given here
    /// yet". One that must be given is reported whatever is given, the
    /// first as it declares them, before anything given is looked at.
    CannotBeGiven {
        /// The def-type's path.
        def_type: String,
        /// The parameter's name.
        parameter: String,
        /// The parameter's type, as it is written.
        ty: String,
    },
    /// An argument names no invocation parameter of the def-type:
    /// "`package` has no invocation parameter `jbos`".
    UnknownParameter {
        /// The def-type's path.
        def_type: String,
        /// The name the argument gives.
        parameter: String,
        /// The names of its invocation parameters, as it declares them, in
        /// order.
        parameters: Vec<String>,
    },
    /// Two arguments or more name one invocation parameter: "`package`'s
    /// parameter `version` is given more than once".
    DuplicateArgument {
        /// The def-type's path.
        def_type: String,
        /// The parameter's name.
        parameter: String,
    },
    /// An argument's text is not a value of its parameter's type as the
    /// parameter's type reads it, or a value of another type is given for
    /// it: "`x` is not an `Int`, the type of `jobs` of `package org.foo`".
    CannotParse {
        /// The def-type's path.
        def_type: String,
        /// The definition's path.
        path: String,
        /// The parameter's name.
        parameter: String,
        /// Which of the four types that can be given the parameter's is,
        /// named as it is written: `"String"`, `"Int"`, `"Float"` or
        /// `"Bool"` (an alias's, or an impure def-type's either shape's).
        expected: &'static str,
        /// The text given, or the value given, as [`Given`] shows it.
        text: String,
    },
    /// Invocation parameters that have no default are given no argument:
    /// "`package` needs `version`, which was not given", "`deploy` needs
    /// `target` and `mode`, which were not given".
    MissingArguments {
        /// The def-type's path.
        def_type: String,
        /// The names of the parameters given none, as it declares them, in
        /// order.
        parameters: Vec<String>,
    },
    /// A function asked for by name to call is no top-level function of its
    /// file ([`Workspace::call_composed`](super::Workspace::call_composed)):
    /// "`pekit.gcsl` has no top-level function `teardown`".
    NoFunction {
        /// The file's path, relative to the root.
        file: String,
        /// The name asked for.
        name: String,
    },
    /// A field a host sets a root context from is no top-level field of its
    /// file ([`Workspace::prepare_call_with`](super::Workspace::prepare_call_with)):
    /// "`workspace.gcsl` has no top-level field `pekitconf`".
    NoField {
        /// The file's path, relative to the root.
        file: String,
        /// The name asked for.
        name: String,
    },
    /// A context a host sets for an invocation is declared nowhere in the
    /// workspace ([`Workspace::invoke_with`](super::Workspace::invoke_with)): "no context `arhc` is
    /// declared".
    NoContext {
        /// The name given.
        name: String,
        /// The names of the workspace's contexts, sorted.
        contexts: Vec<String>,
    },
    /// A value a host sets a context to for an invocation is not of its
    /// type ([`TypeTable::holds`](super::TypeTable::holds)): "the value given for context `arch` is
    /// not a `String`".
    ContextValue {
        /// The context's name.
        name: String,
        /// Its declared type, shown.
        ty: String,
    },
    /// A context a host sets for an invocation is given more than once.
    DuplicateContext {
        /// The context's name.
        name: String,
    },
}

impl RequestError {
    /// The line that says what is wrong, without a trailing full stop.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            RequestError::NoDefType { name, .. } => format!("no def-type `{name}`"),
            RequestError::NoDefinition { def_type, path, .. } => {
                format!("`{def_type}` has no definition `{path}`")
            }
            RequestError::NotADefinition { def_type, path, .. } => {
                format!("`{path}` is not a `{def_type}` definition")
            }
            RequestError::Ambiguous {
                def_type,
                path,
                candidates,
            } => format!(
                "`{def_type} {path}` is defined in {} directories, so it is named with its \
                 directory",
                candidates.len()
            ),
            RequestError::CannotBeGiven {
                def_type,
                parameter,
                ty,
            } => format!(
                "`{def_type}`'s parameter `{parameter}` is `{ty}`, but only `String`, `Int`, \
                 `Float`, `Bool`, `Duration` and `Size` parameters can be given here yet"
            ),
            RequestError::UnknownParameter {
                def_type,
                parameter,
                ..
            } => format!("`{def_type}` has no invocation parameter `{parameter}`"),
            RequestError::DuplicateArgument {
                def_type,
                parameter,
            } => format!("`{def_type}`'s parameter `{parameter}` is given more than once"),
            RequestError::CannotParse {
                def_type,
                path,
                parameter,
                expected,
                text,
            } => {
                let article = if *expected == "Int" { "an" } else { "a" };
                format!(
                    "`{text}` is not {article} `{expected}`, the type of `{parameter}` of \
                     `{def_type} {path}`"
                )
            }
            RequestError::MissingArguments {
                def_type,
                parameters,
            } => {
                let was = if parameters.len() == 1 { "was" } else { "were" };
                format!(
                    "`{def_type}` needs {}, which {was} not given",
                    and_list(parameters)
                )
            }
            RequestError::NoFunction { file, name } => {
                format!("`{file}` has no top-level function `{name}`")
            }
            RequestError::NoField { file, name } => {
                format!("`{file}` has no top-level field `{name}`")
            }
            RequestError::NoContext { name, .. } => format!("no context `{name}` is declared"),
            RequestError::ContextValue { name, ty } => {
                format!("the value given for context `{name}` is not a `{ty}`")
            }
            RequestError::DuplicateContext { name } => {
                format!("context `{name}` is given more than once")
            }
        }
    }

    /// The notes that follow the message: what there is to ask for instead,
    /// "the def-types are `build`, `package`", or with one, "the only
    /// def-type is `package`", and so "its definitions are `a`, `b`" and
    /// "its only definition is `a`", and "its invocation parameters are
    /// `a`, `b`" and "its only invocation parameter is `a`"; or how a value
    /// of the type asked for is written, "a `Bool` is written `true` or
    /// `false`".
    #[must_use]
    pub fn notes(&self) -> Vec<String> {
        match self {
            RequestError::NoDefType { def_types, .. } if def_types.is_empty() => {
                vec!["this workspace declares no def-types".to_owned()]
            }
            RequestError::NoDefType { def_types, .. } => match &def_types[..] {
                [only] => vec![format!("the only def-type is `{only}`")],
                _ => vec![format!("the def-types are {}", quoted_list(def_types))],
            },
            RequestError::NoDefinition { definitions, .. }
            | RequestError::NotADefinition { definitions, .. }
                if definitions.is_empty() =>
            {
                vec!["it has no definitions".to_owned()]
            }
            RequestError::NoDefinition { definitions, .. }
            | RequestError::NotADefinition { definitions, .. } => match &definitions[..] {
                [only] => vec![format!("its only definition is `{only}`")],
                _ => vec![format!("its definitions are {}", quoted_list(definitions))],
            },
            RequestError::Ambiguous { candidates, .. } => {
                vec![format!("name one of {}", quoted_list(candidates))]
            }
            RequestError::UnknownParameter { parameters, .. } => match &parameters[..] {
                [] => vec!["it has no invocation parameters".to_owned()],
                [only] => vec![format!("its only invocation parameter is `{only}`")],
                _ => vec![format!(
                    "its invocation parameters are {}",
                    quoted_list(parameters)
                )],
            },
            RequestError::CannotParse { expected, .. } => match *expected {
                "Int" => vec![format!(
                    "an `Int` is written as a decimal integer, with `-` before it if it is \
                     negative, from {} to {}",
                    i64::MIN,
                    i64::MAX
                )],
                "Float" => vec![
                    "a `Float` is written as a decimal number, `1.5` or `2`, and must be finite"
                        .to_owned(),
                ],
                "Bool" => vec!["a `Bool` is written `true` or `false`".to_owned()],
                "Duration" => vec![
                    "a `Duration` is written as a duration literal, `30s` or `1h30m`, with `-` \
                     before it if it is negative"
                        .to_owned(),
                ],
                "Size" => vec![
                    "a `Size` is written as a size literal, `512B` or `2GiB`, with `-` before it \
                     if it is negative"
                        .to_owned(),
                ],
                _ => Vec::new(),
            },
            RequestError::NoContext { contexts, .. } => match &contexts[..] {
                [] => vec!["this workspace declares no contexts".to_owned()],
                [only] => vec![format!("the only context is `{only}`")],
                _ => vec![format!("the contexts are {}", quoted_list(contexts))],
            },
            RequestError::ContextValue { .. } => {
                vec!["a context is given data of its type: no function or future".to_owned()]
            }
            RequestError::CannotBeGiven { .. }
            | RequestError::DuplicateArgument { .. }
            | RequestError::MissingArguments { .. }
            | RequestError::NoFunction { .. }
            | RequestError::NoField { .. }
            | RequestError::DuplicateContext { .. } => Vec::new(),
        }
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for RequestError {}

/// `items`, each in backquotes, joined by `, `.
pub(super) fn quoted_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `items`, each in backquotes, the last two joined by ` and `, the others
/// by `, `: "`a`", "`a` and `b`", "`a`, `b` and `c`".
pub(super) fn and_list(items: &[String]) -> String {
    match items {
        [rest @ .., last] if !rest.is_empty() => {
            format!("{} and `{last}`", quoted_list(rest))
        }
        _ => quoted_list(items),
    }
}

/// A value given for an invocation parameter of a def-type, to invoke one of
/// its definitions with ([`Workspace::invoke`](super::Workspace::invoke)): of
/// one of the six types a parameter given an argument can be, `String`, `Int`,
/// `Float`, `Bool`, `Duration` or `Size`, which the call that invokes the
/// definition writes as a literal of its kind (see the module documentation,
/// "Def-types by name"). [`Workspace::arguments`](super::Workspace::arguments)
/// reads one from text, by the parameter's type.
#[derive(Clone, Debug, PartialEq)]
pub enum Given {
    /// A `String`.
    Str(String),
    /// An `Int`.
    Int(i64),
    /// A `Float`, which must be finite.
    Float(f64),
    /// A `Bool`.
    Bool(bool),
    /// A `Duration` or a `Size`, of its smallest unit.
    Quantity(Quantity, i64),
}

impl std::fmt::Display for Given {
    /// The value as text: a string as it is, unquoted; a number in decimal;
    /// a `Bool` as `true` or `false`; a duration or a size as its canonical
    /// literal, `1m30s`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Given::Str(text) => f.write_str(text),
            Given::Int(value) => write!(f, "{value}"),
            Given::Float(value) => write!(f, "{value}"),
            Given::Bool(value) => write!(f, "{value}"),
            &Given::Quantity(quantity, amount) => Value::Quantity(quantity, amount).fmt(f),
        }
    }
}

/// One of the six types an invocation parameter given an argument can be
/// (see [`Given`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scalar {
    /// `String`.
    String,
    /// `Int`.
    Int,
    /// `Float`.
    Float,
    /// `Bool`.
    Bool,
    /// `Duration` or `Size`.
    Quantity(Quantity),
}

impl Scalar {
    /// The six.
    pub(super) const ALL: [Scalar; 6] = [
        Scalar::String,
        Scalar::Int,
        Scalar::Float,
        Scalar::Bool,
        Scalar::Quantity(Quantity::Duration),
        Scalar::Quantity(Quantity::Size),
    ];

    /// Its type.
    pub(super) fn ty(self) -> TyId {
        match self {
            Scalar::String => TyId::STRING,
            Scalar::Int => TyId::INT,
            Scalar::Float => TyId::FLOAT,
            Scalar::Bool => TyId::BOOL,
            Scalar::Quantity(quantity) => TyId::of_quantity(quantity),
        }
    }

    /// Its name, as a type is written.
    pub(super) fn name(self) -> &'static str {
        match self {
            Scalar::String => "String",
            Scalar::Int => "Int",
            Scalar::Float => "Float",
            Scalar::Bool => "Bool",
            Scalar::Quantity(quantity) => quantity.type_name(),
        }
    }

    /// The value of this type `text` is, if it is one: for a `String`, the
    /// text as it is; for an `Int`, a decimal integer, with `-` before it if
    /// negative (no `+`), in range; for a `Float`, a number Rust's `f64`
    /// reads (`1.5`, `2`, `1e3`), finite; for a `Bool`, `true` or `false`;
    /// for a `Duration` or a `Size`, one of its literals, with `-` before it
    /// if negative, as the lexer reads it ([`quantity_literal`]).
    pub(super) fn read(self, text: &str) -> Option<Given> {
        match self {
            Scalar::String => Some(Given::Str(text.to_owned())),
            Scalar::Int => {
                let digits = text.strip_prefix('-').unwrap_or(text);
                let decimal = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
                decimal
                    .then(|| text.parse::<i64>().ok())
                    .flatten()
                    .map(Given::Int)
            }
            Scalar::Float => text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Given::Float),
            Scalar::Bool => match text {
                "true" => Some(Given::Bool(true)),
                "false" => Some(Given::Bool(false)),
                _ => None,
            },
            Scalar::Quantity(quantity) => quantity_literal(text)
                .filter(|&(read, _)| read == quantity)
                .map(|(read, amount)| Given::Quantity(read, amount)),
        }
    }

    /// Whether `given` is a value of this type: of its kind, and, for a
    /// `Float`, finite.
    pub(super) fn fits(self, given: &Given) -> bool {
        match (self, given) {
            (Scalar::String, Given::Str(_))
            | (Scalar::Int, Given::Int(_))
            | (Scalar::Bool, Given::Bool(_)) => true,
            (Scalar::Quantity(quantity), &Given::Quantity(given, _)) => quantity == given,
            (Scalar::Float, Given::Float(value)) => value.is_finite(),
            _ => false,
        }
    }
}

/// An argument for an invocation parameter, as [`Workspace::given`](super::Workspace::given) reads
/// it: the text of a value, read by the parameter's type, or none, its value
/// not known yet; or a value.
#[derive(Clone, Copy)]
pub(super) enum Argument<'a> {
    /// The text of the value, or none.
    Text(Option<&'a str>),
    /// The value.
    Given(&'a Given),
}

/// An invocation parameter of a def-type, as [`Workspace::given`](super::Workspace::given) reads
/// arguments for it.
pub(super) struct Invocable<'a> {
    /// Its name.
    pub(super) name: &'a str,
    /// Its type, as it is written.
    pub(super) written: &'a str,
    /// Whether it has a default.
    pub(super) optional: bool,
    /// Which of the four types an argument can be given for it is, if one.
    pub(super) scalar: Option<Scalar>,
}

/// A def-type as [`Workspace::def_types`](super::Workspace::def_types) finds
/// it, with what invoking one of its definitions needs besides: its item, the
/// type of each of its invocation parameters as it is written, and the item of
/// each of its definitions, in the order of `listed`'s.
#[derive(Debug)]
pub(super) struct FoundDefType {
    /// The def-type as it is listed.
    pub(super) listed: DefType,
    /// Its `def`'s item.
    pub(super) item: ItemId,
    /// The type of each of its invocation parameters, as it is written, in
    /// the order of `listed`'s parameters.
    pub(super) written: Vec<String>,
    /// The item of each of its definitions.
    pub(super) items: Vec<ItemId>,
}

/// A definition resolved for a host request, with both typed runtime cells.
/// Named fields keep the definition's index distinct from its storage slot.
pub(super) struct ResolvedDefinition {
    pub(super) def_type: FoundDefType,
    pub(super) definition_index: usize,
    pub(super) def_type_cell: (Slot, TyId),
    pub(super) definition_cell: (Slot, TyId),
}

/// The name, no source can write it, that the call invoking a definition by
/// name reads the def-type by (see [Def-types by name](#def-types-by-name)).
pub(super) const INVOKED_DEF_TYPE: &str = "<def-type>";

/// The name, no source can write it, that the call invoking a definition by
/// name reads the definition by.
pub(super) const INVOKED_DEFINITION: &str = "<definition>";
