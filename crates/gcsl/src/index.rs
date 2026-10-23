//! An index of a checked workspace, for a language server: what each name
//! written in a file refers to, where that is declared, and what it is.
//!
//! [`crate::workspace::index_with_standard`] (and, for a lone file,
//! [`crate::workspace::index_lone_with_standard`]) checks a workspace as
//! `gcsl run` does, with the same diagnostics, each file's check recording
//! what it resolved by scope (see [`crate::check`], "Recording"), and builds
//! an [`Index`] of what it found, WHETHER THE CHECK SUCCEEDED OR FAILED: in a
//! file with type errors, or with syntax errors its parser recovered from,
//! everything that resolved is indexed, an expression that failed (of type
//! `Error`) answering nothing. A file whose braces do not match
//! ([`crate::ast::Syntax::Unbalanced`]) is not checked, and has nothing
//! indexed: a query at a place of it is answered from the workspace
//! indexed again with its braces balanced virtually (see "Unbalanced
//! files"). A file that needs a file whose check failed, which `gcsl run`
//! does not check (see [`crate::workspace`], "Syntax errors"), is checked
//! for its index alone once every other file is, a name that names nothing
//! there no error (it may be declared in the file that failed), and what
//! its check finds thrown away, so that the diagnostics are still exactly
//! `gcsl run`'s: it is indexed for what resolved, a name that names nothing
//! being no occurrence. What it reads of a file that failed through an
//! import has the type that file's check found; what it reads of one of
//! its own directory chain, whose declarations were never committed, has
//! none.
//!
//! # Occurrences
//!
//! Each file's index is a list of OCCURRENCES ([`Occurrence`]), in source
//! order: a span written in the file (a name, a segment of a path, a type's
//! name) and the [`Symbol`] it refers to, if it refers to one that has an
//! identity, with whether it is the symbol's declaration, whether it writes
//! it (a declaration, or a `:=`), and the type the check gave what is
//! written there. Everything a language server answers is read from them:
//! a hover from the occurrence at a place, the definition from its symbol's
//! declaration sites, and the references to a symbol from the occurrences
//! of it, the link table inverted (see "References"). They are found once,
//! as the index is built, from the trees, the scan and what the checks
//! recorded:
//!
//! - A binding declared in a file (a `let`, `let mut`, `fn`, parameter,
//!   `for` variable, or field of a record block) is a [`Symbol::Local`], by
//!   its file and its name's span; its declaration is an occurrence, and so
//!   is each `Name` the check read from it, as it recorded. A name read
//!   from the narrowed copy of a variable (`if x is Int { x }`) is the
//!   variable's, and a function's name for itself the function's. A `:=`'s
//!   variable is an occurrence of the variable it rebinds, which writes it:
//!   a variable's `:=` declares a binding in place of the old one, which is
//!   taken as the same local, so its reads after it are the old one's.
//! - Any other `Name`, and each field access directly on it, `a.b.c`, is
//!   resolved in the file's view ([`FileScope`]) as the check resolves it,
//!   to the longest prefix the view declares: each segment of it an
//!   occurrence of the workspace's node there, a [`Symbol::Node`] (a cell),
//!   a [`Symbol::View`] (a node several sources of the view share) or a
//!   [`Symbol::Module`] (a directory, a file or a file's part read through
//!   an import, or the `fileroot` module); the field accesses after it are
//!   accesses of the values' fields. A top-level field item's path is the
//!   declaration of a node at each of its segments.
//! - A field access that no node is, on any value, is an occurrence of the
//!   field read, of the type the check gave it. It has a symbol only when
//!   the record's origin is cheap to find (best effort): a record block
//!   written as the value of the variable, field or node read, whose field
//!   item of that name is then the symbol; nothing else is tracked.
//! - A type's name in a type position, bare or qualified, is a
//!   [`Symbol::Type`] (its `type` item's), or a [`Symbol::BuiltinType`]; a
//!   `type` item's name is its declaration.
//! - An import's names are occurrences of what they bind, and each prefix
//!   of its path of the module it names.
//! - A def-type reference's and a definition's path, `package
//!   dev.peios.x`, are occurrences of the definition's node in the def-type's
//!   namespace, a segment past a definition of a field of its meta; a
//!   definition's path goes, as a definition, to its def-type's `def` (see
//!   [`Occurrence::jump`]). Their def-type is an ordinary name.
//!
//! Keywords, punctuation, literals, comments and whitespace are no
//! occurrence, and nor is a name that names nothing.
//!
//! # Declarations
//!
//! The declaration sites of a symbol ([`Index::definition`]) are where its
//! name is written as it is declared, not its whole item: a local's name;
//! for a node, each item at or under it, in whichever files, at the
//! segment of its path at the node's depth (an interior node, of several
//! items of one file or of several files, has several), and for a node
//! the scan found conflicting, each declaration it dropped as a later
//! definition of it too (see [`crate::workspace`], "Scan errors"), so that
//! a read of it goes to every definition a fix chooses among, while it has
//! no hover, being erroneous; for a node several
//! sources of a view share, every part's; for a directory, its first file's
//! first declaration, and for a file its start; a type's name. A symbol of
//! the standard library or a built-in has none: the standard files are
//! compiled in and have no file on disk.
//!
//! # Hovers
//!
//! A hover ([`Index::hover`]) is Markdown: the occurrence as GCSL, in a
//! `gcsl` code block, `name: Type`, a function as its signature, `fn
//! name(a: Int): Int` or `def name(...)`, its parameters named as its source
//! names them, a mutable variable `mut name: Type`, a type `type Name =
//! TYPE` with its expansion, a built-in type its name; then its
//! documentation, if it has any (see "Documentation"); then, if useful, one
//! line saying where it comes from ("parameter", "field of directory
//! `pkgs/`", "standard library `path`", "built-in", "imported from `a.b`").
//! Types are shown by the printer diagnostics use
//! ([`TypeTable::display`]); a line wider than 80 columns, a hover
//! panel's width, lays its record out a field per line, or its
//! signature or function type a parameter per line, each indented by 2;
//! and anything of more than [`MAX_LINES`] lines is cut short with `…`. An
//! occurrence whose type the check found erroneous, or did not find, has no
//! hover, but a type's, a module's and a node's whose type another file
//! gives.
//!
//! # Documentation
//!
//! The DOC COMMENT of a declaration (a `let`, a field, a `fn`, a `def`, a
//! `defmeta`, a definition or a `type` item) is the run of comment lines
//! directly above its item, its attributes' lines apart, with no blank line
//! between: each line a comment alone (a trailing comment, on a line with
//! code, is none, so neither is the declaration's own), shown without its
//! `#` and one space after it, blank lines at either end left out
//! ([`Index::documentation`]). A run that starts on the file's first line,
//! a shebang's run included, is the file's HEADER, which describes the
//! file, and is no declaration's doc comment, whatever is below it: a
//! package's file starts with its description above its imports, and a
//! lock file with how it was written, above its one field. The comments are the lexer's
//! ([`crate::lexer::lex_with_comments`]), so an attribute, `#[...]`, is
//! none, and neither is a `#` in a string. A node declared by several
//! items is documented by the first of them, in input then source order,
//! that has a doc comment, and a definition with none by its meta's, since
//! a package is described above its `defmeta`; an interior node, written by
//! no item of its own, a parameter and a module have none.
//!
//! # References
//!
//! The REFERENCES of what is at a place ([`Index::references`]) are the
//! occurrences of its symbol in every file of the workspace (none in the
//! standard files'), its declarations among them if asked: a definition's
//! path counts as the declaration of its node, a reference to it,
//! `package x.y`, as a use. Occurrences are filed by their symbols once,
//! the first time references are asked for, in one pass over every file's:
//! a node several sources of a view share is filed under each node it
//! merges, and a file's part of a node under the node, so that a read
//! through a shared root is a reference to each declaration it merges.
//! What the standard library or a built-in declares has its references in
//! the workspace, and no declaration. Its HIGHLIGHTS
//! ([`Index::highlights`]) are its references in the one file, each a
//! write (a declaration or a `:=`) or a read.
//!
//! # Rename
//!
//! A RENAME of what is at a place ([`Index::prepare_rename`],
//! [`Index::rename`]) edits every occurrence of its symbol that is written
//! with its name, in every file of the workspace, and of every symbol filed
//! under the same nodes (each node a view's read merges), so that an
//! import's alias, `import x as y`, keeps its name, only `x` changing; a
//! qualified type, `lib.T`, has its name changed; and a definition's
//! segment renames its node everywhere, the references to it and the path
//! of its meta, `defmeta package x.y`, which is paired with it by path,
//! included. No file or directory is renamed, so a definition renamed no
//! longer mirrors its directory's name. It is refused, with why, for
//! nothing there (a keyword, a literal, what names nothing, a field read of
//! a record not tracked); for what the standard library or a built-in
//! declares; for a name a directory or a file gives (`sandbox` of `import
//! _lib.sandbox`), or `fileroot`'s; for an import's alias; for a name
//! written somewhere as a string, `"a-b" = 1`; and for a new name that is
//! not one identifier (a definition's segment: a segment as the parser
//! takes one). Then it is checked: the files with its edits made are
//! indexed again, and it is refused, naming the first problem, if that
//! finds an error this index did not (by message and place, the places
//! mapped back through the edits), or if any occurrence of any file
//! refers to something other than it did, its symbol's declarations
//! mapped back through the edits being elsewhere: a name captured by
//! another, or one that refers to nothing any more.
//!
//! # Symbols
//!
//! A file's SYMBOLS ([`Index::document_symbols`]) are its top-level
//! fields, each with the fields of the record block written as its value as
//! children, three levels deep at most; its top-level `let`s and `fn`s; its
//! `def`s, definitions and metas, named by their whole paths, `package
//! x.y`; its `type` items; and its imports, as one group. Each has its type
//! on one line, cut to 60 characters. The workspace's
//! ([`Index::workspace_symbols`]) are every file's but the imports, matched
//! by a subsequence of a query, ignoring case.
//!
//! # Completion
//!
//! What may be written at a place ([`Index::completion`]) is found from the
//! file's text as it is, lexed again, and the index of it: while it is
//! edited it is mostly broken, and the parser keeps what an editor leaves
//! half-typed for it, `a.` and `f(` (see the parser's [Errors and
//! recovery](crate::parser#errors-and-recovery)), so what is before the
//! place is checked and indexed. The tokens before the place, the word
//! being typed at it set aside (the client filters by it; a name typed up
//! to a `-`, `libcap-`, which the lexer takes for a `-` that is ambiguous,
//! is such a word too), say what the place is, and so what is offered, each
//! a [`Completion`] with its kind, its
//! type on one line cut to 60 characters, its doc comment (see
//! "Documentation"), and its name quoted if it is no identifier. What a
//! completion writes over is the name the place is in, by GCSL's rules for
//! names, a `-` between letters part of it ([`name_around`]): from its start
//! to the place, or to its end, as the client chooses:
//!
//! - In a comment or in a string's text, nothing. Nor where a name is
//!   declared (after `let`, `mut`, `fn`, `def`, `for`, at a function's or a
//!   definition's parameter, a `type` item's name), nor after what ends an
//!   expression (a literal, a closing bracket).
//! - After a `.`, the fields of what is before it: of a path through the
//!   file's view, `com.foo.`, the children of its node, of a module an
//!   import reaches, `lib.`, its members, of `fileroot.`, its four; else of
//!   the record type the check gave it (a local's, a field's, a call's),
//!   each documented when the record is written as a record block, the
//!   value of a local or a node. The access is found in the tree, a
//!   [`ExprKind::NoField`] or a [`ExprKind::Field`] whose name is being
//!   typed; where the tree has none, a path of names is followed by its
//!   names.
//! - In an `import`'s path, after `import` or `from`, the modules under the
//!   segments before the place, those before its last `/`: the directories
//!   and files of the directory they name, and at the start the standard
//!   library's modules, every one of them whether the scan loaded it or
//!   not, `builtins` and `fileroot` (only those in a lone file). The path
//!   so far must be one run of glued tokens, as the parser reads it (see
//!   its [Imports](crate::parser#imports)), the word half-typed directly
//!   after it. What each writes over is not the name around the place but
//!   the whole segment around it, dots and all
//!   ([`module_segment_around`], [`Completion::range`]), so that a
//!   directory named `net.busybox.busybox` replaces `net.bu`. Asked for
//!   because a `/` was typed ([`Index::completion_triggered`]), only this
//!   place offers anything: anywhere else a `/` is a division, after which
//!   no list should open unasked.
//! - After a def-type's name, `package dev.`, the children of the node of
//!   the path in the def-type's namespace: the definitions under it. The
//!   namespace's key is a reference's or a definition's of the def-type
//!   already in the tree, of the file or else of any file whose def-type is
//!   what the name is here.
//! - In a type, after a `:`, a return marker, `is`, `as`, `|`, `List<` or a
//!   `type` item's `=`: the types of the file's view, then the built-in
//!   types; after `lib.` there, the types of the module `lib` imports.
//! - Anywhere else, a name of an expression: everything in scope, each name
//!   once, the nearest, ordered by group ([`Completion::sort`]): where an
//!   argument of a call starts (after its `(` or a `,` of its argument
//!   list), first the named arguments, `name = `, of the callee's
//!   parameters no other argument fills (as signature help counts them;
//!   none for `len` and `show`); the locals
//!   (the `let`s, `fn`s and fields of each block around the place before
//!   the item it is in, the parameters of each function whose body it is
//!   in, the variable of each `for` whose body or guard it is in), rebuilt
//!   from the tree at the place, innermost and latest first, with the types
//!   the check recorded; then the file's own top-level `let`s before the
//!   place, `fn`s and fields; its directories' fields; what it imports; the
//!   standard library's, the prelude's names; and the keywords an
//!   expression or an item may start with. The field being declared, the
//!   top-level field named by one segment whose value the place is in, is
//!   not offered, as no block's item is before its own end; a `def` or a
//!   `fn`, which may call itself, is.
//!
//! # Inlay hints
//!
//! A file's INLAY HINTS ([`Index::inlay_hints`]) are the types it does not
//! write, each shown where it would be written, read from what its check
//! recorded of its declarations: `: Int` after the name of a `let` or a
//! `let mut` named by one segment and written without a type, and after
//! the variable of a `for`; and for a definition made with a def-type,
//! whose parameters are written by name alone and whose result is not
//! written, `: Int` after each parameter, and its result after its `)` as
//! GCSL writes one after its marker, `: Int`, ` ->> Int` or ` -> Int`,
//! as the def-type declares it. GCSL's other functions write their
//! parameters' and results' types, as must be, so a function expression has
//! none, and neither has a call's argument: arguments are often named
//! already. They are conservative: a field has none (a configuration is
//! mostly fields, `version = "1"`), nor has a `let` whose value spells its
//! type (a literal, a negated number among them; a list written out; a
//! record block; a function expression; an ascription or an `as`), nor a
//! type the check did not find or found erroneous anywhere in it, nor one
//! longer than [`MAX_HINT`] characters (cut short, a definition's record
//! result would say nothing useful, and sit on most definitions). A type
//! is shown as a source writes it, without the either shape (`T |
//! Future<T>`) or a provisional layer that stands for a `T` written, on one
//! line. A file not indexed has none, but one whose braces do not match
//! has those of the workspace indexed again with them balanced (see
//! "Unbalanced files").
//!
//! # Signature help
//!
//! The signature of the call a place is in the argument list or trailing
//! block of ([`Index::signature`]) is found from the brackets open before
//! it, innermost first, in the file's text lexed again: the first `(` that
//! is a call's, a [`ExprKind::Call`] or an [`ExprKind::Unclosed`] whose
//! callee ends just before it (one that only groups is passed over), or the
//! `{` of a call's trailing block; a block's `{` or a string's
//! interpolation ends the search. The callee's type is its declaration's (a
//! node's cell's, a `def`'s or a local function's), else what the check
//! gave the callee; `len` and `show` have their generic signatures. The
//! [`Signature`] is the callee as written, a `def`'s with `def`, each
//! parameter named as its source names it and typed as written (a
//! parameter that accepts a future too shows the type it is declared
//! with), with its default as written, `mid: String = "é"`, where the
//! source is known (a `def`'s, a local function's), the parameter's offsets
//! covering it, with its doc comment. Its active parameter is the one a named
//! argument names, `name = ...`, when the place is in one; else the next one
//! not yet filled, from the left, by a positional argument before the
//! place, nor anywhere by a named one, nor, the last, by a trailing block;
//! in a trailing block, the last. The arguments are counted on the tokens,
//! so a list half-typed counts as it is written. A trailing block left
//! open, `pair("a", "b") {` while it is typed, leaves the file's braces
//! unmatched, so the file has nothing indexed: its signature is found as
//! any query's is in such a file (see "Unbalanced files").
//!
//! # Unbalanced files
//!
//! A file whose braces do not match is the normal state of one while a
//! block is typed, `r = { k = 1; j = k`. It has nothing indexed, so a query
//! at a place of it (a completion, a hover, a definition, references,
//! highlights, a rename's preparation, signature help, inlay hints) is
//! answered from the workspace indexed again with the file's braces
//! balanced VIRTUALLY, only for the query ([`Index::closing`],
//! [`Closing::index`], [`Closed`]): each `}` that closes nothing is taken
//! out, and the `}`s the file lacks are put in, so that the place stays in
//! the blocks open there. The `}`s of the blocks open at the place that no
//! `}` after it closes go at the end of the last token of the place's line
//! (after the name being typed, before any trailing comment; at the place
//! itself if no token of its line ends at or after it), and any others at
//! the end of the file, on a line of their own. If the file so balanced
//! still does not parse with its braces matching, or parses with no fewer
//! errors than the file as it is, every `}` goes at the end of the file
//! instead, if that parses with its braces matching and with fewer errors
//! than the first, or the first did not balance: at most two such texts
//! are parsed, and the workspace is indexed again once, with the one
//! chosen. A place before a `}` put in is the same in
//! both texts; what the query finds is mapped back to the text as it is,
//! a place after a `}` put in or taken out shifted, and what touches the
//! text put in dropped. A place of another file is renumbered, by path.
//!
//! The index made again answers a query at any place that no `}` put in
//! is before, so that a burst of queries at one place, a completion, then
//! signature help, then a hover, indexes the workspace again once; and it
//! answers inlay hints whatever it closed, a range asked for first being
//! balanced at its end ([`ClosedCache`]). A document's symbols and code
//! actions are not answered so, and neither are diagnostics, which are
//! the check's of the text as it is: its syntax error. A RENAME in such a
//! file is refused, "the file has an unclosed `{`" (or "a `}` that closes
//! nothing"), as its check needs the text as it is; its preparation is
//! answered.

mod closing;
mod complete;
mod docs;
mod inlay;
mod rename;
mod signature;
mod symbols;

use std::cell::OnceCell;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

pub use closing::{Closed, ClosedCache, Closing};
pub use complete::{Completion, CompletionKind, module_segment_around, name_around};
pub use inlay::{InlayHint, MAX_HINT};
pub use rename::{Edit, Prepared};
pub use signature::Signature;
pub use symbols::{DocumentSymbol, FoundSymbol, SymbolKind};

use crate::ast::{
    Ast, ExprId, ExprKind, ForHead, ImportForm, ImportMember, ItemId, ItemKind, PathRange,
    PathSegment, TypeExprKind, TypeLink,
};
use crate::check::{BUILTIN_TYPE_NAMES, BindingKind, Builtin, Constant, Fileroot, Record, RecordedDecl};
use crate::diagnostic::{Diagnostic, Severity};
use crate::lexer::Comments;
use crate::mentions::Resolve;
use crate::names::{self, Name, Names};
use crate::source::{SourceFile, Span};
use crate::strings::FieldName;
use crate::types::{TyId, TyKind, TypeTable, write_constant};
use crate::workspace::{
    File, FileScope, Modules, Recorded, STANDARD_DIR, Scan, Target, module, target_roots,
};

/// The most lines a hover shows of what it shows: more are cut short, the
/// last line shown being `…`.
pub const MAX_LINES: usize = 20;

/// The widest line, in columns, a record type or a signature is written
/// on: a wider one is laid out a field or a parameter per line, indented
/// by 2. Hover panels are often 80 columns wide.
const WIDTH: usize = 80;

/// The longest line a hover shows: a longer one is cut short with `…`.
const MAX_LINE: usize = 200;

/// How deep the type of a directory, file or shared node read whole is
/// assembled for a hover before it gives up.
const MAX_NESTING: usize = 64;

/// What [`crate::workspace::index_with_standard`] found: the diagnostics a
/// check of the workspace reports, and the index.
#[derive(Debug)]
pub struct Indexed {
    /// The standard files the scan loaded, its first inputs, as
    /// [`crate::workspace::check_with_standard`] gives them, whether the
    /// check succeeded or failed.
    pub standard: Vec<SourceFile>,
    /// What the check found: its warnings if it succeeded, else every
    /// diagnostic, as [`crate::workspace::check_with_standard`] reports
    /// them.
    pub diagnostics: Vec<Diagnostic>,
    /// Whether the check succeeded.
    pub succeeded: bool,
    /// The index: always one of a workspace or a lone file, whose scan
    /// recovers from its errors (see [`crate::workspace`], "Scan errors").
    pub index: Option<Index>,
}

/// What a name refers to (see the module documentation, "Occurrences").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    /// A binding declared in a file: a variable, a parameter, a `for`
    /// variable, a local function or a field of a record block, by its
    /// file's input number and the span of its name there.
    Local {
        /// The file.
        file: u32,
        /// Where its name is written.
        name: Span,
    },
    /// A node of a directory's merged tree, a cell of the workspace, by its
    /// index in [`Scan::nodes`].
    Node(u32),
    /// A node of a file's view that several of its sources share, by its
    /// index in [`Scan::views`].
    View(u32),
    /// A directory, a file, a file's part of a shared node, or the
    /// `fileroot` module or one of its members, read through an import.
    Module(Target),
    /// A declared type, by its index in [`Scan::types`].
    Type(u32),
    /// A built-in type, `Int`, or type constructor, `List`, by its name.
    BuiltinType(Name),
}

/// A span written in a file that refers to something (see the module
/// documentation, "Occurrences").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    /// Where it is written: a name, a segment of a path, a type's name.
    pub span: Span,
    /// What it refers to, if that has an identity: `None` for a field read
    /// of a record whose origin is not tracked.
    pub symbol: Option<Symbol>,
    /// Whether it is where its symbol is declared.
    pub declaration: bool,
    /// Whether it writes its symbol: a declaration, or the variable a `:=`
    /// rebinds.
    pub write: bool,
    /// Where a definition request at it goes, if not to its symbol's
    /// declarations: a definition's path goes to its def-type.
    pub jump: Option<Symbol>,
    /// The name written there, for a name or a path's segment.
    name: Option<Name>,
    /// The type the check gave what is written there; `Error` if it found
    /// none, or found it erroneous.
    ty: TyId,
    /// The `import` item a name of the view was reached through, if it was.
    via: Option<ItemId>,
}

/// A place of a file: its input number and a span in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    /// The file's input number.
    pub file: u32,
    /// The span.
    pub span: Span,
}

/// What [`Index::hover`] answers: Markdown, and the span it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// The text, Markdown.
    pub text: String,
    /// The span of the occurrence hovered.
    pub span: Span,
}

/// What a file's index holds.
#[derive(Debug, Default)]
struct FileIndex {
    /// Whether the file was checked, and so indexed.
    checked: bool,
    /// Its occurrences, by start, none overlapping another.
    occurrences: Vec<Occurrence>,
    /// What each of its locals is, by the span of its name.
    locals: HashMap<Span, Local>,
    /// The type of each expression, as its check left it.
    types: Vec<TyId>,
    /// The type each of its `type` items stands for, as its check found it.
    declared: Vec<(ItemId, TyId)>,
}

/// A binding a file declares, as a hover shows it.
#[derive(Clone, Copy, Debug)]
struct Local {
    /// Its name.
    name: Name,
    /// Its type as declared.
    ty: TyId,
    /// What it is.
    what: LocalKind,
    /// The item that declares it, if an item does.
    item: Option<ItemId>,
}

/// What a [`Local`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalKind {
    /// A variable, declared by a `let` or a `:=`.
    Variable,
    /// A mutable variable, declared by a `let mut`.
    Mutable,
    /// A function's parameter.
    Parameter,
    /// A `for`'s variable.
    For,
    /// A field of a record block.
    Field,
    /// A function, `fn NAME` or `let NAME = fn ...`: its function
    /// expression.
    Function(ExprId),
}

/// An index of a checked workspace (see the module documentation).
#[derive(Debug)]
pub struct Index {
    /// The scan, its names in [`Scan::names`].
    scan: Scan,
    /// The table the types are interned into.
    types: TypeTable,
    /// The type of each cell, as the check of the file computing it found
    /// it, failed or not.
    cells: Vec<TyId>,
    /// Each file's index, by input number.
    files: Vec<FileIndex>,
    /// Each file's source, by input number, if it was given.
    sources: Vec<Option<SourceFile>>,
    /// The errors the check reported, which a rename must not add to.
    errors: Vec<Diagnostic>,
    /// The occurrences of each symbol, by key, in files of the workspace:
    /// the link table inverted, made on first use.
    table: OnceCell<HashMap<Key, Vec<(u32, u32)>>>,
    /// Each file's comments, by input number, found on first use.
    comments: Vec<OnceCell<Comments>>,
    /// Every file's symbols, for [`Index::workspace_symbols`], made on
    /// first use.
    flat: OnceCell<Vec<symbols::Flat>>,
}

/// What occurrences are filed under in the link table: a symbol, a node of
/// a view by each node it merges (see [`Index::keys`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    /// A local.
    Local(u32, Span),
    /// A merged node.
    Node(u32),
    /// A module that is no node: a directory, a file, `fileroot`.
    Module(Target),
    /// A declared type.
    Type(u32),
    /// A built-in type.
    BuiltinType(Name),
}

/// A place a symbol is written in a file, as
/// [`Index::highlights`] finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    /// Where.
    pub span: Span,
    /// Whether it writes the symbol: a declaration, or a `:=`.
    pub write: bool,
}

impl Index {
    /// The index of `scan`, its types interned into `types`, from what its
    /// files' checks `recorded`; `sources` are its inputs' sources, by
    /// name, and `diagnostics` what the check reported.
    pub(crate) fn new(
        scan: Scan,
        types: TypeTable,
        recorded: Recorded,
        (sources, diagnostics): (Vec<SourceFile>, &[Diagnostic]),
    ) -> Self {
        // By name; a lone file, which the scan names by its file name, by
        // its place among them.
        let mut by_place: Vec<Option<SourceFile>> = sources.into_iter().map(Some).collect();
        let aligned = by_place.len() == scan.files.len();
        let mut by_name: HashMap<String, usize> = (0..)
            .zip(&by_place)
            .filter_map(|(at, source)| Some((source.as_ref()?.name().to_owned(), at)))
            .collect();
        let sources = (0..)
            .zip(&scan.files)
            .map(|(at, file)| {
                let at = by_name
                    .remove(&file.path)
                    .or_else(|| aligned.then_some(at))?;
                by_place.get_mut(at)?.take()
            })
            .collect();
        let errors = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .cloned()
            .collect();
        let mut index = Self::of(scan, types, recorded);
        index.sources = sources;
        index.errors = errors;
        index
    }

    /// The text of the file of input number `file`: empty if it has none.
    fn text(&self, file: u32) -> &str {
        self.sources
            .get(file as usize)
            .and_then(Option::as_ref)
            .map_or("", SourceFile::text)
    }

    /// [`Index::new`], without the sources and diagnostics.
    fn of(scan: Scan, types: TypeTable, recorded: Recorded) -> Self {
        let Recorded { files, cells } = recorded;
        let mut indexed = Vec::with_capacity(scan.files.len());
        let dirs: HashMap<String, u32> = (0u32..)
            .zip(&scan.dirs)
            .map(|(index, dir)| (dir.path.clone(), index))
            .collect();
        let modules = Modules::of(&scan, &dirs);
        let mut records = files.into_iter();
        for (index, file) in (0u32..).zip(&scan.files) {
            let file_index = match records.next().flatten() {
                Some(record) => {
                    Builder::new((&scan, &types, &modules), (index, file), *record).build()
                }
                None => FileIndex::default(),
            };
            indexed.push(file_index);
        }
        Self {
            comments: (0..scan.files.len()).map(|_| OnceCell::new()).collect(),
            sources: Vec::new(),
            scan,
            types,
            cells,
            files: indexed,
            errors: Vec::new(),
            table: OnceCell::new(),
            flat: OnceCell::new(),
        }
    }

    /// The number of standard files, the first inputs: none of their
    /// places is a definition's, having no file on disk.
    #[must_use]
    pub fn standard(&self) -> u32 {
        self.scan.standard()
    }

    /// The input number of the file at `path`, relative to the root (a lone
    /// file's, the path it was given as).
    #[must_use]
    pub fn file(&self, path: &str) -> Option<u32> {
        (0u32..)
            .zip(&self.scan.files)
            .find(|(_, file)| file.path == path)
            .map(|(index, _)| index)
    }

    /// The path of the file of input number `file`.
    #[must_use]
    pub fn path(&self, file: u32) -> Option<&str> {
        self.scan.files.get(file as usize).map(|f| f.path.as_str())
    }

    /// Whether the file of input number `file` was checked, and so is
    /// indexed: not one whose braces do not match.
    #[must_use]
    pub fn checked(&self, file: u32) -> bool {
        self.files.get(file as usize).is_some_and(|f| f.checked)
    }

    /// The occurrences of the file of input number `file`, in source order.
    #[must_use]
    pub fn occurrences(&self, file: u32) -> &[Occurrence] {
        self.files
            .get(file as usize)
            .map_or(&[], |f| f.occurrences.as_slice())
    }

    /// The occurrence of the file of input number `file` at the byte
    /// `offset`, if one covers it.
    #[must_use]
    pub fn at(&self, file: u32, offset: u32) -> Option<&Occurrence> {
        let occurrences = self.occurrences(file);
        let after = occurrences.partition_point(|o| o.span.start <= offset);
        let found = occurrences.get(after.checked_sub(1)?)?;
        (offset < found.span.end).then_some(found)
    }

    /// Where what is at byte `offset` of the file of input number `file` is
    /// declared: none for nothing there, for a field read of a record whose
    /// origin is not tracked, and for what the standard library or a
    /// built-in declares (see the module documentation, "Declarations").
    #[must_use]
    pub fn definition(&self, file: u32, offset: u32) -> Vec<Location> {
        let Some(found) = self.at(file, offset) else {
            return Vec::new();
        };
        match found.jump.or(found.symbol) {
            Some(symbol) => self.declarations(symbol),
            None => Vec::new(),
        }
    }

    /// Where `symbol` is declared, in files of the workspace (see the module
    /// documentation, "Declarations"), in input then source order.
    #[must_use]
    pub fn declarations(&self, symbol: Symbol) -> Vec<Location> {
        let mut found = match symbol {
            Symbol::Local { file, name } => vec![Location { file, span: name }],
            Symbol::Node(node) => self.node_sites(node),
            Symbol::View(view) => self.view_sites(view),
            Symbol::Module(target) => self.module_sites(target),
            Symbol::Type(decl) => self
                .scan
                .types
                .get(decl as usize)
                .map(|declared| Location {
                    file: declared.file,
                    span: declared.span,
                })
                .into_iter()
                .collect(),
            Symbol::BuiltinType(_) => Vec::new(),
        };
        let standard = self.scan.standard();
        found.retain(|location| location.file >= standard);
        found.sort_by_key(|location| (location.file, location.span.start, location.span.end));
        found.dedup();
        found
    }

    /// Where what is at byte `offset` of the file of input number `file` is
    /// written in the files of the workspace, its declarations among them
    /// if `declarations` (see the module documentation, "References"), in
    /// input then source order: none for nothing there and for a field read
    /// of a record whose origin is not tracked.
    #[must_use]
    pub fn references(&self, file: u32, offset: u32, declarations: bool) -> Vec<Location> {
        let Some(symbol) = self.at(file, offset).and_then(|found| found.symbol) else {
            return Vec::new();
        };
        let mut found: Vec<Location> = self
            .occurrences_of(&self.keys(symbol))
            .into_iter()
            .filter(|&(_, occurrence)| declarations || !occurrence.declaration)
            .map(|(file, occurrence)| Location {
                file,
                span: occurrence.span,
            })
            .collect();
        found.sort_by_key(|location| (location.file, location.span.start, location.span.end));
        found.dedup();
        found
    }

    /// Where what is at byte `offset` of the file of input number `file` is
    /// written in that file, each place with whether it writes it (see the
    /// module documentation, "References"), in source order.
    #[must_use]
    pub fn highlights(&self, file: u32, offset: u32) -> Vec<Highlight> {
        let Some(symbol) = self.at(file, offset).and_then(|found| found.symbol) else {
            return Vec::new();
        };
        let keys = self.keys(symbol);
        let mut found: Vec<Highlight> = self
            .occurrences(file)
            .iter()
            .filter(|occurrence| {
                occurrence
                    .symbol
                    .is_some_and(|other| self.keys(other).iter().any(|key| keys.contains(key)))
            })
            .map(|occurrence| Highlight {
                span: occurrence.span,
                write: occurrence.write,
            })
            .collect();
        found.dedup();
        found
    }

    /// The keys the occurrences of `symbol` are filed under: a view's
    /// node's, each node it merges; a file's part of a node, the node.
    fn keys(&self, symbol: Symbol) -> Vec<Key> {
        let mut keys = Vec::new();
        match symbol {
            Symbol::Local { file, name } => keys.push(Key::Local(file, name)),
            Symbol::Node(node) => keys.push(Key::Node(node)),
            Symbol::View(view) => self.target_keys(Target::View(view), 0, &mut keys),
            Symbol::Module(target) => self.target_keys(target, 0, &mut keys),
            Symbol::Type(decl) => keys.push(Key::Type(decl)),
            Symbol::BuiltinType(name) => keys.push(Key::BuiltinType(name)),
        }
        let mut unique = Vec::with_capacity(keys.len());
        for key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        unique
    }

    /// Adds the keys of `target`, `depth` views in, to `keys`.
    fn target_keys(&self, target: Target, depth: usize, keys: &mut Vec<Key>) {
        match target {
            Target::Node(node) => keys.push(Key::Node(node)),
            Target::Part { file, node } => {
                let global = self
                    .scan
                    .files
                    .get(file as usize)
                    .and_then(|f| f.global.get(node as usize).copied());
                keys.push(global.map_or(Key::Module(target), Key::Node));
            }
            Target::View(view) => {
                let Some(entry) = self.scan.views.get(view as usize) else {
                    return;
                };
                if depth > MAX_NESTING {
                    return;
                }
                for part in &entry.parts {
                    self.target_keys(part.target, depth + 1, keys);
                }
            }
            Target::Dir(_) | Target::File(_) | Target::Fileroot(_) => {
                keys.push(Key::Module(target));
            }
        }
    }

    /// The link table: the occurrences of the files of the workspace, by
    /// each key of their symbols, each by its file and its index there.
    fn table(&self) -> &HashMap<Key, Vec<(u32, u32)>> {
        self.table.get_or_init(|| {
            let mut table: HashMap<Key, Vec<(u32, u32)>> = HashMap::new();
            for (file, index) in (0u32..).zip(&self.files).skip(self.standard() as usize) {
                for (at, occurrence) in (0u32..).zip(&index.occurrences) {
                    let Some(symbol) = occurrence.symbol else {
                        continue;
                    };
                    for key in self.keys(symbol) {
                        table.entry(key).or_default().push((file, at));
                    }
                }
            }
            table
        })
    }

    /// The occurrences filed under any of `keys`, each once, by file.
    fn occurrences_of(&self, keys: &[Key]) -> Vec<(u32, &Occurrence)> {
        let table = self.table();
        let mut places: Vec<(u32, u32)> = keys
            .iter()
            .filter_map(|key| table.get(key))
            .flatten()
            .copied()
            .collect();
        places.sort_unstable();
        places.dedup();
        places
            .into_iter()
            .filter_map(|(file, at)| {
                let occurrence = self
                    .files
                    .get(file as usize)?
                    .occurrences
                    .get(at as usize)?;
                Some((file, occurrence))
            })
            .collect()
    }

    /// What is at byte `offset` of the file of input number `file`, as
    /// Markdown (see the module documentation, "Hovers"): `None` for
    /// nothing there, and for what has no type the check found.
    pub fn hover(&mut self, file: u32, offset: u32) -> Option<Hover> {
        let found = *self.at(file, offset)?;
        let (code, origin) = self.describe(file, &found)?;
        let mut text = format!("```gcsl\n{}\n```", cut(&code));
        if let Some(doc) = found
            .symbol
            .and_then(|symbol| self.documentation_of(symbol))
        {
            text.push_str("\n\n");
            text.push_str(&doc);
        }
        if let Some(origin) = origin {
            text.push_str("\n\n");
            text.push_str(&origin);
        }
        Some(Hover {
            text,
            span: found.span,
        })
    }

    /// The code and the line of origin a hover of `found`, an occurrence of
    /// file `file`, shows.
    fn describe(&mut self, file: u32, found: &Occurrence) -> Option<(String, Option<String>)> {
        match found.symbol {
            Some(Symbol::Local { file: at, name }) => self.describe_local(at, name, found),
            Some(Symbol::Node(node)) => self.describe_node(file, node, found),
            Some(Symbol::View(view)) => {
                let path = self.view_path(view);
                let ty = self
                    .target_type(Target::View(view), 0)
                    .or_else(|| known(found.ty))?;
                let prefix = format!("{path}: ");
                Some((
                    format!("{prefix}{}", self.pretty(&prefix, ty)),
                    Some("field of several sources of this file's view".to_owned()),
                ))
            }
            Some(Symbol::Module(target)) => Some(self.describe_module(target, found)),
            Some(Symbol::Type(decl)) => self.describe_type(decl),
            Some(Symbol::BuiltinType(name)) => Some((
                self.scan.names.get(name).to_owned(),
                Some("built-in type".to_owned()),
            )),
            None => {
                let ty = known(found.ty)?;
                let name = self.scan.names.get(found.name?);
                let prefix = format!("{}: ", FieldName(name));
                Some((
                    format!("{prefix}{}", self.pretty(&prefix, ty)),
                    Some("field".to_owned()),
                ))
            }
        }
    }

    /// A hover of `found`, an occurrence of the local of file `file` whose
    /// name is at `name`.
    fn describe_local(
        &mut self,
        file: u32,
        name: Span,
        found: &Occurrence,
    ) -> Option<(String, Option<String>)> {
        let local = self
            .files
            .get(file as usize)
            .and_then(|f| f.locals.get(&name))
            .copied();
        let text = match (local, found.name) {
            (Some(local), _) => self.scan.names.get(local.name).to_owned(),
            (None, Some(written)) => self.scan.names.get(written).to_owned(),
            (None, None) => return None,
        };
        let Some(local) = local else {
            let ty = known(found.ty)?;
            let prefix = format!("{}: ", FieldName(&text));
            return Some((format!("{prefix}{}", self.pretty(&prefix, ty)), None));
        };
        if let LocalKind::Function(function) = local.what {
            // The function expression's type, its binding's at worst.
            let ty = self
                .files
                .get(file as usize)
                .and_then(|f| f.types.get(function.index()).copied())
                .and_then(known)
                .or_else(|| known(found.ty))
                .or_else(|| known(local.ty));
            if let Some(ty) = ty
                && let Some(code) = self.signature_of(file, function, ty, "fn", &text)
            {
                return Some((code, None));
            }
        }
        let ty = known(found.ty).or_else(|| known(local.ty))?;
        let (prefix, origin) = match local.what {
            LocalKind::Mutable => ("mut ", None),
            LocalKind::Parameter => ("", Some("parameter")),
            LocalKind::For => ("", Some("`for` variable")),
            LocalKind::Field => ("", Some("field")),
            LocalKind::Variable | LocalKind::Function(_) => ("", None),
        };
        let prefix = format!("{prefix}{}: ", FieldName(&text));
        Some((
            format!("{prefix}{}", self.pretty(&prefix, ty)),
            origin.map(str::to_owned),
        ))
    }

    /// A hover of `found`, an occurrence of the node `node` in file `file`.
    fn describe_node(
        &mut self,
        file: u32,
        node: u32,
        found: &Occurrence,
    ) -> Option<(String, Option<String>)> {
        let path = self.node_path(node);
        let via = found.via.and_then(|item| self.import_path(file, item));
        let origin = self.node_origin(node, via);
        let ty = self
            .cells
            .get(node as usize)
            .copied()
            .and_then(known)
            .or_else(|| known(found.ty));
        // A `def`'s leaf shows the function's signature.
        if let Some((owner, function)) = self.def_of(node) {
            let ty = ty.or_else(|| {
                let types = &self.files.get(owner as usize)?.types;
                types.get(function.index()).copied().and_then(known)
            });
            if let Some(ty) = ty
                && let Some(code) = self.signature_of(owner, function, ty, "def", &path)
            {
                return Some((code, origin));
            }
        }
        let ty = ty?;
        let prefix = format!("{path}: ");
        Some((format!("{prefix}{}", self.pretty(&prefix, ty)), origin))
    }

    /// A hover of `found`, an occurrence of the module `target`.
    fn describe_module(&mut self, target: Target, found: &Occurrence) -> (String, Option<String>) {
        let scan = &self.scan;
        let (path, origin) = match target {
            Target::Dir(dir) if dir == scan.builtins_dir => {
                ("builtins".to_owned(), "built-in module".to_owned())
            }
            Target::Dir(dir) => {
                let path = scan.dirs.get(dir as usize).map_or("", |d| d.path.as_str());
                (path.replace('/', "."), format!("directory `{path}/`"))
            }
            Target::File(file) => {
                let path = scan
                    .files
                    .get(file as usize)
                    .map_or("", |f| f.path.as_str());
                if let Some(stem) = standard_stem(path) {
                    (stem.to_owned(), format!("standard library `{stem}`"))
                } else {
                    let stem = path.strip_suffix(".gcsl").unwrap_or(path);
                    (stem.replace('/', "."), format!("file `{path}`"))
                }
            }
            Target::Part { file, node } => {
                let path = scan
                    .files
                    .get(file as usize)
                    .map_or_else(Vec::new, |f| f.mentions.tree.path(node));
                let shown = names::show_path(&scan.names, &path);
                let owner = scan
                    .files
                    .get(file as usize)
                    .map_or("", |f| f.path.as_str());
                (shown, format!("the part of it `{owner}` declares"))
            }
            Target::Fileroot(None) => (Fileroot::MODULE.to_owned(), "built-in module".to_owned()),
            Target::Fileroot(Some(member)) => (
                format!("{}.{}", Fileroot::MODULE, member.name()),
                "built-in".to_owned(),
            ),
            Target::Node(_) | Target::View(_) => (String::new(), String::new()),
        };
        let ty = match target {
            Target::Fileroot(None) => None,
            Target::Fileroot(Some(_)) => known(found.ty),
            _ => self.target_type(target, 0).or_else(|| known(found.ty)),
        };
        let code = match ty {
            Some(ty) => {
                let prefix = format!("{path}: ");
                format!("{prefix}{}", self.pretty(&prefix, ty))
            }
            None => path,
        };
        (code, Some(origin))
    }

    /// A hover of the declared type `decl`, an index in [`Scan::types`]:
    /// its declaration with what it stands for.
    fn describe_type(&self, decl: u32) -> Option<(String, Option<String>)> {
        let declared = *self.scan.types.get(decl as usize)?;
        let name = self.scan.names.get(declared.name);
        let expansion = self
            .files
            .get(declared.file as usize)
            .and_then(|f| f.declared.iter().find(|&&(item, _)| item == declared.item))
            .map(|&(_, ty)| ty)
            .and_then(known);
        let shown = match expansion {
            Some(ty) => self.pretty(&format!("type {name} = "), ty),
            None => self
                .scan
                .files
                .get(declared.file as usize)?
                .ast
                .show_type_with(declared.ty, &self.scan.names),
        };
        let path = self
            .scan
            .files
            .get(declared.file as usize)
            .map_or("", |f| f.path.as_str());
        let origin = match standard_stem(path) {
            Some(stem) => format!("standard library `{stem}`"),
            None => format!("type of `{path}`"),
        };
        Some((format!("type {name} = {shown}"), Some(origin)))
    }

    /// The signature of the function expression `function` of file `file`,
    /// of type `ty`, declared with `keyword` and named `name`: `fn name(a:
    /// Int): Int`, each parameter named as the source names it, laid out a
    /// parameter per line if it is too wide for one. `None` if `ty` is no
    /// function's type.
    fn signature_of(
        &self,
        file: u32,
        function: ExprId,
        ty: TyId,
        keyword: &str,
        name: &str,
    ) -> Option<String> {
        let table = &self.types;
        let sig = table.fn_sig(table.strip(ty))?;
        let ast = &self.scan.files.get(file as usize)?.ast;
        let written = match ast.get(function).kind {
            ExprKind::Fn(id) => ast.params(ast.fn_decl(id).params),
            _ => &[],
        };
        let params: Vec<String> = sig
            .params
            .iter()
            .enumerate()
            .map(|(at, &(named, ty))| {
                let param = written
                    .get(at)
                    .map(|param| param.name)
                    .or(named)
                    .map(|param| format!("{}: ", self.scan.names.get(param)))
                    .unwrap_or_default();
                format!("{param}{}", table.display(ty, &self.scan.names))
            })
            .collect();
        let tail = self.sig_tail(sig);
        let one = format!("{keyword} {name}({}{tail}", params.join(", "));
        if one.chars().count() <= WIDTH || params.is_empty() {
            return Some(one);
        }
        let mut code = format!("{keyword} {name}(\n");
        for param in &params {
            code.push_str("  ");
            code.push_str(param);
            code.push_str(",\n");
        }
        code.push_str(&tail);
        Some(code)
    }

    /// What a function type's signature ends with after its parameters:
    /// `): Int`, its marker and its return type, and its meta's.
    /// Formatting writes to a String and uses our infallible type displays,
    /// so the formatting results can be discarded.
    fn sig_tail(&self, sig: &crate::types::FnTy) -> String {
        let table = &self.types;
        let mut tail = sig.effect.params_end().to_owned();
        let _ = write!(tail, "{}", table.display(sig.ret, &self.scan.names));
        if let Some(meta) = sig.meta {
            let _ = write!(
                tail,
                " {} {}",
                meta.effect.meta_marker(),
                table.display(meta.ty, &self.scan.names)
            );
        }
        tail
    }

    /// The file and function expression of the `def` whose leaf is the node
    /// `node`, if it is one's.
    fn def_of(&self, node: u32) -> Option<(u32, ExprId)> {
        let owner = self.scan.nodes.get(node as usize)?.leaf?;
        let file = self.scan.files.get(owner as usize)?;
        let own = file.global.iter().position(|&g| g == node)?;
        let leaf = file.mentions.tree.nodes.get(own)?.leaf?;
        let function = file.mentions.functions.get(leaf.function? as usize)?;
        Some((owner, function.expr))
    }

    /// The path of the node `node`, as a diagnostic shows it: `a.b.c`, a
    /// definition's `package dev.peios.x`.
    fn node_path(&self, node: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.scan.nodes.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.scan.names, &path)
    }

    /// The path of the view node `view`, as [`Index::node_path`] shows one.
    fn view_path(&self, view: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(view);
        while let Some(index) = at
            && let Some(entry) = self.scan.views.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.scan.names, &path)
    }

    /// Where the node `node` comes from, as a hover says it, reached
    /// through the import of the path `via` if it was.
    fn node_origin(&self, node: u32, via: Option<String>) -> Option<String> {
        let scan = &self.scan;
        if (node as usize) < Builtin::ALL.len() {
            return Some("built-in".to_owned());
        }
        let entry = scan.nodes.get(node as usize)?;
        let mut root = node;
        while let Some(parent) = scan.nodes.get(root as usize).and_then(|n| n.parent) {
            root = parent;
        }
        let root_text = scan
            .nodes
            .get(root as usize)
            .map(|n| scan.names.get(n.name));
        if let Some(def_type) = root_text.and_then(names::key_def_type) {
            return Some(format!("definition of def-type `{def_type}`"));
        }
        if let Some(def_type) = root_text.and_then(names::key_meta) {
            return Some(format!("meta of a definition of def-type `{def_type}`"));
        }
        let standard = (0..scan.standard()).find_map(|file| {
            let f = scan.files.get(file as usize)?;
            (f.dir == entry.dir).then(|| standard_stem(&f.path))?
        });
        if let Some(stem) = standard {
            return Some(format!("standard library `{stem}`"));
        }
        if let Some(path) = via {
            return Some(format!("imported from `{path}`"));
        }
        let dir = scan.dirs.get(entry.dir as usize)?;
        if dir.path.is_empty() {
            Some("field of the root directory".to_owned())
        } else {
            Some(format!("field of directory `{}/`", dir.path))
        }
    }

    /// The path of the `import` item `item` of file `file`, as written:
    /// `lib/x`.
    fn import_path(&self, file: u32, item: ItemId) -> Option<String> {
        let ast = &self.scan.files.get(file as usize)?.ast;
        let &ItemKind::Import(id) = ast_item(ast, item)? else {
            return None;
        };
        let segments = ast.path(ast.import_decl(id).path);
        let shown: Vec<&str> = segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.scan.names.get(name))
            .collect();
        Some(shown.join("/"))
    }

    /// `segments`, names joined by `.`.
    fn show_segments(&self, segments: &[PathSegment]) -> String {
        segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.scan.names.get(name))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Where the node `node` is declared: each item at or under it, in each
    /// file with one, at its path's segment at the node's depth.
    fn node_sites(&self, node: u32) -> Vec<Location> {
        let Some(entry) = self.scan.nodes.get(node as usize) else {
            return Vec::new();
        };
        let mut depth = 0;
        let mut at = entry.parent;
        while let Some(parent) = at {
            depth += 1;
            at = self.scan.nodes.get(parent as usize).and_then(|n| n.parent);
        }
        let files = match entry.leaf {
            Some(owner) => vec![owner],
            None => entry.contributors.clone(),
        };
        let mut found = Vec::new();
        for owner in files {
            let Some(file) = self.scan.files.get(owner as usize) else {
                continue;
            };
            if let Some(own) = file.global.iter().position(|&g| g == node) {
                own_sites(
                    owner,
                    file,
                    u32::try_from(own).unwrap_or(u32::MAX),
                    depth,
                    &mut found,
                );
            }
        }
        // A conflicted node is declared by the definitions the scan dropped
        // as later definitions of it too, which a fix may want to keep.
        for &(owner, item) in self.scan.conflicting(node) {
            if let Some(file) = self.scan.files.get(owner as usize)
                && let Some(span) = item_segment(&file.ast, item, depth)
            {
                found.push(Location { file: owner, span });
            }
        }
        found
    }

    /// Where the view node `view` is declared: where each of its parts is.
    fn view_sites(&self, view: u32) -> Vec<Location> {
        let Some(entry) = self.scan.views.get(view as usize) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for part in &entry.parts {
            found.extend(self.module_sites(part.target));
        }
        found
    }

    /// Where the module, or node, `target` is declared: a node's or a
    /// part's items; a directory's first file's first declaration; a file's
    /// start.
    fn module_sites(&self, target: Target) -> Vec<Location> {
        match target {
            Target::Node(node) => self.node_sites(node),
            Target::View(view) => self.view_sites(view),
            Target::Part { file, node } => {
                let mut found = Vec::new();
                if let Some(entry) = self.scan.files.get(file as usize) {
                    let mut depth = 0;
                    let mut at = entry
                        .mentions
                        .tree
                        .nodes
                        .get(node as usize)
                        .and_then(|n| n.parent);
                    while let Some(parent) = at {
                        depth += 1;
                        at = entry
                            .mentions
                            .tree
                            .nodes
                            .get(parent as usize)
                            .and_then(|n| n.parent);
                    }
                    own_sites(file, entry, node, depth, &mut found);
                }
                found
            }
            Target::Dir(dir) => (0u32..)
                .zip(&self.scan.files)
                .filter(|(_, file)| file.dir == dir)
                .find_map(|(index, file)| {
                    let ast = &file.ast;
                    ast.items(ast.root()).iter().find_map(|&item| {
                        let span = declared_name(ast, item)?;
                        Some(Location { file: index, span })
                    })
                })
                .into_iter()
                .collect(),
            Target::File(file) => vec![Location {
                file,
                span: Span::new(0, 0),
            }],
            Target::Fileroot(_) => Vec::new(),
        }
    }

    /// The type of `target` read whole, assembled from its children's
    /// cells, `depth` levels into the assembly: `None` if any is not known,
    /// or it is nested too deeply.
    fn target_type(&mut self, target: Target, depth: usize) -> Option<TyId> {
        if depth > MAX_NESTING {
            return None;
        }
        if let Target::Node(node) = target {
            return self.cells.get(node as usize).copied().and_then(known);
        }
        if let Target::File(file) = target
            && let Some(cell) = self.scan.value_cell(file)
        {
            return self.cells.get(cell as usize).copied().and_then(known);
        }
        let names = match target {
            Target::Dir(_) | Target::File(_) => target_roots(&self.scan, target),
            Target::View(view) => self.scan.view_names(view),
            Target::Part { .. } => self.scan.child_names(target),
            Target::Node(_) | Target::Fileroot(_) => return None,
        };
        let mut fields = Vec::with_capacity(names.len());
        for name in names {
            if names::is_namespace_key(self.scan.names.get(name)) {
                continue;
            }
            let child = self.scan.target_child(target, name)?;
            fields.push((name, self.target_type(child, depth + 1)?));
        }
        Some(self.types.record(fields))
    }

    /// `ty` as a hover shows it after `prefix` on its first line, `name: `
    /// (see the module documentation, "Hovers").
    fn pretty(&self, prefix: &str, ty: TyId) -> String {
        let mut shown = String::new();
        self.write_pretty(ty, (0, prefix.chars().count()), &mut shown);
        shown
    }

    /// `ty` on one line, as the printer diagnostics use shows it, a
    /// built-in function by its signature.
    fn one_line(&self, ty: TyId) -> String {
        match self.types.kind(ty) {
            TyKind::Builtin(builtin) => match builtin {
                Builtin::Len => "fn<T>(list: List<T>): Int",
                Builtin::Show => "fn<T>(value: T): String",
                _ => builtin.display(),
            }
            .to_owned(),
            _ => self.types.display(ty, &self.scan.names).to_string(),
        }
    }

    /// Writes `ty` as a hover shows it, its lines after the first indented
    /// by `indent`, its first line starting at `column`, to `out`: on one
    /// line if that is [`WIDTH`] columns at most, else, for a record, a
    /// field per line.
    /// Formatting results are discarded because the destination is a String and
    /// every display used here propagates only that destination's writes.
    fn write_pretty(&self, ty: TyId, (indent, column): (usize, usize), out: &mut String) {
        let table = &self.types;
        let names = &self.scan.names;
        let one = self.one_line(ty);
        let fields = table.record_fields(ty);
        match fields {
            Some(fields) if column + one.chars().count() > WIDTH && !fields.is_empty() => {
                let mut sorted: Vec<(&str, Name, TyId)> = fields
                    .iter()
                    .map(|&(name, field)| (names.get(name), name, field))
                    .collect();
                sorted.sort_unstable_by_key(|&(text, _, _)| text);
                out.push_str("{\n");
                for (text, name, field) in sorted {
                    let _ = write!(
                        out,
                        "{:width$}{}: ",
                        "",
                        FieldName(text),
                        width = indent + 2
                    );
                    let column = indent + 2 + FieldName(text).to_string().chars().count() + 2;
                    self.write_pretty(field, (indent + 2, column), out);
                    if let Some(constant) = table.field_default(ty, name) {
                        out.push_str(" = ");
                        let _ = write!(out, "{}", Shown(constant, names));
                    }
                    out.push_str(",\n");
                }
                // The rest-field after the named ones, as a type shows it.
                if let Some(rest) = table.record_rest(ty) {
                    let _ = write!(out, "{:width$}*: ", "", width = indent + 2);
                    self.write_pretty(rest, (indent + 2, indent + 5), out);
                    out.push_str(",\n");
                }
                let _ = write!(out, "{:width$}}}", "", width = indent);
            }
            None if column + one.chars().count() > WIDTH
                && !matches!(table.kind(ty), TyKind::Builtin(_)) =>
            {
                // A function type too wide: a parameter per line.
                match table.fn_sig(table.strip(ty)) {
                    Some(sig) if !sig.params.is_empty() => {
                        out.push_str("fn(\n");
                        for &(label, param) in &sig.params {
                            let label = label
                                .map(|name| format!("{}: ", names.get(name)))
                                .unwrap_or_default();
                            let shown = table.display(param, names);
                            let _ =
                                writeln!(out, "{:width$}{label}{shown},", "", width = indent + 2);
                        }
                        let _ = write!(out, "{:width$}{}", "", self.sig_tail(sig), width = indent);
                    }
                    _ => out.push_str(&one),
                }
            }
            _ => out.push_str(&one),
        }
    }
}

/// A field's default, displayed as it is written.
struct Shown<'a>(&'a Constant, &'a Names);

impl std::fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_constant(f, self.0, self.1)
    }
}

/// `ty`, if it is a type the check found: not `Error`.
fn known(ty: TyId) -> Option<TyId> {
    (ty != TyId::ERROR).then_some(ty)
}

/// `code` cut to [`MAX_LINES`] lines, each to [`MAX_LINE`] bytes, as a
/// hover shows it: the last line shown is `…` if any is left out.
fn cut(code: &str) -> String {
    let mut lines: Vec<String> = code
        .lines()
        // One lookahead line tells us whether to append the truncation marker.
        // Do not allocate or trim the rest of a large declaration just to discard it.
        .take(MAX_LINES + 1)
        .map(|line| {
            if line.len() <= MAX_LINE {
                return line.to_owned();
            }
            let mut end = MAX_LINE;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &line[..end])
        })
        .collect();
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES - 1);
        lines.push("…".to_owned());
    }
    lines.join("\n")
}

/// The stem of the standard file at `path`, `<stdlib>/STEM.gcsl`, if it is
/// one.
fn standard_stem(path: &str) -> Option<&str> {
    path.strip_prefix(STANDARD_DIR)?
        .strip_prefix('/')?
        .strip_suffix(".gcsl")
}

/// The item `item` of `ast`'s kind, if the tree has it.
fn ast_item(ast: &Ast, item: ItemId) -> Option<&ItemKind> {
    let number = u32::try_from(item.index()).ok()?;
    ast.item_at(number).map(|id| &ast.item(id).kind)
}

/// Where the name the top-level item `item` declares is written, if it
/// declares one: a field's or `def`'s first segment, a definition's path.
fn declared_name(ast: &Ast, item: ItemId) -> Option<Span> {
    match ast.item(item).kind {
        ItemKind::Field { path, .. } => ast.path(path).first().map(|segment| segment.span),
        ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. } => {
            path_span(ast.path(ast.def_type(def).path))
        }
        _ => None,
    }
}

/// The span of `segments`, first through last.
fn path_span(segments: &[PathSegment]) -> Option<Span> {
    Some(segments.first()?.span.join(segments.last()?.span))
}

/// Adds to `found` where the own node `own` of `file`, the file of input
/// number `index`, is declared: each leaf's item at or under it, at its
/// path's segment at `depth`.
fn own_sites(index: u32, file: &File, own: u32, depth: usize, found: &mut Vec<Location>) {
    let tree = &file.mentions.tree;
    let mut stack = vec![own];
    while let Some(at) = stack.pop() {
        let Some(node) = tree.nodes.get(at as usize) else {
            continue;
        };
        match node.leaf {
            Some(leaf) => {
                if let Some(span) = item_segment(&file.ast, leaf.item, depth) {
                    found.push(Location { file: index, span });
                }
            }
            None => stack.extend(&node.children),
        }
    }
}

/// The span of the segment at `depth` of the path the top-level item
/// `item` declares in its tree: a field's or `def`'s path's; a definition's
/// path's after its namespace's key (which is no segment written, so its
/// def-type stands for it); a meta's whole path, which is one name there.
fn item_segment(ast: &Ast, item: ItemId, depth: usize) -> Option<Span> {
    match ast.item(item).kind {
        ItemKind::Field { path, .. } => ast.path(path).get(depth).map(|segment| segment.span),
        ItemKind::DefTypeDef { def, .. } => {
            let used = ast.def_type(def);
            match depth.checked_sub(1) {
                Some(at) => ast.path(used.path).get(at).map(|segment| segment.span),
                None => Some(ast.get(used.dt).span),
            }
        }
        ItemKind::DefMeta { def, .. } => path_span(ast.path(ast.def_type(def).path)),
        _ => Some(ast.item(item).span),
    }
}

/// The span of the member `name` of the field access `access`, `.name`:
/// the name as written, quoted if it is no identifier.
fn member_span(ast: &Ast, access: ExprId, name: &str) -> Span {
    let expr = ast.get(access);
    let Some(member) = expr.member_span(ast) else {
        return expr.span;
    };
    let written = u32::try_from(FieldName(name).to_string().len()).unwrap_or(u32::MAX);
    let start = member.end.saturating_sub(written).max(member.start);
    Span::new(start, member.end)
}

/// `id`, without the parentheses around it.
fn unparen(ast: &Ast, mut id: ExprId) -> ExprId {
    while let ExprKind::Paren(inner) = ast.get(id).kind {
        id = inner;
    }
    id
}

/// What a name a check read from a binding of its file is read from, once
/// its declaration is found.
#[derive(Clone, Copy, Debug)]
enum Bound {
    /// A local, by its name's span.
    Local(Span),
    /// A top-level field or `def` of the file, which is a node of its view.
    View,
    /// The narrowed copy of the variable the `is` whose operand is this
    /// expression tests.
    Narrowed(ExprId),
}

/// Where the declarations of a file's bindings are, by the spans the check
/// records them by ([`RecordedDecl::declared`]).
#[derive(Debug, Default)]
struct Sites {
    /// The items that declare bindings (`let`, field, `:=`, spread), by
    /// span.
    items: HashMap<Span, ItemId>,
    /// The parameters, by span, with where each one's name is.
    params: HashMap<Span, Span>,
    /// The spans of the `for`s' variables' names.
    fors: HashSet<Span>,
    /// The function expressions, by span.
    fns: HashMap<Span, ExprId>,
    /// The `is` expressions, by span.
    tests: HashMap<Span, ExprId>,
    /// The item each function expression is the value of, if one is.
    holders: HashMap<ExprId, ItemId>,
    /// The items of the file's outermost block.
    roots: HashSet<ItemId>,
}

/// Builds the index of one file.
struct Builder<'s> {
    /// The scan.
    scan: &'s Scan,
    /// The table the types are interned into.
    table: &'s TypeTable,
    /// Its modules, for imports' paths.
    modules: &'s Modules<'s>,
    /// The file's input number.
    index: u32,
    /// The file.
    file: &'s File,
    /// Its tree.
    ast: &'s Ast,
    /// Its view.
    scope: FileScope<'s>,
    /// The type of each expression, as its check left it.
    types: Vec<TyId>,
    /// The type each `type` item stands for.
    declared: Vec<(ItemId, TyId)>,
    /// The bindings its check declared, by where and name.
    decls: HashMap<(Span, Name), RecordedDecl>,
    /// The bindings in declaration order, each once.
    order: Vec<(Span, Name)>,
    /// Where each name its check read from a binding was declared.
    uses: HashMap<ExprId, Span>,
    /// Where its bindings' declarations are.
    sites: Sites,
    /// The field access directly on each expression that has one.
    parents: HashMap<ExprId, ExprId>,
    /// The field accesses an occurrence is made of already.
    covered: HashSet<ExprId>,
    /// The symbol of each name and field access that has one.
    symbols: HashMap<ExprId, Symbol>,
    /// The occurrences found.
    occurrences: Vec<Occurrence>,
    /// The locals found.
    locals: HashMap<Span, Local>,
    /// Each `:=` that declared a variable in place of another, by where
    /// each was declared, as the check recorded them.
    rebinds: Vec<(Span, Span)>,
    /// The local each variable a `:=` declared in place of another stands
    /// for, by where their names are: the variable it replaces.
    replaced: HashMap<Span, Span>,
    /// Where each `:=`'s variable is written.
    writes: HashSet<Span>,
}

impl<'s> Builder<'s> {
    /// The builder of the index of `file`, input number `index` of `scan`,
    /// whose types are interned into `table` and whose modules are
    /// `modules`, from what its check recorded.
    fn new(
        (scan, table, modules): (&'s Scan, &'s TypeTable, &'s Modules<'s>),
        (index, file): (u32, &'s File),
        record: Record,
    ) -> Self {
        let mut decls = HashMap::new();
        let mut order = Vec::new();
        for decl in record.decls {
            let key = (decl.declared, decl.name);
            match decls.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(decl);
                    order.push(key);
                }
                // A named function item and its function expression start
                // at one place: its name for itself, declared at the
                // function, is not what the item declares. A later
                // declaration of the same is as it last was.
                Entry::Occupied(mut entry) => {
                    if decl.kind != BindingKind::Callee || entry.get().kind == BindingKind::Callee {
                        entry.insert(decl);
                    }
                }
            }
        }
        Self {
            scan,
            table,
            modules,
            index,
            file,
            ast: &file.ast,
            scope: FileScope::new(scan, file),
            types: record.types,
            declared: record.declared_types,
            decls,
            order,
            uses: record.uses.into_iter().collect(),
            sites: Sites::default(),
            parents: HashMap::new(),
            covered: HashSet::new(),
            symbols: HashMap::new(),
            occurrences: Vec::new(),
            locals: HashMap::new(),
            rebinds: record.rebinds,
            replaced: HashMap::new(),
            writes: HashSet::new(),
        }
    }

    /// Finds the variable each `:=` stands for, and where each `:=`'s is
    /// written.
    fn link_rebinds(&mut self) {
        let ast = self.ast;
        for id in ast.item_ids() {
            if let ItemKind::Rebind { path, .. } = ast.item(id).kind
                && let Some(first) = ast.path(path).first()
            {
                self.writes.insert(first.span);
            }
        }
        for at in 0..self.rebinds.len() {
            let Some(&(declared, replaced)) = self.rebinds.get(at) else {
                break;
            };
            let Some(&item) = self.sites.items.get(&declared) else {
                continue;
            };
            let ItemKind::Rebind { path, .. } = ast.item(item).kind else {
                continue;
            };
            let Some((first, name)) = ast
                .path(path)
                .first()
                .and_then(|first| Some((first.span, first.name()?)))
            else {
                continue;
            };
            let Some(Bound::Local(old)) = self.bound(replaced, name) else {
                continue;
            };
            if self.decls.contains_key(&(declared, name)) {
                // A variable's: the binding it declares is the old one.
                if old != first {
                    self.replaced.insert(first, old);
                }
            } else {
                // A mutable variable's: the binding is written there.
                let ty = self
                    .decls
                    .get(&(replaced, name))
                    .map_or(TyId::ERROR, |decl| decl.ty);
                let symbol = self.local(old);
                self.add(first, Some(symbol), Some(name), ty);
            }
        }
    }

    /// The symbol of the local whose name is at `at`: the variable it
    /// replaces, if a `:=` declared it in place of another.
    fn local(&self, mut at: Span) -> Symbol {
        for _ in 0..MAX_NESTING {
            match self.replaced.get(&at) {
                Some(&before) => at = before,
                None => break,
            }
        }
        Symbol::Local {
            file: self.index,
            name: at,
        }
    }

    /// The type of expression `id`, as the check left it.
    fn ty(&self, id: ExprId) -> TyId {
        self.types.get(id.index()).copied().unwrap_or(TyId::ERROR)
    }

    /// The text of `name`.
    fn text(&self, name: Name) -> &'s str {
        self.scan.names.get(name)
    }

    /// Records an occurrence.
    fn add(&mut self, span: Span, symbol: Option<Symbol>, name: Option<Name>, ty: TyId) {
        if span.end <= span.start {
            return;
        }
        self.occurrences.push(Occurrence {
            span,
            symbol,
            declaration: false,
            write: false,
            jump: None,
            name,
            ty,
            via: None,
        });
    }

    /// Records an occurrence that is its symbol's declaration.
    fn declare(&mut self, span: Span, symbol: Option<Symbol>, name: Option<Name>, ty: TyId) {
        self.add(span, symbol, name, ty);
        if let Some(last) = self.occurrences.last_mut()
            && last.span == span
        {
            last.declaration = true;
        }
    }

    /// The index of the file.
    fn build(mut self) -> FileIndex {
        self.find_sites();
        self.link_rebinds();
        self.declarations();
        self.items();
        self.names();
        self.def_types();
        self.fields();
        self.type_names();
        self.imports();
        let mut occurrences = self.occurrences;
        occurrences.sort_by_key(|o| (o.span.start, o.span.end));
        occurrences.dedup_by(|later, earlier| later.span.start < earlier.span.end);
        for occurrence in &mut occurrences {
            occurrence.write = occurrence.declaration || self.writes.contains(&occurrence.span);
        }
        FileIndex {
            checked: true,
            occurrences,
            locals: self.locals,
            types: self.types,
            declared: self.declared,
        }
    }

    /// Finds where the file's bindings can be declared, and the field
    /// access on each expression.
    fn find_sites(&mut self) {
        let ast = self.ast;
        for id in ast.expr_ids() {
            let expr = ast.get(id);
            match expr.kind {
                ExprKind::Fn(function) => {
                    self.sites.fns.insert(expr.span, id);
                    for param in ast.params(ast.fn_decl(function).params) {
                        let length = u32::try_from(self.text(param.name).len()).unwrap_or(0);
                        let name = Span::new(
                            param.span.start,
                            param.span.start.saturating_add(length).min(param.span.end),
                        );
                        self.sites.params.insert(param.span, name);
                    }
                }
                ExprKind::For(id) => {
                    if let ForHead::Each { name_span, .. } = ast.for_decl(id).head {
                        self.sites.fors.insert(name_span);
                    }
                }
                ExprKind::Is { .. } => {
                    self.sites.tests.insert(expr.span, id);
                }
                ExprKind::Field { expr: record, .. } => {
                    self.parents.insert(record, id);
                }
                _ => {}
            }
        }
        for id in ast.item_ids() {
            let item = ast.item(id);
            match item.kind {
                ItemKind::Let { value, .. }
                | ItemKind::Field { value, .. }
                | ItemKind::Rebind { value, .. }
                | ItemKind::Spread(value) => {
                    self.sites.items.insert(item.span, id);
                    self.sites.holders.insert(unparen(ast, value), id);
                }
                ItemKind::DefTypeDef { value, .. } | ItemKind::DefMeta { value, .. } => {
                    self.sites.holders.insert(value, id);
                }
                _ => {}
            }
        }
        self.sites.roots = ast.items(ast.root()).iter().copied().collect();
    }

    /// What the binding declared at `declared` named `name` is read from.
    fn bound(&self, declared: Span, name: Name) -> Option<Bound> {
        let decl = self.decls.get(&(declared, name))?;
        match decl.kind {
            BindingKind::Callee => {
                let function = self.sites.fns.get(&declared)?;
                let item = *self.sites.holders.get(function)?;
                self.item_bound(item)
            }
            BindingKind::Narrowed => {
                let test = *self.sites.tests.get(&declared)?;
                let ExprKind::Is { expr, .. } = self.ast.get(test).kind else {
                    return None;
                };
                Some(Bound::Narrowed(unparen(self.ast, expr)))
            }
            BindingKind::Variable
            | BindingKind::Mutable
            | BindingKind::Field
            | BindingKind::Header => {
                if let Some(&name) = self.sites.params.get(&declared) {
                    return Some(Bound::Local(name));
                }
                if self.sites.fors.contains(&declared) {
                    return Some(Bound::Local(declared));
                }
                let item = *self.sites.items.get(&declared)?;
                self.item_bound(item)
            }
        }
    }

    /// What a binding the item `item` declares is read from.
    fn item_bound(&self, item: ItemId) -> Option<Bound> {
        let ast = self.ast;
        match ast.item(item).kind {
            ItemKind::Field { .. } | ItemKind::DefTypeDef { .. }
                if self.sites.roots.contains(&item) =>
            {
                Some(Bound::View)
            }
            ItemKind::Let { path, .. }
            | ItemKind::Field { path, .. }
            | ItemKind::Rebind { path, .. } => ast
                .path(path)
                .first()
                .map(|segment| Bound::Local(segment.span)),
            ItemKind::Spread(value) => Some(Bound::Local(ast.get(value).span)),
            _ => None,
        }
    }

    /// The occurrences of the bindings' declarations, and what each local
    /// is.
    fn declarations(&mut self) {
        for at in 0..self.order.len() {
            let Some(&key) = self.order.get(at) else {
                break;
            };
            let Some(&decl) = self.decls.get(&key) else {
                continue;
            };
            if matches!(decl.kind, BindingKind::Callee | BindingKind::Narrowed) {
                continue;
            }
            let Some(Bound::Local(name)) = self.bound(decl.declared, decl.name) else {
                continue;
            };
            let item = self.sites.items.get(&decl.declared).copied();
            let what = self.local_kind(&decl, item);
            // A spread's soft fields are declared at its operand, which is
            // an expression of its own.
            if item.is_some_and(|item| matches!(self.ast.item(item).kind, ItemKind::Spread(_))) {
                continue;
            }
            // A `:=`'s variable is the one it replaces, written there.
            let symbol = self.local(name);
            if symbol
                == (Symbol::Local {
                    file: self.index,
                    name,
                })
            {
                self.declare(name, Some(symbol), Some(decl.name), decl.ty);
            } else {
                self.add(name, Some(symbol), Some(decl.name), decl.ty);
            }
            self.locals.insert(
                name,
                Local {
                    name: decl.name,
                    ty: decl.ty,
                    what,
                    item,
                },
            );
        }
    }

    /// What the local `decl`, declared by `item` if an item declares it, is.
    fn local_kind(&self, decl: &RecordedDecl, item: Option<ItemId>) -> LocalKind {
        if self.sites.params.contains_key(&decl.declared) {
            return LocalKind::Parameter;
        }
        if self.sites.fors.contains(&decl.declared) {
            return LocalKind::For;
        }
        if decl.kind == BindingKind::Mutable {
            return LocalKind::Mutable;
        }
        let Some(item) = item else {
            return LocalKind::Variable;
        };
        match self.ast.item(item).kind {
            ItemKind::Let { value, path, .. } if self.ast.path(path).len() == 1 => {
                match self.ast.get(unparen(self.ast, value)).kind {
                    ExprKind::Fn(_) => LocalKind::Function(unparen(self.ast, value)),
                    _ => LocalKind::Variable,
                }
            }
            ItemKind::Field { .. } => LocalKind::Field,
            _ => LocalKind::Variable,
        }
    }

    /// The occurrences of the paths of the items that declare fields and
    /// variables: a top-level field's, each segment a node of its tree; any
    /// other's segments after its first, fields of its value.
    fn items(&mut self) {
        let ast = self.ast;
        let tree = &self.file.mentions.tree;
        for (position, &id) in ast.items(ast.root()).iter().enumerate() {
            let ItemKind::Field { path, .. } = ast.item(id).kind else {
                continue;
            };
            if tree.item_leaf.get(position).copied().flatten().is_none() {
                continue;
            }
            let mut own: Option<u32> = None;
            for segment in ast.path(path) {
                let Some(name) = segment.name() else {
                    break;
                };
                let next = match own {
                    None => tree.root(name),
                    Some(parent) => tree.child(parent, name),
                };
                let Some(next) = next else {
                    break;
                };
                own = Some(next);
                let global = self.file.global.get(next as usize).copied();
                self.declare(
                    segment.span,
                    global.map(Symbol::Node),
                    Some(name),
                    TyId::ERROR,
                );
            }
        }
        for id in ast.item_ids() {
            if self.sites.roots.contains(&id) && matches!(ast.item(id).kind, ItemKind::Field { .. })
            {
                continue;
            }
            let (ItemKind::Let { path, .. }
            | ItemKind::Field { path, .. }
            | ItemKind::Rebind { path, .. }) = ast.item(id).kind
            else {
                continue;
            };
            let segments = ast.path(path);
            let Some(first) = segments.first().and_then(PathSegment::name) else {
                continue;
            };
            let mut ty = self
                .decls
                .get(&(ast.item(id).span, first))
                .map_or(TyId::ERROR, |decl| decl.ty);
            for segment in segments.iter().skip(1) {
                let Some(name) = segment.name() else {
                    break;
                };
                ty = self.field_type(ty, name).unwrap_or(TyId::ERROR);
                self.declare(segment.span, None, Some(name), ty);
            }
        }
    }

    /// The type of the field `name` of `ty`, if it is a record type with
    /// one, or a future or the either shape of one.
    fn field_type(&self, ty: TyId, name: Name) -> Option<TyId> {
        let fields = self.table.record_fields(self.table.strip(ty))?;
        fields
            .iter()
            .find(|&&(field, _)| field == name)
            .map(|&(_, ty)| ty)
    }

    /// The occurrences of the names, and of the paths through the view.
    fn names(&mut self) {
        let ast = self.ast;
        for id in ast.expr_ids() {
            let expr = ast.get(id);
            let ExprKind::Name(name) = expr.kind else {
                continue;
            };
            if names::is_namespace_key(self.text(name)) {
                continue;
            }
            let declared = self.uses.get(&id).copied();
            let bound = declared.and_then(|declared| self.bound(declared, name));
            if declared.is_some() && bound.is_none() {
                // Read from a binding whose declaration is not found: what
                // it is, but no more.
                self.add(expr.span, None, Some(name), self.ty(id));
                continue;
            }
            match bound {
                Some(Bound::Local(at)) => {
                    let symbol = self.local(at);
                    self.symbols.insert(id, symbol);
                    self.add(expr.span, Some(symbol), Some(name), self.ty(id));
                }
                Some(Bound::Narrowed(operand)) => {
                    let symbol = self.symbols.get(&operand).copied();
                    if let Some(symbol) = symbol {
                        self.symbols.insert(id, symbol);
                    }
                    self.add(expr.span, symbol, Some(name), self.ty(id));
                }
                Some(Bound::View) | None => self.view_path(id, name),
            }
        }
    }

    /// The symbol of the scope's number `number`.
    fn symbol_of(&self, number: u32) -> Option<Symbol> {
        scope_symbol(&self.scope, number)
    }

    /// The occurrences of the reference whose variable is the name `name`
    /// at `id`, resolved in the file's view: each segment of the longest
    /// prefix the view declares.
    fn view_path(&mut self, id: ExprId, name: Name) {
        let Some(mut number) = self.scope.root(name) else {
            return;
        };
        let via = self
            .scope
            .target(number)
            .and_then(|_| {
                self.file
                    .aliases
                    .iter()
                    .find(|alias| alias.name == name && !alias.implicit)
            })
            .and_then(|alias| alias.item);
        let span = self.ast.get(id).span;
        let symbol = self.symbol_of(number);
        self.add_via(span, symbol, name, self.ty(id), via);
        if let Some(symbol) = symbol {
            self.symbols.insert(id, symbol);
        }
        let mut at = id;
        while let Some(&access) = self.parents.get(&at) {
            let ExprKind::Field { name: member, .. } = self.ast.get(access).kind else {
                break;
            };
            let Some(child) = self.scope.child(number, member) else {
                break;
            };
            number = child;
            let symbol = self.symbol_of(number);
            let span = member_span(self.ast, access, self.text(member));
            self.add_via(span, symbol, member, self.ty(access), via);
            if let Some(symbol) = symbol {
                self.symbols.insert(access, symbol);
            }
            self.covered.insert(access);
            at = access;
        }
    }

    /// Records an occurrence reached through the import `via`, if it was.
    fn add_via(
        &mut self,
        span: Span,
        symbol: Option<Symbol>,
        name: Name,
        ty: TyId,
        via: Option<ItemId>,
    ) {
        self.add(span, symbol, Some(name), ty);
        if let Some(last) = self.occurrences.last_mut()
            && last.span == span
        {
            last.via = via;
        }
    }

    /// Whether the field access `id` is a namespace's read, a def-type
    /// reference's or a definition's, which no source writes: its variable
    /// is a namespace's key.
    fn reads_namespace(&self, mut id: ExprId) -> bool {
        while let ExprKind::Field { expr, .. } = self.ast.get(id).kind {
            id = expr;
        }
        matches!(self.ast.get(id).kind, ExprKind::Name(name) if names::is_namespace_key(self.text(name)))
    }

    /// The occurrences of the field accesses no path through the view
    /// covers: a field read of a value, its symbol found if its record's
    /// origin is cheap to find.
    fn fields(&mut self) {
        let ast = self.ast;
        for id in ast.expr_ids() {
            let ExprKind::Field { expr: record, name } = ast.get(id).kind else {
                continue;
            };
            if self.covered.contains(&id) || self.reads_namespace(id) {
                continue;
            }
            let symbol = self
                .symbols
                .get(&record)
                .copied()
                .and_then(|base| self.member(base, name));
            if let Some(symbol) = symbol {
                self.symbols.insert(id, symbol);
            }
            let span = member_span(ast, id, self.text(name));
            self.add(span, symbol, Some(name), self.ty(id));
        }
    }

    /// The field `name` of the record `base` holds, if `base`'s value is
    /// written as a record block with a field item of that name: the field
    /// item's first segment, a local of the file that writes it.
    fn member(&self, base: Symbol, name: Name) -> Option<Symbol> {
        let (index, file, value) = match base {
            Symbol::Local { file, name: at } if file == self.index => {
                let item = self.locals.get(&at)?.item?;
                (self.index, self.file, item_value(self.ast, item)?)
            }
            Symbol::Node(node) => {
                let owner = self.scan.nodes.get(node as usize)?.leaf?;
                let file = self.scan.files.get(owner as usize)?;
                let own = file.global.iter().position(|&g| g == node)?;
                let leaf = file.mentions.tree.nodes.get(own)?.leaf?;
                (owner, file, leaf.value)
            }
            _ => return None,
        };
        let ast = &file.ast;
        let ExprKind::Block { items, .. } = ast.get(unparen(ast, value)).kind else {
            return None;
        };
        let field = ast.items(items).iter().find_map(|&item| {
            let ItemKind::Field { path, .. } = ast.item(item).kind else {
                return None;
            };
            let first = ast.path(path).first()?;
            (first.name() == Some(name)).then_some(first.span)
        })?;
        Some(Symbol::Local {
            file: index,
            name: field,
        })
    }

    /// The occurrences of def-type references', definitions' and metas'
    /// paths.
    fn def_types(&mut self) {
        let ast = self.ast;
        for (_, used) in ast.def_types() {
            let Some(key) = used.key else {
                continue;
            };
            if let Some(read) = used.read {
                self.reference_path(used.path, key, read);
            } else if let Some((meta_key, meta)) = used.meta
                && used.meta_read.is_none()
            {
                // A meta: its path is one name under its namespace's key.
                let number = self
                    .scope
                    .root(meta_key)
                    .and_then(|n| self.scope.child(n, meta));
                let symbol = number.and_then(|n| self.symbol_of(n));
                if let Some(span) = path_span(ast.path(used.path)) {
                    self.declare(span, symbol, None, TyId::ERROR);
                }
            } else {
                let jump = self.symbols.get(&used.dt).copied();
                self.definition_path(used.path, key, jump);
            }
        }
    }

    /// The occurrences of the path `path` of a def-type reference whose
    /// namespace's key is `key` and whose read of the definition is `read`,
    /// a field access for each segment: each segment the node the read
    /// reaches, or, past the definition, a field of its meta.
    fn reference_path(&mut self, path: PathRange, key: Name, read: ExprId) {
        let ast = self.ast;
        let mut accesses = Vec::new();
        let mut at = read;
        while let ExprKind::Field { expr, name } = ast.get(at).kind {
            accesses.push((at, name));
            self.covered.insert(at);
            at = expr;
        }
        accesses.reverse();
        let mut number = self.scope.root(key);
        for (segment, &(access, name)) in ast.path(path).iter().zip(&accesses) {
            number = number.and_then(|n| self.scope.child(n, name));
            let symbol = number.and_then(|n| self.symbol_of(n));
            self.add(segment.span, symbol, Some(name), self.ty(access));
        }
    }

    /// The occurrences of the path `path` of a definition whose namespace's
    /// key is `key`: the declaration of the node of each segment, going, as
    /// a definition, to `jump`, its def-type's.
    fn definition_path(&mut self, path: PathRange, key: Name, jump: Option<Symbol>) {
        let mut number = self.scope.root(key);
        for segment in self.ast.path(path) {
            let Some(name) = segment.name() else {
                break;
            };
            number = number.and_then(|n| self.scope.child(n, name));
            let symbol = number.and_then(|n| self.symbol_of(n));
            self.declare(segment.span, symbol, Some(name), TyId::ERROR);
            if let Some(last) = self.occurrences.last_mut()
                && last.span == segment.span
            {
                last.jump = jump;
            }
        }
    }

    /// The occurrences of the types' names in type positions, and of the
    /// `type` items' names.
    fn type_names(&mut self) {
        let ast = self.ast;
        let base = self.scan.type_base;
        let decl_index = |number: u32| {
            number
                .checked_sub(base)
                .filter(|&at| (at as usize) < self.scan.types.len())
        };
        for id in ast.type_ids() {
            let span = ast.type_expr(id).span;
            match ast.written_type(id) {
                TypeExprKind::Named(name) => {
                    let symbol = match self.file.type_view.get(&name) {
                        Some(&number) => decl_index(number).map(Symbol::Type),
                        None => BUILTIN_TYPE_NAMES
                            .contains(&self.text(name))
                            .then_some(Symbol::BuiltinType(name)),
                    };
                    if symbol.is_some() {
                        self.add(span, symbol, Some(name), TyId::ERROR);
                    }
                }
                TypeExprKind::Applied { name, span: at, .. } => {
                    if BUILTIN_TYPE_NAMES.contains(&self.text(name)) {
                        self.add(at, Some(Symbol::BuiltinType(name)), Some(name), TyId::ERROR);
                    }
                }
                TypeExprKind::Path(path) => {
                    let link = self.scan.qualified_type(self.file, ast.path(path), span);
                    if let TypeLink::Decl(number) = link
                        && let Some(decl) = decl_index(number)
                    {
                        self.add(span, Some(Symbol::Type(decl)), None, TyId::ERROR);
                    }
                }
                _ => {}
            }
        }
        for &id in ast.items(ast.root()) {
            let ItemKind::Type {
                name, name_span, ..
            } = ast.item(id).kind
            else {
                continue;
            };
            let decl = (0u32..)
                .zip(&self.scan.types)
                .find(|(_, declared)| declared.file == self.index && declared.item == id)
                .map(|(at, _)| Symbol::Type(at));
            self.declare(name_span, decl, Some(name), TyId::ERROR);
        }
    }

    /// The occurrences of the imports' names and paths.
    fn imports(&mut self) {
        let ast = self.ast;
        let implicit = ast.implicit_import();
        for &item in ast.items(ast.root()) {
            let ItemKind::Import(id) = ast.item(item).kind else {
                continue;
            };
            if Some(item) == implicit {
                continue;
            }
            let decl = ast.import_decl(id);
            let path = ast.path(decl.path);
            // Each prefix of the path, the module it names.
            for end in 1..=path.len() {
                let (Some(prefix), Some(segment)) = (path.get(..end), path.get(end - 1)) else {
                    break;
                };
                let Ok(target) = module(self.scan, self.modules, prefix, &mut Vec::new()) else {
                    continue;
                };
                self.add(
                    segment.span,
                    Some(target_symbol(target)),
                    segment.name(),
                    TyId::ERROR,
                );
            }
            let alias_of = |name: Name| {
                self.file
                    .aliases
                    .iter()
                    .find(|alias| alias.item == Some(item) && alias.name == name)
                    .map(|alias| alias.target)
            };
            match decl.form {
                ImportForm::Whole(alias) => {
                    let (Some(alias), Some(target)) =
                        (alias, alias.and_then(|alias| alias_of(alias.name)))
                    else {
                        continue;
                    };
                    self.add(
                        alias.span,
                        Some(target_symbol(target)),
                        Some(alias.name),
                        TyId::ERROR,
                    );
                }
                ImportForm::Members(members) => {
                    for &member in ast.import_members(members) {
                        let ImportMember::Named { name, alias } = member else {
                            continue;
                        };
                        let bound = alias.unwrap_or(name);
                        let Some(target) = alias_of(bound.name) else {
                            continue;
                        };
                        let symbol = Some(target_symbol(target));
                        self.add(name.span, symbol, Some(name.name), TyId::ERROR);
                        if let Some(alias) = alias {
                            self.add(alias.span, symbol, Some(alias.name), TyId::ERROR);
                        }
                    }
                }
            }
        }
    }
}

/// The value of the item `item`, a `let`'s or field's.
fn item_value(ast: &Ast, item: ItemId) -> Option<ExprId> {
    match ast.item(item).kind {
        ItemKind::Let { value, .. } | ItemKind::Field { value, .. } => Some(value),
        _ => None,
    }
}

/// The symbol of the number `number` of the file's view `scope`.
fn scope_symbol(scope: &FileScope<'_>, number: u32) -> Option<Symbol> {
    if let Some(global) = scope.merged(number) {
        return Some(Symbol::Node(global));
    }
    if let Some(view) = scope.own_view(number) {
        return Some(Symbol::View(view));
    }
    scope.target(number).map(target_symbol)
}

/// The symbol of what an import or a path through the view reaches.
const fn target_symbol(target: Target) -> Symbol {
    match target {
        Target::Node(node) => Symbol::Node(node),
        Target::View(view) => Symbol::View(view),
        other => Symbol::Module(other),
    }
}

#[cfg(test)]
mod tests;
