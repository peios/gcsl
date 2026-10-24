//! The parser: tokens to an [`Ast`].
//!
//! # Items and bodies
//!
//! A file, like the inside of a block `{ ... }`, is a *body*: a sequence of
//! items separated by newlines or `;` (`Parser::parse_body`). Runs of
//! separators, and so blank and comment-only lines, are skipped. An item is
//! `let PATH (: TYPE)? = EXPR`, `let mut NAME (: TYPE)? = EXPR`, `PATH :=
//! EXPR`, a field `PATH (: TYPE)? = EXPR`, `= EXPR`, a spread `...EXPR`, a
//! named function `fn NAME(...) ...`, a `break`, a `continue` or a `return`
//! (see [Loops](#loops)), an expression, or, in a file's body only, an
//! `import` (see [Imports](#imports)), a `type` (see [Types](#types)) or a
//! definition made with a def-type (see [Def-types](#def-types)).
//! After an item the next
//! token must be a separator or the end of the body; anything else is
//! "expected end of item".
//!
//! `let mut` declares a mutable variable: an ordinary `let` item, marked as
//! mutable (see [`ItemKind::Let`]). Its path must be a single name: `let mut a.b = e` is "`let mut`
//! needs a
//! plain name", at the path, since a dotted `let` updates a variable rather
//! than declaring one. `mut` is allowed nowhere else, and is "`mut` is only
//! allowed after `let`" at the `mut` wherever an item, an expression, a
//! name, a type or a function's parameter list would start, or an item
//! would end: before a field (`mut x = 1`) and a parameter (`fn(mut n:
//! Int)`) with a note saying what to write instead, and before a `for`'s
//! variable, after `fn` and everywhere else of those without one. (Where a
//! particular token is expected after a complete part of an item or an
//! expression, a separator, a closing bracket, `=`, `in` or a block, as in
//! `f(x mut)` or `let x mut = 1`, the usual "expected ..., found `mut`"
//! stands.)
//!
//! A spread item is recognised by `...` at the start of an item, and its
//! expression is parsed as a field's value is, at minimum binding power 0,
//! so `...conf : {b: Int}` spreads `conf : {b: Int}`. Whether the block it
//! is in may have a result is the checker's business, as for a field.
//!
//! A named function is recognised by `fn` followed by an identifier at the
//! start of an item, and is parsed as `let NAME = <the function expression
//! from its '(' on>`, both spanning `fn` through the body's `}`. `fn`
//! followed by anything else starts an ordinary expression item.
//!
//! A public function, `def PATH PARAMS RETURN BODY`, is recognised by `def`
//! at the start of an item, and has exactly a named function's grammar,
//! with the same three markers, but that it is named by a path; it is
//! parsed as the field `PATH = <the function expression from its '(' on>`.
//! Both forms are marked as written (`function` on [`ItemKind::Let`] and
//! [`ItemKind::Field`]), so the tree tells them from `let NAME = fn ...`
//! and `PATH = fn ...`. A `def`'s path is made of identifiers, `def
//! hello`, `def dev.peios.hello`: one that starts with a string is "a `def`
//! is named by an identifier, not a string", a quoted or an index segment
//! after its first, `def a."b"` or `def a[0]`, is "a `def`'s path is made
//! of identifiers", at the segment, and anything else where a name is
//! expected, after `def` or after a `.`, is an error at it. `def` is a
//! keyword, so anywhere else, as a name or an expression, it is reported as
//! any keyword is there.
//!
//! # Attributes
//!
//! An *attribute* is a token of its own (see the lexer's
//! [Attributes](crate::lexer#attributes)): `#[NAME]` an outer one, which
//! applies to the item after it, and `#![NAME]` an inner one, which applies
//! to the whole file. There are three names, and no arguments:
//!
//! - `#[shadowable]` marks a top-level field (a dotted one included), a
//!   `def`, a definition made with a def-type or an `import` of any form, as
//!   one a later declaration of its name may shadow (see
//!   [`crate::workspace`], "Shadowable declarations"), recorded in the
//!   tree's side table ([`Ast::is_shadowable`]). It applies to the next
//!   item, on its line or a later one, comments and blank lines between
//!   being skipped as any separators are.
//! - `#[def_type]` marks a top-level `def`, of one name or a path, as a
//!   def-type (see [Def-types](#def-types)), recorded in a side table of its
//!   own ([`Ast::is_def_type`]). It applies to the next item as
//!   `#[shadowable]` does, and both may mark one item, in either order;
//!   before anything but a top-level `def` (a definition made with a
//!   def-type, a field, a `fn`, an item of an inner block) it is
//!   "`#[def_type]` applies only to a file's top-level `def`s", at the
//!   attribute, and its other errors are `#[shadowable]`'s, with its name.
//! - `#![no_prelude]` marks the file as one without the implicit `import *
//!   from prelude` ([`Ast::no_prelude`]). It must come before the file's
//!   first item, comments and blank lines alone before it.
//!
//! The errors, each at the attribute: an unknown name is "unknown attribute
//! `NAME`"; `#![shadowable]` is "`shadowable` applies to a declaration:
//! write `#[shadowable]`" and `#[no_prelude]` "`no_prelude` applies to the
//! whole file: write `#![no_prelude]`"; one written twice on one item is
//! "`#[shadowable]` is written twice", and twice on the file "`#![no_prelude]`
//! is written twice", each at the second; `#![no_prelude]` anywhere but
//! before the file's first item (after an item or an outer attribute, or in
//! a block) is "`#![no_prelude]` must come before anything else in the
//! file"; `#[shadowable]` before anything but a top-level field, `def` or
//! `import` (a `let`, `let mut`, `fn`, `:=`, a `type`, a spread, a result,
//! an expression, a `break`, `continue` or `return`, or any item of an
//! inner block or of a `for` header) is "`#[shadowable]` applies only to a
//! file's
//! top-level fields, `def`s and imports", with the note "a `let` or `fn`
//! already shadows" before a `let`, a `let mut` or a `fn`; and one with no
//! item after it, at the end of its body, is "`#[shadowable]` is not
//! followed by anything". An attribute where an expression or a type
//! starts, or after an item on its line, is reported as any token is there.
//! Whether a session's input may be marked `#![no_prelude]` is the
//! workspace's to say: the parser does not know what it parses.
//!
//! # Def-types
//!
//! A def-type, a `def` marked `#[def_type]`, makes definitions, and a
//! reference calls one (see the checker's "Def-types"):
//!
//! ```text
//! DEFINITION = "def" DT PATH "(" (NAME ("," NAME)* ","?)? ")" BLOCK
//! REFERENCE  = DT PATH
//! META       = "defmeta" DT PATH (BLOCK | "=" EXPR)
//! DT         = IDENT ("." IDENT)*
//! PATH       = SEGMENT ("." (SEGMENT | NUMBER))*
//! SEGMENT    = IDENT | IDENT "+" ...      (the run glued to it, below)
//! NUMBER     = INT ...  | FLOAT ...       (from a digit, below)
//! ```
//!
//! A segment of PATH, and only of PATH, may have `+` in its name after its
//! first character, as real packages' names do, `org.gnu.gmp-c++`, whose
//! `+` the lexer, unchanged, makes tokens of their own
//! (`Parser::path_segment`): when a `+` directly follows a segment's
//! identifier, every token directly following the one before it, nothing
//! between them, whose text is made only of ASCII letters, digits, `_`, `+`
//! and `-` is glued to the identifier, a lone `-` only with a token glued
//! after it, and the segment is named by the source text of the run,
//! interned as one name. So a segment is a maximal run of ASCII letters,
//! digits, `_`, `-` and `+`, starting with a letter or `_` (or, after the
//! first segment, a digit, below), in which every `-` is directly followed
//! by a letter, digit, `_` or `+`, and which may end with `+` but not with
//! `-`, whatever its tokens are: `gmp-c++`, `a+b`,
//! `c++11`, and `gcc-libstdc++-devel`, lexed as `gcc-libstdc`, `+`, `+`, a
//! negation's `-`, and `devel`. What the lexer rejects stays rejected
//! (`c++11-x` is "`-` here is ambiguous", as `1-x` is, and `c++1_` a number
//! literal's error), and a token only part of whose text is of those
//! characters, a float with a fraction (`c++1.5`) or a raw string
//! (`a+r"x"`), ends the segment before it, where both are an error. `.`,
//! `(`, `[`, a space or any other token ends a segment. Anywhere else,
//! `a+b` is the addition it always was: in a field's path, a variable, a
//! `def`'s name (`def a+b()` is "expected `(` after `fn`, found `+`") and
//! DT. One program that parsed before means something else: a reference
//! directly followed by `+`, `package x.a+b` or `package x.a+1`, was the
//! addition of the reference and what followed, and is now one reference
//! (references are new); with a space before the `+`, it is the addition
//! still. A `+` directly after a `.` is the error it was, since a segment
//! starts with an identifier or a digit. The names are definitions' like
//! any other from here on, shown as written wherever a definition's path is
//! ([`Ast::show_definition_path`]).
//!
//! A segment after the first may start with a digit, as versioned names
//! do, `org.golang.go-stage0-1.24` and `org.rust-lang.rust-1.83`
//! (`Parser::number_segments`): after a `.`, an integer literal starts a
//! segment, and so does a float, split at its `.` as if it were an integer,
//! `.` and an integer, each part with its own span (the lexer, unchanged,
//! makes `x.1.83.0` `x`, `.`, the float `1.83`, `.` and `0`: the segments
//! `x`, `1`, `83` and `0`). The segment the integer starts, or the float's
//! part after its `.`, is the run of it and every token glued to it as
//! above, `+` after it or not, named by the run's source text: `24rc1`
//! (`24` and `rc1`) is one segment, `1.83rc1` the two `1` and `83rc1`, `1_0`
//! is named `1_0`, a float with an exponent and no `.`, `1e5`, is one
//! segment, and `1.5e3` is `1` and `5e3`. A segment of digits alone is a
//! name like any other, `24`, which is not `024`. The first segment still
//! starts with an identifier, which is what makes a definition, a meta or
//! a reference (`def package 1.x(v)` is "expected `(` after `fn`, found
//! `1`", as it was), and nowhere else does a digit start a name: `x.0` in a
//! field's path, an expression, a `def`'s name or DT is the error it was.
//! What the lexer rejects stays rejected: an integer out of the range of
//! `Int`, `x.99999999999999999999` (a float's part after its `.` is no
//! integer, so `x.1.99999999999999999999` is `1` and
//! `99999999999999999999`), and a `-` directly after a number, `x.1-rc`,
//! "`-` here is ambiguous". A float in a run, not directly after a `.`, is
//! not split: `x.c++1.5` and `x.24+1.5` end the segment before it, where it
//! is an error, as `c++1.5` was. No program that parsed means anything
//! else: a number after a `.` that ends a reference was "expected a field
//! name after `.`", and after a definition's or a meta's path "expected `(`
//! after the definition's path" or "expected `{` or `=` after the meta's
//! path".
//!
//! A definition, [`ItemKind::DefTypeDef`], is recognised after `def` and a
//! name or a path (`Parser::parse_def`): an identifier that follows them on
//! their line starts its path. Its parameters are written by name alone,
//! each given a type expression nothing reads ([`TypeExprKind::Invalid`]);
//! a type written is "the parameter types of a `package` definition come
//! from `package`", at its `:`, and a return marker after them "the result
//! type of a `package` definition comes from `package`", at the marker,
//! `package` being its DT as written. Its path is a `def`'s, identifiers
//! joined by `.`, but that a segment may have `+` in it, and one after the
//! first start with a digit, as above ("a `def`'s path is made of
//! identifiers" otherwise, at the segment), then `(`, the parameters and the
//! body, each "expected ..." otherwise. It is an item of a file's outermost body only: anywhere
//! else,
//! "a def-type definition is declared only at the top level of a file", at
//! its `def`. Its function expression spans its path through its body, the
//! expression that reads its DT coming before its body's in the arena.
//!
//! A reference, [`ExprKind::DefTypeRef`], is recognised where a postfix
//! operator may follow an expression (`Parser::parse_expr_inner`): after a
//! name, or a chain of field accesses directly on one, an identifier that
//! starts on the same line starts its path, so `package dev.peios.testpkg()`
//! is the call of the reference, and `-p x` the negation of one; it binds as
//! a field access does. "The same line" is the source's: the lexer emits no
//! newline inside parentheses, so `f(a\nb)` is the error it was, and two
//! names on two lines of a body are two items. Two names in a row never
//! parsed before, so every program that parsed parses as it did, but for
//! a reference directly followed by `+` (above); one that
//! did not may now fail later, at the checker, which notes that two names
//! in a row are read as a reference. Its path is a definition's, read
//! whole, however long, since what part of it names a definition the
//! checker finds (`package dev.peios.testpkg.versions` reads the meta of
//! `package dev.peios.testpkg`, see the checker's "Metas"); but that an
//! index ends it, as in `package x.versions[0]`, where a definition's path
//! is the error below. A quoted
//! field name of the form of a namespace's key (see [`crate::names`],
//! "Namespace keys") is "a field cannot be named `\"<p#1.2>\"`: names of
//! that form are def-types' namespaces", at the name, so no source can
//! write a key, nor a meta key, `\"<defmeta p#1.2@3>\"`. A definition also
//! holds the read of its meta, a name and a field access both standing for
//! nothing until the workspace names them, allocated with its path, before
//! its function in the arena and starting at its `DT`
//! (`Parser::unresolved_meta_read`).
//!
//! A definition's meta, [`ItemKind::DefMeta`] (see the checker's "Metas"),
//! is recognised where an item starts (`Parser::starts_defmeta`): the name
//! `defmeta`, then on its line a name or a dotted path, its DT, then on the
//! line of that an identifier, which starts its PATH, a definition's path.
//! `defmeta` is contextual, as `type` is: anywhere else it is a name, a
//! variable, a field or a def-type (`defmeta = 1`, `x = defmeta p`, `defmeta
//! p { ... }` the call of a reference with a trailing block), and two paths in
//! a row after it never parsed before, so every program that parsed parses
//! as it did. After PATH comes its body, a block, or `=` and any
//! expression, which is the result of a block of that one item, written
//! nowhere: the body of a function expression of no parameters, no return
//! type, and a marker that means nothing, spanning its path through its
//! body, as a definition's does (`Parser::parse_defmeta`). Anything else
//! after PATH is "expected `{` or `=` after the meta's path", and `META` and
//! a marker there "a meta is written only in a function type". It is an
//! item of a file's outermost body only: anywhere else, "a `defmeta` is
//! declared only at the top level of a file", at its `defmeta`.
//! `#[shadowable]` before one is "`#[shadowable]` does not apply to a
//! `defmeta`", at the attribute, since a definition that shadows another
//! brings its own meta; `#[def_type]` is the error it is before anything but
//! a `def`.
//!
//! # Imports
//!
//! An import is an item of a file's outermost body only:
//!
//! ```text
//! IMPORT  = "import" PATH ("as" NAME)?
//!         | "import" MEMBER ("," MEMBER)* "from" PATH
//! MEMBER  = NAME ("as" NAME)? | "*"
//! PATH    = SEGMENT ("/" SEGMENT)*          (one run of glued tokens, below)
//! SEGMENT = a run of ASCII letters, digits, `_`, `-`, `+` and `.`
//! ```
//!
//! (see [`ItemKind::Import`] and [`ImportDecl`]). `import` is a keyword. `as`
//! was already one, for narrowing (`x as Int`), and is the same token here.
//! `from` is contextual: it is the keyword wherever it stands in an import
//! item, its first token after `import` included, and a name everywhere
//! else, so a module or a member called `from` cannot be imported (`import
//! from as f` is "expected a path after `from`"). The two forms are told
//! apart after the first path or member and its `as`: a `,` or `from` next
//! makes it a member list.
//!
//! A path names directories and files from the workspace's root, as the
//! file system does: its segments are separated by `/`, and a segment is
//! named as its directory or file is, dots and all, so
//! `pkgs/net.busybox.busybox` is the two segments `pkgs` and
//! `net.busybox.busybox` (`Parser::parse_module_path`). The lexer, unchanged,
//! makes tokens of the path's parts, `pkgs`, `/`, `net`, `.` and so on, and
//! the path is the run of the token it starts with, an identifier or a
//! number, and every token after it that directly follows the one before,
//! nothing between them, whose text is made only of ASCII letters, digits,
//! `_`, `-`, `+`, `.` and `/` (`Parser::module_run`); the run's text is split
//! at each `/`, and each part, interned, names a segment
//! ([`PathSegment`], a name of the source's text, spanning its part). So
//! a space ends the path: `import lib /x` is the path `lib` and "expected
//! end of item, found `/`". A segment starts with an ASCII letter, a digit
//! or `_`, ends with one or `+` (`gmp-c++`), and has no `..` in it
//! ([`lexer::is_module_segment`]): any other is "`x.` cannot name a module:
//! ...", at the segment, and a `/` that ends the path, or follows another,
//! "expected a module name before and after each `/`", at it.
//!
//! A whole import with no `as` binds its path's last segment, which must
//! then be a name: an identifier, not a keyword. `import
//! pkgs/net.busybox.busybox` is "`net.busybox.busybox` is not a name, so
//! the import needs one", at the segment, with a note to write `as NAME`,
//! and, when the segment is identifiers joined by `.`, a second saying that
//! a path separates its segments with `/`: dots once separated them, and
//! `import lib.x`, which was `lib/x.gcsl`, is now the one segment `lib.x`
//! (the workspace says the same of such a path that names nothing, and
//! suggests the path with slashes, see [`crate::workspace`], "Imports").
//! With `as`, any segment will do.
//!
//! An `import` that starts an item of any other body, a block's, a
//! function's or a `lazy`'s, is "`import` is only allowed at the top level
//! of a file"; anywhere else it is reported as any keyword is. A path
//! written as a string (`import "github.com/x"`) is "a module path is
//! written without quotes, its segments separated by `/`"; any other token
//! where a path starts is "expected a module path after `import`" or
//! "expected a path after `from`". A member that is not one name, a path
//! (`import a/b, c from p`) or a name with dots (`import a.b from p`), is
//! "a name imported with `from` is a single name". `*` without `from` is
//! "`import *` needs `from` and a path", and a second `*` in one list "`*`
//! is imported more than once". Whether the path names anything is the
//! workspace's business, and outside a workspace the checker's.
//!
//! # Paths
//!
//! A path is a name followed by any number of segments, each `.` and a
//! name or, in the path of a `:=`, an index, `[EXPR]`:
//!
//! ```text
//! PATH    = NAME SEGMENT*
//! SEGMENT = "." NAME | "[" EXPR "]"
//! ```
//!
//! as in `x`, `server.port`, `"log level".format`, `xs[i]`,
//! `conf.hosts[0].up` and `grid[y][x]`. An index segment's `[` follows the
//! rule it follows in an expression (see [Lists](#lists)): it must start on
//! the line the path is on, so a `[` after a newline begins the next item.
//! Its position is any expression, parsed as an index's is and kept in the
//! tree, and the source text it is written as is added to the string table,
//! for showing the path as it is written (see [`crate::ast::PathSegment`]);
//! a position written over several lines is shown on one, each line break,
//! with the spaces around it, one space.
//!
//! After `let` the parser reads the path directly: an identifier, then its
//! segments as they follow. A field item or a rebind has no keyword, so the
//! parser looks ahead first (`Parser::starts_path_item`): if the item's
//! tokens are a name or a plain string literal, then `.` and a name or a
//! `[`, anything and its matching `]` as many times as they follow, and
//! then `=` or `:=`, it reads the path directly in the same way, and its
//! first name is then checked as it was when the path was read back from an
//! expression: a variable's name must be an identifier, so the first
//! segment of the path of a `let` or a `:=` cannot be a literal, and a
//! field's may be a plain string literal; later segments may be quoted.
//! Otherwise the item is parsed as an expression, and an `=` or `:=` after
//! it means that it is not a path: "expected end of item", or, before a
//! chain of field accesses on a literal that cannot be its name, the error
//! for that literal. The lookahead only counts brackets, and the tokens
//! between them are then parsed as the position, so an item starting with
//! an index is looked through twice.
//!
//! A field item may be annotated, `PATH: TYPE = EXPR`, as a `let` may
//! ([`ItemKind::Field`]'s `ty`): the lookahead then finds `:` after the
//! path, and then, before the item can end (`Parser::annotated`), an `=`
//! outside any brackets, after the type. Without that `=` the item is the
//! expression it always was, the path's ascription: `x : Int` alone, a
//! block's `{ x : Int }`, or `x : Int` followed on the next line by `y =
//! 2`. The item ends, for the lookahead, at a `;`, a `:=`, a closing
//! bracket it is inside, the end of input, or a newline outside brackets
//! but after `:`, `|`, `<`, `->` or `->>`, after which the type goes on, as
//! a type itself does (newlines inside a record type's braces and a
//! function type's parentheses are skipped as the type is). An annotation
//! must be followed by
//! `=`, "expected `=` after the field's type" otherwise, and a `:=` is never
//! annotated. A quoted name may be annotated as a plain one, `"a b": Int =
//! 1`, and on a dotted path the annotation applies to the last segment, as
//! for a `let`. A `def` is not annotated: it has its signature.
//!
//! Only a `:=` path may have index segments: a `let` path with one, `let
//! xs[0] = 1`, is "a `let` path cannot index a list", and a field's, `xs[0]
//! = 1`, "a field's path cannot index a list", each at the first index's
//! brackets, with a note writing the path as a `:=`. (`let mut xs[0] = 1`
//! is the first, not "`let mut` needs a plain name".)
//!
//! A dotted path nests what its item declares: later stages build a record,
//! or a list, for each segment but the last, updating one inside another,
//! so a path of `N` segments makes its item `2(N - 1)` levels taller than
//! the taller of its value and its index segments' positions (see
//! [`MAX_DEPTH`]).
//!
//! A newline ends an item except where the item cannot be complete: after
//! a binary operator, after `:` (in an annotation or an ascription, or as a
//! function's return marker), `->>`, `->`, `is` and `as`, and after `=`,
//! `:=`, `let`,
//! the `mut` of a `let mut` and a spread item's `...`. There
//! the parser skips newlines before going on. Directly inside parentheses,
//! brackets and interpolations the lexer emits no newlines at all, so an
//! expression, a parameter list, an argument list, a list literal or an
//! index there may span lines freely; a
//! block inside them is newline-sensitive again, so a multi-line block or
//! function body may be passed as an argument.
//!
//! # Names
//!
//! A field's name, where it is declared, accessed or written in a record
//! type, may be an identifier or a plain string literal: one without the
//! `r` prefix or interpolations, whose text the parser interns into the
//! [`Names`] like an identifier, so `"x"` and `x` are the same name. A raw
//! or interpolated literal there is an error. A variable's name, and so a
//! parameter's, must be an identifier.
//!
//! # Types
//!
//! A type is a name, such as `Int`, a qualified name, `t.PackageDef`, a
//! name applied to a type argument, such
//! as `Future<Int>` or `List<Int>`, a record type, a function type, a type
//! in parentheses, a list type written `T[]`, or a union of types. A
//! qualified name is an identifier directly followed by `.`, and then
//! identifiers separated by `.` ("expected a type's name after `.`"
//! otherwise), a [`TypeExprKind::Path`]; what it names is the workspace's
//! business (see [`crate::workspace`], "Types"). So `x : Int.y`, which was
//! the field access `(x : Int).y` and could only be an error, is now the
//! ascription of a qualified name. A
//! record type is `{` then `}`, or entries separated by `,`, each `NAME:
//! TYPE`, `NAME: TYPE = DEFAULT`, a spread, `...TYPE`, whose fields join
//! the record type, or a rest-field, `*: TYPE`, the type of every field the
//! record type does not name, with an optional trailing `,`, then `}`. A
//! rest-field has no default: an `=` after its type is "a rest-field cannot
//! have a default"; how many a record type has is the checker's business.
//! A field's
//! default, [`FieldDefault::Expr`], is any expression, after the `=`, as a
//! parameter's default is (`Parser::parse_field_default`), wherever the
//! record type is written: in an annotation, a `type`, a parameter's or a
//! return type, a list or function type, nested in another record type or
//! in a union; what a default may be is the checker's business (see its
//! "Field defaults"). A spread has none: an `=` after one is "expected
//! `,` or `}`", as before. The lexer does not know it is
//! in a type, so it emits the newlines inside the braces, and the parser
//! skips them all, around a default's `=` too: a record type may span lines
//! freely. Repeated field names
//! are left to the checker, and the merging of spreads to the expansion of
//! the file's types (see [`crate::ast`], "Types"). A function type is
//! `fn`, `(`, parameter types
//! separated by `,` with an optional trailing `,`, `)`, the marker `:`,
//! `->>` or `->`, and the return type; a parameter type is `NAME: TYPE`,
//! recognised
//! by an identifier followed by `:`, or just `TYPE`. A parameter type has no
//! default: an `=` after one is "a default is written only in a function
//! expression's parameters", at the `=` (see [Functions and
//! calls](#functions-and-calls)).
//!
//! A function type may have a *meta* (see the checker's "Metas"): after its
//! whole return type, a union included, the identifier `META` directly
//! followed by `:`, `->>` or `->`, its marker, then the meta's type, newlines
//! skipped after the marker, as in `fn(v: Version): PackageDef META->
//! PackageMeta` ([`MetaTy`], `Parser::meta_marker`). `META` is no keyword:
//! anywhere else it is a name, and followed by anything else after a return
//! type, it is what it always was; a newline outside parentheses ends the
//! type first, as it always did, so a meta begins on the line its return type
//! ends on. Since the innermost return type takes it, the meta of a function
//! type that returns a function type is written with that return type in
//! parentheses, `fn(): (fn(): Int) META: {a: Int}`. Only a function type has
//! a meta: `META` and a marker after the return type of a function
//! expression (a `fn` or a `def` included), or after the parameters of a
//! definition made with a def-type, are "a meta is written only in a
//! function type", at `META` and its marker. No program that parsed had
//! `META` and a marker there, so every one parses as it did.
//!
//! An applied type is any identifier directly followed by `<`, a type, and
//! `>`, which must follow the type directly; there is no `>>` token, so
//! `Future<Future<Int>>` closes with two `>`. Which names take an argument
//! is the checker's business. A union is two types separated by `|`, which
//! binds looser than everything else in a type: after any type, while a `|`
//! follows, the parser reads another and makes the union of the two, so a
//! union is left-associative. A function type's return type is a whole
//! type, so it takes a union that follows it: `fn() -> Int | Future<Int>`
//! returns the union. Parentheses group a type, `(` TYPE `)`, so that
//! `(fn(): Int) | Future<fn(): Int>` is the union of a function type and a
//! future; they leave no node of their own, the type inside standing for
//! itself, and a missing `)` is "expected `)` to close the type". Newlines
//! are skipped after `<`, `|` and `(`, as after `:`. In expression position
//! `|` is not an operator, so it is "expected expression".
//!
//! `T[]` is a postfix on any type that is not a union (a *member*, as
//! `Parser::parse_type_member` parses it), repeatable, so `Int[][]` is a
//! list of lists, `{a: Int}[]` and `(fn(): Int)[]` are lists, and `Int |
//! Future<Int>[]` is the union of `Int` and a future of a list, which the
//! checker rejects. A `[` after a type is this postfix only if a `]`
//! follows it directly; in `fn(): Int[]` it belongs to the return type,
//! which is parsed first.
//!
//! A `type` item, `type NAME = TYPE` ([`ItemKind::Type`]), declares a type
//! (`Parser::parse_type_item`), its name a single identifier, newlines
//! skipped after its `=`. `type` is no keyword of the lexer: it is
//! contextual, a `type` item's start only where an item starts and an
//! identifier follows it on its line (`Parser::starts_type_item`), which no
//! item could before, so every program that parsed parses as it did, and a
//! variable or field may still be named `type` (`type = 1`, `let type =
//! 2`, `type.x = 3`, `type: Int = 4`). A `type` item is an item of a file's
//! outermost body only: anywhere else, a block's, a function's or a
//! `lazy`'s, it is "`type` is declared only at the top level of a file", at
//! the `type`. Type parameters, `type Box<T> = ...`, are "a `type` takes no
//! type parameters", at the `<`; anything else after the name is "expected
//! `=` after the type's name". `#[shadowable]` before one is the error it is
//! before anything but a field, a `def` or an import. Whether a name is
//! some built-in type's, or declared twice, is the checker's business.
//!
//! # Functions and calls
//!
//! A function expression is `fn`, `(`, parameters `NAME: TYPE` separated by
//! `,` with an optional trailing `,`, `)`, the return marker, the return
//! type, and the body, a block whose `{` must follow the return type
//! directly, as an `if`'s block follows its condition. Every parameter is
//! annotated. A parameter may have a default, `NAME: TYPE = EXPR`, any
//! expression after the `=` as an argument is (`Parser::parse_default`),
//! in any position, in a `fn` expression, a named `fn` item, a `def` and a
//! def-type's `def` alike, since each is parsed as one ([`Param::default`]).
//! Once the parameters and the return marker are parsed, each default that
//! is no constant ([`crate::ast::is_constant`]) is given its *default
//! function* ([`Param::default_fn`], `Parser::with_default_fns`): a
//! [`FnDecl`] with no expression of its own, whose parameters are copies of
//! the ones before it, as written, without their defaults, whose return
//! type is the parameter's, whose effect is the function's (a memoised
//! one's is impure), whose marker is the function's, and whose body is the
//! default, as a `lazy`'s body is an expression. It adds no syntax: the
//! default is still where it is written, and the formatter and the index see
//! it there. How it is checked, and run, is the checker's business (see
//! [`crate::check`], "Calls"). Not in a function type,
//! nor in the parameters of a definition made with a def-type, which are
//! names alone ("expected `,` or `)`" at an `=` there, as before). The
//! marker is one of three, each an [`Effect`] of the
//! [`FnDecl`] and of a function type alike: `:` for a pure function, `->>`
//! for an impure one that is memoised, run at most once per distinct
//! arguments, and `->` for an impure one run on every call.
//!
//! A call is a postfix operator: after an operand, `(` starts an argument
//! list, whose arguments are separated by `,` with an optional trailing
//! `,`, each an expression or `NAME = EXPR` (recognised by an identifier
//! followed by `=`). After a named argument, an argument that is a name
//! alone, an identifier directly followed by `,` or `)`, is shorthand for
//! `NAME = NAME`, as in `gcc.compile(src, output = "a.o", compiler)`: a
//! named argument ([`ArgKind::Named`], marked `shorthand`) whose value is
//! the read of the variable of its name, at the same span. Before any named
//! argument a name alone is positional, as any expression is, and after one
//! any other expression is positional still (`f(a = 1, 2)`), so only a
//! program that passed a variable positionally, by its name alone, after a
//! named argument means something else: now the argument of the parameter
//! of the variable's name, an error if there is none. After the `)`, or
//! instead of the list, a `{` on the same line starts a trailing block: an
//! ordinary block, passed as the call's last argument. The parser accepts
//! arguments in any order; binding them to parameters is the checker's
//! business.
//!
//! # Lists
//!
//! A list literal is `[`, elements separated by `,` with an optional
//! trailing `,`, and `]`; `[]` is the empty list. An element is an
//! expression, or `...` and an expression, a spread, parsed at minimum
//! binding power 0 as any element is, so `...a + b` spreads `a + b`. A
//! `...` anywhere else that an expression starts, but at the start of an
//! item (see [Items and bodies](#items-and-bodies)), is an error, "`...` is
//! only allowed in a list or at the start of an item".
//! Directly inside `[ ]` the lexer emits no newlines, as inside `( )`, so a
//! literal may span lines.
//!
//! An index, `EXPR[INDEX]`, is a postfix operator, binding as tightly as a
//! call and a field access and chaining with them: `f(x)[0].name[1]`. Like a
//! trailing block, it must start on the line its operand ends on: after a
//! newline the item has ended, so a `[` starting a line begins a list
//! literal, the next item. Inside brackets, where there are no newline
//! tokens, a `[` on the next line is still an index.
//!
//! # Futures
//!
//! `lazy EXPR` is a prefix expression that takes the longest expression it
//! can: its operand is parsed at minimum binding power 0, so `lazy a + b`
//! defers the sum and `lazy { ... }` a block. The operand becomes the body
//! of a parameterless function in the function arena, with no return type,
//! impure, whose marker is the `lazy` keyword. `await` is a prefix operator
//! binding exactly as tightly as `-`: `await x + 1` is `(await x) + 1`,
//! `await f()` awaits the call's result, and `await x.y` the field.
//!
//! # Trailing blocks
//!
//! A trailing block would make `if x { ... }` a call of `x`, so while the
//! parser is in the condition of an `if` (or `else if`), the iterable or
//! the guard of a `for`, or the header of a condition loop, a `{` after an
//! expression never starts one: the parser keeps a flag,
//! `Parser::braces`, that `Parser::parse_if` sets around its condition and
//! `Parser::parse_for` around its iterable and guard, or its header. The
//! flag applies to the whole condition at every operator, so `if a and b {
//! }` is an `if` with condition `a and b`. It is lifted, and restored
//! afterwards, inside parentheses, argument lists, list literals, indexes,
//! blocks and interpolations, so `if (f { }) { }` calls `f`. A newline
//! before a `{` ends the item as always, so it never starts a trailing
//! block either.
//!
//! In a `for` header the flag says more: a `{` is the body even where an
//! expression starts, so a block, a record block included, written as a
//! header's value or condition is an error, "a `{` in a `for` header begins
//! its body", whose note says to put the block in parentheses, as in `for
//! let r = ({ a = 1 }); r.a < 3; ... { }`. No `;` is looked for ahead to
//! decide what a `{` is.
//!
//! # Newlines after an `if`
//!
//! The one place a newline does not end an item even though the expression
//! before it could be complete is after the block of an `if`: an `else` may
//! start the next line, so the parser looks past newlines for one, and puts
//! them back for the item separator if it finds none.
//!
//! # Loops
//!
//! Every loop is a prefix expression, with its details in the tree's `for`
//! arena (see [`crate::ast::ForDecl`]). After `for` and its label, if any
//! (see [Labels](#labels)), what follows decides the form
//! (`Parser::parse_for`):
//!
//! - A name directly followed by `in` begins the iteration, `for NAME in
//!   ITERABLE (if GUARD)? BLOCK`: the iterable, parsed at minimum 0 as an
//!   `if`'s condition is, optionally `if` and a guard, parsed the same way,
//!   and the body, a block that must follow on the same line. No operator
//!   starts with `if`, so an `if` after the complete iterable always begins
//!   the guard; an iterable that is itself an `if` expression starts with
//!   `if` and is parsed whole first.
//! - A `{` begins the body of `for BLOCK`, which has no condition.
//! - Anything else begins a header (`Parser::parse_header`): parts
//!   separated by `;`, after each of which newlines are skipped, and then
//!   the body. A part that begins `let` is a `let` item, parsed as one is
//!   in a block, and one that begins as a path item does
//!   (`Parser::starts_path_item`), with `:=`, is a rebind; any other part
//!   is an expression, the condition. Every expression in it, the values of
//!   its items included, is parsed as an `if`'s condition is, with no
//!   trailing block, and a `{` where one would start is the body, and so
//!   an error (see [Trailing blocks](#trailing-blocks)). Exactly one part
//!   is the condition: a second is "a `for` header has one condition", at
//!   it, labelled "the condition is here" at the first, and none is "a
//!   `for` header needs a condition", at the header, with a note to write
//!   `true`. A `let` after the condition is "a `let` after the condition
//!   would be new on every iteration and seen by nothing", with a note to
//!   declare it before the condition and rebind it with `:=`; a field item
//!   is "a `for` header cannot define a field". Where neither `;` nor `{`
//!   follows a part, the error is the one the iteration's would be when
//!   the header is a single part that looks like one: "expected a name
//!   after `for`" for a part that starts with no name and is followed by
//!   `in` (a variable's name for a string literal), and "expected `in`
//!   after the `for` variable" at what follows the name of a part that
//!   starts with one; and "expected `;` or `{` in a `for` header"
//!   otherwise.
//!
//! `do BLOCK for COND` (`Parser::parse_do`) is `do`, its label, if any,
//! the body, and `for`, which must follow the body's `}` on the same line,
//! "expected `for` and a condition after the body of a `do`" otherwise,
//! and then the condition, an ordinary expression at minimum 0, which ends
//! as an expression statement ends: a `{` after it on its line starts a
//! trailing block, as anywhere else, not a body. Only a condition may
//! follow the `for`: a label, a `let` or a name followed by `in` there is
//! "only a condition follows the `for` of a `do`".
//!
//! `break`, `continue` and `return` are items (`Parser::parse_jump`),
//! allowed where any item of a block is, the keyword followed by its label,
//! if any, and its value, if one begins on the same line: a newline, a
//! `;`, a `}` or the end of input after the keyword and the label means
//! there is none. Where an expression starts, they are "expected
//! expression", as `let` is, so `x or return 1` is an error. Where a
//! `return` may go, and what it leaves, is the checker's business.
//!
//! # Labels
//!
//! A label, [`TokenKind::Label`], is written directly after the `for`,
//! `do`, `break`, `continue` or `return` it belongs to, or directly after
//! the `{` of a block, which it names (`Parser::parse_block`): any block,
//! a block expression, a function's body, a branch of an `if`, a loop's
//! body, a `lazy`'s block or a trailing block. With anything between, it
//! is "a label is written directly after `for`, with no space" (or `do`,
//! `break`, `continue`, `return`, `{`), at the label. A block with a field
//! or a spread item, a record block, cannot be labelled: "a record block
//! cannot be labelled", at the label, once the block is parsed. Anywhere
//! else, where an item or an expression starts or ends, or the name of a
//! `for`'s variable or of a `let` is expected, an interpolation's braces
//! included, it is "a label names a loop or a block: it follows `for`,
//! `do`, `break`, `continue`, `return` or `{`", at the label.
//!
//! # Expressions
//!
//! Expressions are parsed by a Pratt parser, also called precedence
//! climbing. Instead of one grammar function per precedence level, a single
//! function, [`Parser::parse_expr`], parses any expression whose operators
//! bind at least as tightly as a given *minimum binding power*. It first
//! parses a *prefix* expression (a literal, a name, a string, a
//! parenthesised expression, a block, a list literal, an `if`, a `for`, a
//! function expression, a `lazy`, or a prefix operator applied to an
//! operand), which
//! becomes the
//! left-hand side. Then it loops: if the
//! next token is a binary operator whose left binding power is at least the
//! minimum, it consumes the operator, parses the right-hand side by calling
//! itself with the operator's right binding power as the new minimum, and
//! folds the result into a new left-hand side. Otherwise the operator
//! belongs to an enclosing call, and the loop stops.
//!
//! A literal is a number, a duration or a size, a string, `true` or `false`,
//! each one token. A number with a name glued to it, `30sec`, which the
//! lexer leaves a number before any name that is no unit's (see the lexer),
//! is "unknown unit `sec`" where an expression is expected, noted with the
//! units there are (`Parser::unitless`); a module path or a definition's
//! path takes such a run of glued tokens as its segment.
//!
//! Precedence and associativity both fall out of the binding-power pairs:
//!
//! | operator                    | left bp | right bp / operand min bp  |
//! |-----------------------------|---------|----------------------------|
//! | `or`                        | 1       | 2                          |
//! | `and`                       | 3       | 4                          |
//! | prefix `not`                | (none)  | operand parsed with min 5  |
//! | `==` `!=` `===` `!==`       | 5       | 6 (non-associative)        |
//! | `<` `<=` `>` `>=`           | 5       | 6 (non-associative)        |
//! | postfix `is` TYPE           | 5       | (none: non-associative)    |
//! | `..` `..=`                  | 6       | 7 (non-associative)        |
//! | `+` `-`                     | 7       | 8                          |
//! | `*` `/` `//` `%`            | 9       | 10                         |
//! | postfix `:` TYPE, `as` TYPE | 11      | (none: parses a type)      |
//! | prefix `-` and `await`      | (none)  | operand parsed with min 13 |
//! | postfix `.` NAME            | 15      | (none: takes a name)       |
//! | postfix call `(`...`)` `{`  | 15      | (none: arguments at min 0) |
//! | postfix index `[`...`]`     | 15      | (none: position at min 0)  |
//!
//! A tighter operator has higher powers, so in `1 + 2 * 3` the recursive
//! call for the right-hand side of `+` (minimum 8) still accepts `*` (left
//! power 9). Left associativity comes from making the right power one higher
//! than the left: in `8 - 3 - 2`, the right-hand side of the first `-` is
//! parsed with minimum 8, which rejects the second `-` (left power 7), so the
//! first subtraction completes before the second begins.
//!
//! Comparison is non-associative rather than left-associative: its right
//! operand stops before another comparison as usual, and the parser then
//! reports "comparison operators cannot be chained" if one follows, since
//! `a < b < c` would otherwise mean `(a < b) < c`.
//!
//! The ranges, `..` and `..=`, bind tighter than comparison and looser than
//! `+` and `-`: their left power is comparison's right power, so the right
//! operand of `==` takes in a range, and their right power is the left
//! power of `+` and `-`, so a range's right operand takes in a sum and
//! `0..len(xs) - 1` is `0..(len(xs) - 1)`, while `a..b == c` is `(a..b) ==
//! c`. They are non-associative as comparison is: a range directly
//! followed by another, `a..b..c` or `a..b..=c`, is "`..` cannot be
//! chained" (or "`..=` cannot be chained"), at the second operator.
//! Negation binds tighter, so `-1..1` is `(-1)..1`.
//!
//! `not` is a prefix operator whose operand is parsed with minimum 5, the
//! left power of comparison, so it takes in a comparison but not `and` or
//! `or`: `not a == b` is `not (a == b)`, and `not a and b` is
//! `(not a) and b`.
//!
//! Type ascription `EXPR : TYPE` is a postfix operator in the same loop: when
//! the next token is `:` and its left power 11 is at least the minimum, the
//! parser consumes it, parses a type, and wraps the left-hand side. It binds
//! tighter than every binary operator, so `1 + 2 : Float` is
//! `1 + (2 : Float)`, and looser than negation, whose operand is parsed with
//! minimum 13, so `-5 : Float` is `(-5) : Float`. It chains left to right.
//!
//! `EXPR as TYPE`, a checked narrowing, is a postfix operator binding
//! exactly as the ascription does, chaining with it left to right: `-x as
//! Int` is `(-x) as Int`, and `a + b as Int` is `a + (b as Int)`. `EXPR is
//! TYPE`, a type test, is a postfix operator binding as a comparison does:
//! its left power is comparison's, so `x + 1 is Int` is `(x + 1) is Int`,
//! `not x is Int` is `not (x is Int)` and `x is Int and y` is `(x is Int)
//! and y`; and it does not chain with a comparison or another `is`, either
//! way round, so `a == b is Int` and `x is Int == y` are "comparison
//! operators cannot be chained" (`Parser::not_chained`). Each parses a
//! whole type after its keyword, a union included, so `x is Int | String`
//! tests for the union. `is` and `as` are keywords (see [`crate::lexer`]).
//!
//! Field access `EXPR.NAME` is a postfix operator too, binding tighter than
//! everything: its left power 15 is above every minimum, so a `.` always
//! applies to the operand just parsed. `-a.b` is `-(a.b)`, `a.b : Int` is
//! `(a.b) : Int`, and `a.b.c` is `(a.b).c`. The parser does not skip
//! newlines around `.`, so a `.` starting a line is not a continuation.
//!
//! A call binds exactly as tightly as field access, in the same loop, so
//! `a.b(1).c` is `((a.b)(1)).c`, `-f(1)` is `-(f(1))` and `f(1)(2)` calls
//! the result of `f(1)`. Each argument is parsed at minimum 0. So does an
//! index, whose position is parsed at minimum 0 too.
//!
//! Negation is its own token, [`TokenKind::Neg`], distinct from subtraction's
//! [`TokenKind::Minus`]; the lexer tells them apart by spacing. `Neg` is only
//! accepted in prefix position and `Minus` only as a binary operator.
//!
//! An `if` is a prefix expression: `if`, a condition parsed at minimum 0, a
//! block, and optionally `else` followed by a block or another `if`. An `if`
//! without `else` gets an empty block as its else branch, allocated by the
//! parser with an empty span at the end of the `if`'s block.
//!
//! A `for` is a prefix expression too, usable wherever an `if` is, and so
//! is a `do`: see [Loops](#loops).
//!
//! An interpolated string arrives from the lexer as a
//! [`TokenKind::StrStart`], each expression's tokens, a
//! [`TokenKind::StrMid`] between each two expressions, and a
//! [`TokenKind::StrEnd`]. The parser parses each expression at minimum 0
//! and collects the parts on a stack shared by all the interpolations being
//! parsed, as it does for items, so nested interpolations need no
//! allocation of their own.
//!
//! # Errors and recovery
//!
//! Errors are values. Parsing functions return `Result`, and a failure is
//! recorded in the parser's diagnostic sink before the `Err` propagates, up
//! to the nearest *recovery point*, which puts back what the failed parse
//! made (the parser's stacks and every arena of the tree, so that nothing
//! of it is left in the tree, `Parser::restore`), skips the source to a
//! *synchronisation point* and goes on. [`parse_all`] returns the tree so
//! recovered, with every error; [`parse`] returns the first error, as it
//! always did, or the tree if there is none.
//!
//! The recovery points, innermost first:
//!
//! - **An entry of a list**: an argument of a call, a parameter of a
//!   function expression, of a function type or of a definition made with
//!   a def-type, an element of a list literal, an entry of a record type.
//!   The list skips to the `,` after the entry and goes on with the next,
//!   or to its closing bracket, and ends. Each later entry is parsed as any
//!   is, its errors reported. A list with an error fails once it has ended,
//!   so its errors are reported and its construct (the call, the function,
//!   the list, the type) is no part of the tree.
//! - **The expression of an interpolation**: the string skips to the piece
//!   after it and goes on, to fail at its end, as a list does.
//! - **The value of an item of a body**, after the `=` of a `let`, a field
//!   or a result, the `:=` of a rebind or the `...` of a spread, the end of
//!   the item checked with it (`Parser::parse_value`): the value is an
//!   error node, [`ExprKind::Error`], spanning what is skipped to the end
//!   of the item, so the item still declares what it declares. Not in a
//!   `for` header, whose parts' errors are the header's.
//! - **An item of a body**, the file's or a block's: it is an error item,
//!   [`ItemKind::Error`], spanning what is skipped to the end of the item
//!   (`Parser::error_item`). So is the item after an attribute that is an
//!   error, which may have meant something else for it, and attributes
//!   waiting for an item that fails are dropped.
//!
//! Two constructs an editor leaves half-typed are kept, for a language
//! server, which completes what is typed after them, rather than failing
//! what they are in: their error is reported as it was, and the parse goes
//! on. A field access with no name after its `.`, `a.`, where what follows
//! may end an expression in the innermost bracket open
//! (`Parser::ends_unfinished`: the end of a line or of the input, a `;`, a
//! `}` of a block, the `)` or `]` of the bracket open, or a `,` in one) is a
//! [`ExprKind::NoField`] of its record, which is checked as any expression
//! is; where anything else follows, it fails as before. A call whose
//! argument list ends where no `)` is, at the end of the input or at the
//! `}` of the block around it, which closes the `(` too, is an
//! [`ExprKind::Unclosed`] of its callee and of the arguments it parsed (an
//! argument that failed leaves nothing of it), from its callee through the
//! last token it went on to; since the lexer emits no newline inside a
//! `(`, that is the rest of the file, or of the block, each line of it
//! parsed as an argument (`q = 2` a named one). A list that ends at its
//! `)` with an error fails as before. The bracket of a list or of
//! parentheses whose `,`, `)` or `]` is missing at the token a `.` with no
//! name was reported at, the end of the input or a `}`, is not reported
//! again (`Parser::report_close`).
//!
//! An item, or a value, that fails having gone on past a newline, as it
//! does after an operator, `=`, `:` or `let`, to a line that is an item of
//! its own (`Parser::item_line`: a `let`, `fn NAME`, `def`, `import`,
//! `type`, `defmeta`, `return`, `break`, `continue`, an attribute, or a
//! path followed by `=`, `:=` or an annotation and `=`), with no bracket
//! opened in it left open, is forgotten, its errors with it, and parsed
//! again with that newline its end (`Parser::rewind`), so that the next
//! line is the next item, as an editor's half-typed `x = ` above it leaves
//! it: its error is then the one at that newline, "expected expression,
//! found newline". A line that may go on the line before, an operand, is
//! not one, and stays the item's. Nor is a line inside a bracket left
//! open, a `(` or `[`: the lexer emits no newline there, and the rest of
//! the source is in the bracket.
//!
//! A skip (`Parser::skip`) follows the brackets the lexer follows (`nest`):
//! it goes over everything between brackets opened since what failed
//! started, newlines inside braces included, and stops, without consuming
//! it, at the synchronisation point with nothing opened since then left
//! open: an item's newline or `;`; a `}` that closes a block's body; an
//! entry's `,`, or the list's closing bracket; the string's piece after an
//! interpolation; or the end of the input. A closing bracket of something
//! opened before what failed that is not the point is something around a
//! list (a `}`, or a string's piece, which closes the list too), where the
//! list ends, or one the lexer ignores, as a `)` or `]` of nothing is,
//! skipped. In the file's body a `}` closes nothing, and is skipped. An
//! item starts at none of the points, so it, and every loop, always moves
//! on; a value may be empty, `x = ;`. Nesting too deep is recovered from by the item or its value
//! alone, not by every list it is in, which would skip what is nested in
//! each of them again.
//!
//! What is reported: each error, once, where it is found. Nothing skipped
//! is reported: it may be part of the error. A list that has failed reports
//! what is wrong in its later entries, and the end of the input where its
//! closing bracket should be. Nothing is reported at a lexical error's
//! token, [`TokenKind::Error`], which the lexer has reported: what it is in
//! fails silently, as an expression, a type, a separator; nor after a
//! lexical error whose token runs to the end of the input (an unterminated
//! string literal), after which nothing was lexed. A block whose body meets
//! the end of the input ends there, "expected `}`", reported once, for the
//! innermost block; a `}` in the file's body that closes nothing is
//! "unexpected `}`", and skipped. The errors of one file, the lexer's and
//! the parser's together, are capped at [`MAX_ERRORS`]: after them the
//! parser skips to the end of the input, and the next is replaced by "too
//! many errors" ([`capped`]).
//!
//! What the tree says: [`Ast::syntax`] is [`Syntax::Clean`] for a tree with
//! no error, [`Syntax::Recovered`] for one with errors, each in an error
//! node or left out of the tree (a failed list's construct, a failed
//! interpolated string), and [`Syntax::Unbalanced`] for one whose braces
//! did not match, a `}` that closes nothing or a block that the end of the
//! input closed, where items may be in the wrong block: the workspace does
//! not check that one. A recovered tree is checked, an error node being of
//! type `Error`, silently, and a block with an error item erroneous, as is
//! a name that names nothing in it (see the workspace's [Syntax
//! errors](crate::workspace#syntax-errors)); it is never lowered.
//!
//! # Depth
//!
//! The parser enforces [`MAX_DEPTH`] in two places, because nesting arises in
//! two ways. Parentheses, prefix operators (`await` included), blocks, `if`s,
//! `for`s, interpolations, function expressions, `lazy`s, argument lists, list
//! literals, indexes, and
//! record, function, applied and parenthesised types nest by recursion, so
//! [`Parser::parse_expr`], the body of every nested block, every `else if`
//! and every record, function, applied and parenthesised type count their
//! own depth and stop before the recursion itself could overflow; a
//! function expression, a `lazy`, a `for`, an argument list, a list literal
//! and an index recurse only through those.
//! Left-associative chains such as `1 + 1 + 1`, `a.b.c`, `xs[0][0]`, a
//! union of many types or `Int[][]` nest without recursion, since the loop
//! folds each operator into the
//! left-hand side, so the parser also tracks the height of every node it
//! allocates, a block being one above the tallest of its items' expressions
//! and types (a `break`'s, `continue`'s or `return`'s value among them), an `if` or an
//! interpolation one above its tallest child, a
//! `for` one above the tallest of its iterable, its guard and its body, a
//! condition loop one above the tallest of its header's items, its
//! condition and its body, a `do` loop one above the taller of its body and
//! its condition, a
//! list literal one above its tallest element (a spread's being its
//! expression's), an index one above the taller of its list and its
//! position, a
//! field access one above its record, an ascription, an `is` or an `as`
//! one above the taller of its expression and its type (a record type
//! being one above its tallest
//! field type or field default, a function type one above the tallest of its parameter and
//! return types, an applied type one above its argument, a `T[]` one above
//! `T`, a parenthesised
//! type one above the type inside, though it leaves no node of its own, and
//! a union one above the taller of its two sides), a function expression
//! one above the tallest of its parameter types, its return type and its
//! body, a `lazy`
//! one above its operand, and a call one above the tallest of its callee and
//! arguments, and rejects a node that would exceed the limit. An item's
//! height is the taller of its value's and its path's index positions',
//! plus two levels for each segment of its path after the first, or its
//! annotation's if that is taller; an item taller than the limit is
//! rejected too, at the item. A position is parsed by
//! `Parser::parse_expr`, so it counts its own depth as any expression
//! does. Together these guarantee that no tree handed to a later stage is
//! deeper than `MAX_DEPTH`.

use crate::MAX_DEPTH;
use crate::ast::{
    Arg, ArgKind, Ast, AstBuilder, BinaryOp, BuilderMark, DefTypeRole, DefTypeUse, Effect, Element,
    Expr, ExprId, ExprKind, FieldDefault, FieldTy, FieldTyKind, FnDecl, ForDecl, ForHead,
    ImportDecl, ImportForm, ImportMember, ImportName, Item, ItemId, ItemKind, ItemRange, Jump,
    Label, MetaTy, Param, ParamTy, PathRange, PathSegment, SegmentKind, StrPart, Syntax, TypeExpr,
    TypeExprId, TypeExprKind, UnaryOp, path_span,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::{self, MAX_ERRORS, Token, TokenKind, lex};
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span};
use crate::strings::{StrId, Strings};

/// Minimum binding power of the operand of prefix `not`: the left power of
/// comparison, so the operand takes in a comparison but not `and` or `or`.
const NOT_BP: u8 = 5;

/// Minimum binding power of the operand of prefix `-`: tighter than every
/// binary operator and than ascription.
const PREFIX_BP: u8 = 13;

/// Left binding power of postfix `:` (type ascription) and of postfix `as`:
/// tighter than every binary operator, looser than prefix `-`.
const ASCRIBE_BP: u8 = 11;

/// Left binding power of postfix `is` (a type test): the left power of
/// comparison, which it binds as.
const IS_BP: u8 = 5;

/// Left binding power of postfix `.` (field access): tighter than
/// everything, prefix `-` included.
const FIELD_BP: u8 = 15;

/// Height of a leaf node, and of a type written as a name.
const LEAF_HEIGHT: u32 = 1;

/// The error for a raw or interpolated string literal where a field name is
/// expected.
const PLAIN_FIELD_NAME: &str = "a field name must be a plain string literal";

/// The error for a string literal where a variable's name is expected.
const PLAIN_VARIABLE_NAME: &str = "variables need plain names";

/// The error for a quoted or an index segment of a `def`'s path, at the
/// segment.
const DEF_PATH: &str = "a `def`'s path is made of identifiers";

/// The error for a `def` named by a string, at the string.
const DEF_STRING_NAME: &str = "a `def` is named by an identifier, not a string";

/// The error for a function, or a function type, without a return marker.
const MISSING_RETURN_TYPE: &str =
    "expected `:`, `->` or `->>` and a return type after the parameters";

/// The error for a `...` that starts neither an element of a list literal
/// nor an item, where an expression starts. (A spread item is recognised
/// by `Parser::parse_item` before any expression is parsed, so it never
/// reaches this.)
const MISPLACED_SPREAD: &str = "`...` is only allowed in a list or at the start of an item";

/// The error for an `import` item anywhere but in a file's outermost body.
const MISPLACED_IMPORT: &str = "`import` is only allowed at the top level of a file";

/// The contextual keyword that begins a `type` item.
const TYPE: &str = "type";

/// The error for a `type` item anywhere but in a file's outermost body.
const MISPLACED_TYPE: &str = "`type` is declared only at the top level of a file";

/// The error for type parameters after a `type` item's name, at their `<`.
const TYPE_PARAMETERS: &str = "a `type` takes no type parameters";

/// The name of the outer attribute that marks a declaration shadowable.
const SHADOWABLE: &str = "shadowable";

/// The name of the inner attribute that turns the implicit prelude off.
const NO_PRELUDE: &str = "no_prelude";

/// The name of the outer attribute that makes a `def` a def-type.
const DEF_TYPE: &str = "def_type";

/// The error for `#[def_type]` before anything but a top-level `def`.
const MISPLACED_DEF_TYPE: &str = "`#[def_type]` applies only to a file's top-level `def`s";

/// The error for a definition made with a def-type anywhere but in a file's
/// outermost body.
const MISPLACED_DEF_TYPE_DEF: &str =
    "a def-type definition is declared only at the top level of a file";

/// The text of the name a def-type reference's read of its definition
/// starts with until the workspace names the def-type's namespace: no
/// program can write it, `<` being in no identifier. A definition's read of
/// its meta starts with it, and its field access names it, until the
/// workspace names the namespace of the metas and the meta's leaf.
const UNRESOLVED_NAMESPACE: &str = "<def-type>";

/// The contextual keyword that begins a definition's meta (see
/// [Def-types](#def-types)).
const DEFMETA: &str = "defmeta";

/// The error for a definition's meta anywhere but in a file's outermost
/// body.
const MISPLACED_DEFMETA: &str = "a `defmeta` is declared only at the top level of a file";

/// The name that, after a function type's return type and followed by `:`,
/// `->` or `->>`, begins its meta (see [Types](#types)).
const META: &str = "META";

/// The error for a meta's marker anywhere but after a function type's
/// return type.
const MISPLACED_META: &str = "a meta is written only in a function type";

/// The error for a parameter's default, `= EXPR`, in a function type, at
/// its `=`: only a function expression's parameter has one.
const MISPLACED_DEFAULT: &str = "a default is written only in a function expression's parameters";

/// The error for `#[shadowable]` before anything but a top-level field,
/// `def` or `import`.
const MISPLACED_SHADOWABLE: &str =
    "`#[shadowable]` applies only to a file's top-level fields, `def`s and imports";

/// The note on [`MISPLACED_SHADOWABLE`] before a `let`, `let mut` or `fn`.
const LET_SHADOWS_NOTE: &str = "a `let` or `fn` already shadows";

/// The error for `#![no_prelude]` anywhere but before the file's first
/// item.
const MISPLACED_NO_PRELUDE: &str = "`#![no_prelude]` must come before anything else in the file";

/// The error for a module path written as a string.
const QUOTED_MODULE_PATH: &str =
    "a module path is written without quotes, its segments separated by `/`";

/// The error for an empty segment of a module path, at its `/`.
const EMPTY_MODULE_SEGMENT: &str = "expected a module name before and after each `/`";

/// What a module path's segment may be made of, for the error when one is
/// not.
const MODULE_SEGMENT: &str =
    "a segment starts with a letter, a digit or `_`, ends with one or `+`, and has no `..`";

/// The error for a member of a `from` import that is not a single name.
const SINGLE_MEMBER: &str = "a name imported with `from` is a single name";

/// The error for a `mut` anywhere but directly after `let`.
const MISPLACED_MUT: &str = "`mut` is only allowed after `let`";

/// The error for a `let mut` whose path is dotted.
const DOTTED_MUT: &str = "`let mut` needs a plain name";

/// The note on [`DOTTED_MUT`].
const DOTTED_MUT_NOTE: &str = "a dotted `let` updates a variable; declare the variable with `let \
     mut a = ...` and update it with `a.b := ...`";

/// The error for an index segment in the path of a `let`.
const LET_INDEX: &str = "a `let` path cannot index a list";

/// The error for an index segment in the path of a field item.
const FIELD_INDEX: &str = "a field's path cannot index a list";

/// The error for a label anywhere but directly after the keyword of what it
/// belongs to.
const MISPLACED_LABEL: &str = "a label names a loop or a block: it follows `for`, `do`, `break`, \
     `continue`, `return` or `{`";

/// The error for a label on a record block, which nothing can leave by
/// it.
const LABELLED_RECORD: &str = "a record block cannot be labelled";

/// The error for a `do` loop whose body is not followed by `for`.
const MISSING_DO_FOR: &str = "expected `for` and a condition after the body of a `do`";

/// The error for anything but a condition after the `for` of a `do` loop.
const DO_CONDITION_ONLY: &str = "only a condition follows the `for` of a `do`";

/// The error for a `{` where an expression starts in a `for` header.
const HEADER_BRACE: &str = "a `{` in a `for` header begins its body";

/// The note on [`HEADER_BRACE`].
const HEADER_BRACE_NOTE: &str =
    "put a block in a `for` header in parentheses, as in `for let r = ({ a = 1 }); ...`";

/// The note on [`MISPLACED_MUT`] before a field.
const MUT_FIELD_NOTE: &str = "a field is defined once; declare a variable with `let mut x = ...` \
     and define the field from it";

/// The note on [`MISPLACED_MUT`] before a parameter.
const MUT_PARAM_NOTE: &str = "copy the parameter into a mutable variable, as in `let mut n = n`";

/// The note on [`MISSING_RETURN_TYPE`].
const RETURN_TYPE_NOTE: &str = "a function declares its return type: `: T` for a pure function, `-> T` for an impure one, \
     `->> T` for a memoised one";

/// Parses `tokens`, lexed from `source` with its names interned into
/// `names` and its string literals' text stored in `strings`, as a file: a
/// body of items. The returned [`Ast`] owns `names` and `strings`.
///
/// All tokens must be consumed. `tokens` should end with
/// [`TokenKind::Eof`], as [`crate::lexer::tokenize`] guarantees; if it does
/// not, running out of tokens is treated as end of input.
///
/// # Errors
///
/// Returns the first syntax error, or, for tokens with a
/// [`TokenKind::Error`] and no syntax error, "invalid token" at the first.
/// [`parse_all`] gives every error, with the tree recovered from them.
pub fn parse(
    tokens: &[Token],
    mut names: Names,
    mut strings: Strings,
    source: &SourceFile,
) -> Result<Ast, Diagnostic> {
    let (builder, root, diagnostics) = parse_parts(tokens, source, (&mut names, &mut strings), 0);
    if let Some(first) = diagnostics.into_iter().next() {
        return Err(first);
    }
    if let Some(token) = tokens.iter().find(|token| token.kind == TokenKind::Error) {
        return Err(Diagnostic::error("invalid token", token.span));
    }
    Ok(builder.finish(root, names, strings))
}

/// Parses `tokens` as [`parse`] does, but going on past every syntax error
/// (see [Errors and recovery](#errors-and-recovery)): returns the tree,
/// with an error node in place of each part of the source that could not
/// be parsed ([`Ast::syntax`] says whether it has any), and every error,
/// `lexical`, the lexer's ([`crate::lexer::lex`]), among them, in source
/// order and capped ([`capped`]).
#[must_use]
pub fn parse_all(
    tokens: &[Token],
    mut names: Names,
    mut strings: Strings,
    source: &SourceFile,
    lexical: Vec<Diagnostic>,
) -> (Ast, Vec<Diagnostic>) {
    let (builder, root, syntax) =
        parse_parts(tokens, source, (&mut names, &mut strings), lexical.len());
    (
        builder.finish(root, names, strings),
        capped(lexical, syntax),
    )
}

/// Lexes and parses `source` going on past every error ([`lex`],
/// [`parse_all`]): its tree, recovered from the errors, and every lexical
/// and syntax error, in source order and capped.
#[must_use]
pub fn parse_source(source: &SourceFile) -> (Ast, Vec<Diagnostic>) {
    let mut names = Names::new();
    let lexed = lex(source, &mut names);
    parse_all(
        &lexed.tokens,
        names,
        lexed.strings,
        source,
        lexed.diagnostics,
    )
}

/// The errors of one source, its lexer's, `lexical`, and its parser's,
/// `syntax`, each in the order found, merged in source order (a lexical
/// error first among those that start at one place) and capped: past
/// [`MAX_ERRORS`], the first after them is replaced by "too many errors",
/// at it, and the rest are left out.
#[must_use]
pub fn capped(lexical: Vec<Diagnostic>, syntax: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = lexical;
    all.extend(syntax);
    all.sort_by_key(|diagnostic| diagnostic.span.start);
    if all.len() > MAX_ERRORS {
        all.truncate(MAX_ERRORS + 1);
        if let Some(last) = all.last_mut() {
            *last = Diagnostic::error("too many errors", last.span).with_note(format!(
                "only the first {MAX_ERRORS} errors of this file are reported"
            ));
        }
    }
    all
}

/// Like [`parse_all`], returning the unfinished arenas and the file's body
/// instead of an [`Ast`], and borrowing the tables rather than taking them,
/// so that a caller lending its [`Names`] hands them over only once the
/// tree is finished, with the parser's errors alone, `lexical` being the
/// number of the lexer's. The text of every quoted field name is interned
/// into `names`, those of source that could not be parsed among them. The
/// source text of the position of every index segment of a path is added
/// to `strings` (see [`crate::ast::SegmentKind::Index`]).
pub(crate) fn parse_parts(
    tokens: &[Token],
    source: &SourceFile,
    (names, strings): (&mut Names, &mut Strings),
    lexical: usize,
) -> (AstBuilder, ItemRange, Vec<Diagnostic>) {
    // A lexical error whose token runs to the end of the source, an
    // unterminated string literal, leaves nothing after it to parse: what
    // is missing there is no error of its own.
    let quiet_from = match tokens {
        [.., last, eof]
            if last.kind == TokenKind::Error
                && eof.kind == TokenKind::Eof
                && last.span.end == source.len() =>
        {
            last.span.start
        }
        _ => u32::MAX,
    };
    let mut parser = Parser {
        tokens,
        pos: 0,
        source,
        names,
        strings,
        // There are never more nodes than tokens, so these never
        // reallocate: every node consumes a token of its own except the
        // empty else block of an `if` without `else`, and that `if`'s own
        // block consumes two tokens, `{` and `}`, for one node; and a call
        // with only a trailing block, whose block likewise consumes two.
        // (A field access consumes two, `.` and the name, a call with an
        // argument list its `(` and `)`, and a function expression at least
        // `fn`, `(`, `)`, a marker and a type besides its body.) An error
        // node takes the place of the nodes of the source it stands for,
        // which is at least the token it is at, but at the end of the input.
        builder: AstBuilder::with_capacity(tokens.len()),
        heights: Vec::with_capacity(tokens.len()),
        depth: 0,
        item_stack: Vec::new(),
        part_stack: Vec::new(),
        field_ty_stack: Vec::new(),
        param_stack: Vec::new(),
        param_ty_stack: Vec::new(),
        arg_stack: Vec::new(),
        element_stack: Vec::new(),
        path: Vec::new(),
        braces: Braces::Free,
        recovery: Recovery::Body { top: true },
        quiet_from,
        unclosed: false,
        deep: false,
        line_end: None,
        diagnostics: Vec::new(),
    };
    if lexical > 0 {
        parser.builder.set_syntax(Syntax::Recovered);
    }
    let (root, _) = parser.parse_body(None);
    (parser.builder, root, parser.diagnostics)
}

/// What an item's height needs to know of its path: its number of segments,
/// each after the first adding two levels, and the height of its tallest
/// index position, 0 if it has none.
#[derive(Clone, Copy, Debug)]
struct PathShape {
    /// The number of segments.
    segments: u32,
    /// The height of the tallest position of an index segment, or 0.
    positions: u32,
}

impl PathShape {
    /// The shape of a single name, and of an item without a path.
    const SINGLE: PathShape = PathShape {
        segments: 1,
        positions: 0,
    };
}

/// The outer attributes read before an item of a body, waiting for it (see
/// [Attributes](#attributes)): where each is written, if it is.
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    /// `#[shadowable]`.
    shadowable: Option<Span>,
    /// `#[def_type]`.
    def_type: Option<Span>,
}

impl Pending {
    /// Whether no attribute is waiting.
    const fn is_empty(&self) -> bool {
        self.shadowable.is_none() && self.def_type.is_none()
    }

    /// The first attribute written of those waiting, by its name and where
    /// it is written.
    fn first(&self) -> Option<(&'static str, Span)> {
        [(SHADOWABLE, self.shadowable), (DEF_TYPE, self.def_type)]
            .into_iter()
            .filter_map(|(name, span)| Some((name, span?)))
            .min_by_key(|&(_, span)| span.start)
    }
}

/// What a `{` means where the parser is (see [Trailing
/// blocks](#trailing-blocks)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Braces {
    /// A `{` after an expression on its line starts a trailing block, and
    /// one where an expression starts, a block.
    Free,
    /// A `{` after an expression never starts a trailing block, since it
    /// begins the block of an `if`, or the body of a `for`; one where an
    /// expression starts is a block.
    NoTrailing,
    /// As [`Braces::NoTrailing`], and a `{` where an expression starts is
    /// the body too, and so an error: in the header of a `for`.
    Header,
}

/// Proof that an error is on record: pushed to the parser's sink, or, for
/// one at a [`TokenKind::Error`], the lexer's. Only [`Parser::report`]
/// creates one, and [`Parser::report_close`], for an error on record at its
/// place already.
#[derive(Debug)]
struct Reported;

type ParseResult<T> = Result<T, Reported>;

/// How an argument of a call failed: its value, or the separator after it,
/// the argument parsed.
#[derive(Debug)]
enum Entry {
    /// The value.
    Value(Reported),
    /// The separator after it.
    Separator(Reported),
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    source: &'a SourceFile,
    /// Where quoted field names are interned.
    names: &'a mut Names,
    /// The text of the string literals, for quoted field names, to which the
    /// source text of each index segment's position is added.
    strings: &'a mut Strings,
    builder: AstBuilder,
    /// The height of each allocated expression node, indexed like the
    /// builder's arena: 1 for a leaf, one more than the tallest child
    /// otherwise. A block's children are its items' expressions.
    heights: Vec<u32>,
    /// Current recursion depth: nested [`Parser::parse_expr`] calls and
    /// nested block bodies.
    depth: usize,
    /// The items of the bodies being parsed, innermost last. Each body
    /// pushes its items here as it parses them and moves them to the
    /// builder's item-list pool when it ends, so nested bodies need no
    /// allocation of their own.
    item_stack: Vec<ItemId>,
    /// The parts of the interpolated strings being parsed, innermost last,
    /// moved to the builder's part pool as each string ends, as for
    /// `item_stack`.
    part_stack: Vec<StrPart>,
    /// The fields of the record types being parsed, innermost last, moved
    /// to the builder's field-type pool as each type ends, as for
    /// `item_stack`.
    field_ty_stack: Vec<FieldTy>,
    /// The parameters of the function expressions being parsed, moved to
    /// the builder's parameter pool as each parameter list ends, as for
    /// `item_stack`.
    param_stack: Vec<Param>,
    /// The parameters of the function types being parsed, innermost last,
    /// moved to the builder's parameter-type pool as each type ends, as for
    /// `item_stack`.
    param_ty_stack: Vec<ParamTy>,
    /// The arguments of the calls being parsed, innermost last, moved to
    /// the builder's argument pool as each call ends, as for `item_stack`.
    arg_stack: Vec<Arg>,
    /// The elements of the list literals being parsed, innermost last,
    /// moved to the builder's element pool as each literal ends, as for
    /// `item_stack`.
    element_stack: Vec<Element>,
    /// The segments of the paths being read, innermost last, moved to the
    /// builder's path pool as each path is complete, as for `item_stack`:
    /// the position of an index segment may hold a block, whose items read
    /// paths of their own.
    path: Vec<PathSegment>,
    /// What a `{` means where the parser is: whether one after an
    /// expression is kept from starting a trailing block, as it is while
    /// parsing the condition of an `if`, so that its block is not taken for
    /// one, and, in the header of a `for`, whether one that starts an
    /// expression is the body too. Lifted inside parentheses, argument
    /// lists, list literals, indexes, blocks and interpolations.
    braces: Braces,
    /// Whether the value of an item being parsed recovers from an error in
    /// it, as a body's items' do, and in which body (see [Errors and
    /// recovery](#errors-and-recovery)).
    recovery: Recovery,
    /// Where the lexical error whose token runs to the end of the source
    /// starts, if one does: no error is reported from there on, since
    /// nothing after it was lexed. `u32::MAX` if none does.
    quiet_from: u32,
    /// Whether the end of the input inside a block has been reported: it is
    /// reported once, at the innermost block.
    unclosed: bool,
    /// Whether the error being recovered from is nesting too deep, which
    /// no list and no interpolation recovers from: the item, or its value,
    /// does.
    deep: bool,
    /// The newline token, by its index, past which no newline is skipped:
    /// set while an item that failed is parsed again up to the end of its
    /// line ([`Parser::item_line`]).
    line_end: Option<usize>,
    /// Errors reported so far, in the order found: at most [`MAX_ERRORS`]
    /// and one more, after which the parser skips to the end of the input.
    diagnostics: Vec<Diagnostic>,
}

/// Whether the value of an item recovers from an error in it (see [Errors
/// and recovery](#errors-and-recovery)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recovery {
    /// It does, as an item of a body does: of the file's body if `top`, of
    /// a block's otherwise.
    Body {
        /// Whether the body is the file's.
        top: bool,
    },
    /// It does not, as a part of a `for` header does not: its error is its
    /// header's.
    None,
}

/// Where the parser's stacks and the builder's arenas were, with its depth
/// and what a `{` meant: what a recovery point puts back when what it
/// parses fails, so that nothing of the source it skips is left in the tree
/// (see [Errors and recovery](#errors-and-recovery)).
#[derive(Clone, Copy, Debug)]
struct Marks {
    builder: BuilderMark,
    heights: usize,
    items: usize,
    parts: usize,
    field_tys: usize,
    params: usize,
    param_tys: usize,
    args: usize,
    elements: usize,
    path: usize,
    depth: usize,
    braces: Braces,
    /// The number of errors reported, the tree's syntax and whether the end
    /// of the input in a block was reported: what [`Parser::rewind`], but
    /// not [`Parser::restore`], puts back.
    diagnostics: usize,
    syntax: Syntax,
    unclosed: bool,
}

/// What ends a skip over source that could not be parsed (see [Errors and
/// recovery](#errors-and-recovery)).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sync {
    /// An item of a body, or its value: a newline or `;` ends it, and so
    /// does a `}` that closes the body, which is a block's but if `top`.
    Item {
        /// Whether the body is the file's, which no `}` closes.
        top: bool,
    },
    /// An entry of a list that `close` closes: a `,` or `close` ends it.
    Entry {
        /// The token that closes the list.
        close: TokenKind,
    },
    /// The expression of an interpolation: the string's piece after it
    /// ends it.
    Interpolation,
}

/// Where a skip stopped, at a token it did not consume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// At the separator after an item or an entry: a newline, a `;` or a
    /// `,`.
    Separator,
    /// At what closes the body, the list or the interpolation.
    Close,
    /// At the end of the input, or at what closes something around the list.
    Outer,
}

/// A bracket opened in the source a skip goes over: what the lexer keeps
/// on its stack, which a skip follows (see [`nest`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    /// `(`
    Paren,
    /// `[`
    Square,
    /// `{`
    Brace,
    /// The start of an interpolated string, which its last piece closes.
    Str,
}

impl Parser<'_> {
    /// Parses a body up to its end: the `}` matching `open` (not consumed),
    /// or the end of input for the file's body, when `open` is `None`.
    /// Returns the body and the height of its tallest item expression (0 if
    /// it has no items).
    ///
    /// An item that fails is an error item, spanning it and what is skipped
    /// after it to the item's end (see [Errors and
    /// recovery](#errors-and-recovery)), and so is the item after an
    /// attribute that is an error. A block's body ends at the end of the
    /// input too, which is reported once, "expected `}`", for the innermost
    /// block; and a `}` in the file's body is reported, "unexpected `}`",
    /// and skipped. Either makes the tree [`Syntax::Unbalanced`].
    fn parse_body(&mut self, open: Option<Span>) -> (ItemRange, u32) {
        let base = self.item_stack.len();
        let mut height = 0;
        let top = open.is_none();
        let recovery = std::mem::replace(&mut self.recovery, Recovery::Body { top });
        // The outer attributes waiting for the item they apply to, and
        // whether one was an error, which makes it an error item.
        let mut pending = Pending::default();
        let mut poisoned = false;
        loop {
            self.skip_separators();
            let token = self.peek();
            if self.diagnostics.len() > MAX_ERRORS {
                // Too many errors: the rest is not parsed.
                self.pos = self.tokens.len().max(self.pos);
                break;
            }
            match token.kind {
                TokenKind::Attribute(name) => {
                    if self.outer_attribute(token, name, &mut pending).is_err() {
                        self.advance();
                        poisoned = true;
                    }
                    continue;
                }
                TokenKind::InnerAttribute(name) => {
                    let first =
                        open.is_none() && self.item_stack.len() == base && pending.is_empty();
                    if self.inner_attribute(token, name, first).is_err() {
                        self.advance();
                        poisoned = true;
                    }
                    continue;
                }
                _ => {}
            }
            match (token.kind, open) {
                (TokenKind::Eof, None) | (TokenKind::RBrace, Some(_)) => {
                    if !poisoned && let Some((written, attribute)) = pending.first() {
                        self.error(
                            format!("`#[{written}]` is not followed by anything"),
                            attribute,
                        );
                    }
                    break;
                }
                (TokenKind::Eof, Some(open)) => {
                    self.builder.set_syntax(Syntax::Unbalanced);
                    if !self.unclosed {
                        self.unclosed = true;
                        let found = self.describe(token);
                        self.report(
                            Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                                .with_context(open, "unclosed `{`"),
                        );
                    }
                    break;
                }
                (TokenKind::RBrace, None) => {
                    self.builder.set_syntax(Syntax::Unbalanced);
                    self.error("unexpected `}`", token.span);
                    self.advance();
                    continue;
                }
                _ => {}
            }
            let start = self.pos;
            let marks = self.marks();
            let waiting = pending;
            let mut parsed = self.parse_body_item(token, open.is_some(), &mut pending);
            // Gone on to a line that is an item of its own: parsed again to
            // the end of its own line, and the next line left for the next
            // item.
            if parsed.is_err()
                && let Some(line) = self.item_line(start)
            {
                self.rewind(start, marks);
                pending = waiting;
                let outer = self.line_end.replace(line);
                parsed = self.parse_body_item(token, open.is_some(), &mut pending);
                self.line_end = outer;
            }
            let (item, item_height) = match parsed {
                Ok(parsed) if !poisoned => parsed,
                Ok(_) | Err(Reported) => {
                    pending = Pending::default();
                    let item = self.error_item(start, marks, Sync::Item { top });
                    // An item starts at no separator, closing brace or end
                    // of input, where a skip stops, so it consumes a token
                    // and the loop ends; this keeps it so whatever happens.
                    let stops = matches!(
                        self.peek().kind,
                        TokenKind::Eof | TokenKind::RBrace | TokenKind::Newline | TokenKind::Semi
                    );
                    if self.pos == start && !stops {
                        self.advance();
                    }
                    (item, 0)
                }
            };
            poisoned = false;
            self.item_stack.push(item);
            height = height.max(item_height);
        }
        self.recovery = recovery;
        let range = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        (range, height)
    }

    /// Parses the item of a body, a block's if `inner`, starting at
    /// `token`, which is followed by the end of the item, and applies the
    /// outer attributes `pending` to it. Returns it and its height.
    fn parse_body_item(
        &mut self,
        token: Token,
        inner: bool,
        pending: &mut Pending,
    ) -> ParseResult<(ItemId, u32)> {
        let (item, item_height) = match token.kind {
            TokenKind::Import if inner => return Err(self.error(MISPLACED_IMPORT, token.span)),
            TokenKind::Import => (self.parse_import(token)?, 0),
            _ if self.starts_type_item() => {
                if inner {
                    return Err(self.error(MISPLACED_TYPE, token.span));
                }
                self.parse_type_item(token)?
            }
            _ if self.starts_defmeta() => {
                if inner {
                    return Err(self.error(MISPLACED_DEFMETA, token.span));
                }
                self.parse_defmeta(token)?
            }
            _ => self.parse_item()?,
        };
        if inner
            && let item = *self.builder.item(item)
            && matches!(item.kind, ItemKind::DefTypeDef { .. })
        {
            // At its `def`.
            let start = item.span.start;
            let keyword = Span::new(start, start.saturating_add(3));
            return Err(self.error(MISPLACED_DEF_TYPE_DEF, keyword));
        }
        if let Some(attribute) = pending.shadowable.take() {
            self.attach_shadowable(attribute, item, !inner)?;
        }
        if let Some(attribute) = pending.def_type.take() {
            self.attach_def_type(attribute, item, !inner)?;
        }
        self.expect_end_of_item()?;
        Ok((item, item_height))
    }

    /// Takes the outer attribute `token`, named `name`, at the start of an
    /// item of a body, as an attribute of the item after it, `pending` (see
    /// [Attributes](#attributes)): `#[shadowable]` and `#[def_type]` are, each
    /// once.
    fn outer_attribute(
        &mut self,
        token: Token,
        name: Name,
        pending: &mut Pending,
    ) -> ParseResult<()> {
        let text = self.names.get(name).to_owned();
        let slot = match text.as_str() {
            SHADOWABLE => &mut pending.shadowable,
            DEF_TYPE => &mut pending.def_type,
            _ => return Err(self.misplaced_attribute(token)),
        };
        if slot.is_some() {
            return Err(self.error(format!("`#[{text}]` is written twice"), token.span));
        }
        *slot = Some(token.span);
        self.advance();
        Ok(())
    }

    /// Takes the inner attribute `token`, named `name`, at the start of an
    /// item of a body, as the file's, if it is `first`: before the file's
    /// first item and any outer attribute (see [Attributes](#attributes)).
    fn inner_attribute(&mut self, token: Token, name: Name, first: bool) -> ParseResult<()> {
        if self.names.get(name) != NO_PRELUDE || !first {
            return Err(self.misplaced_attribute(token));
        }
        if self.builder.no_prelude().is_some() {
            return Err(self.error("`#![no_prelude]` is written twice", token.span));
        }
        self.builder.set_no_prelude(token.span);
        self.advance();
        Ok(())
    }

    /// Reports the attribute `token` where it cannot be: by its name, an
    /// unknown one, one of the other kind, or, where it is, one that is not
    /// allowed there (see [Attributes](#attributes)).
    fn misplaced_attribute(&mut self, token: Token) -> Reported {
        let (inner, name) = match token.kind {
            TokenKind::Attribute(name) => (false, name),
            TokenKind::InnerAttribute(name) => (true, name),
            _ => return self.error("expected an attribute", token.span),
        };
        let text = self.names.get(name).to_owned();
        let message = match (text.as_str(), inner) {
            (SHADOWABLE | DEF_TYPE, true) => {
                format!("`{text}` applies to a declaration: write `#[{text}]`")
            }
            (NO_PRELUDE, false) => {
                "`no_prelude` applies to the whole file: write `#![no_prelude]`".to_owned()
            }
            (SHADOWABLE, false) => MISPLACED_SHADOWABLE.to_owned(),
            (DEF_TYPE, false) => MISPLACED_DEF_TYPE.to_owned(),
            (NO_PRELUDE, true) => MISPLACED_NO_PRELUDE.to_owned(),
            _ => format!("unknown attribute `{text}`"),
        };
        self.error(message, token.span)
    }

    /// Marks `item`, an item of a body that is the file's if `top`, with the
    /// `#[def_type]` written at `attribute` before it, if it is a top-level
    /// `def` (not a definition made with a def-type); else the error that it
    /// is not one (see [Attributes](#attributes)).
    fn attach_def_type(&mut self, attribute: Span, item: ItemId, top: bool) -> ParseResult<()> {
        let kind = self.builder.item(item).kind;
        if top && matches!(kind, ItemKind::Field { function: true, .. }) {
            self.builder.mark_def_type(item);
            return Ok(());
        }
        Err(self.error(MISPLACED_DEF_TYPE, attribute))
    }

    /// Marks `item`, an item of a body that is the file's if `top`, with the
    /// `#[shadowable]` written at `attribute` before it, if it is a
    /// top-level field, `def` or `import`; else the error that it is not one
    /// (see [Attributes](#attributes)).
    fn attach_shadowable(&mut self, attribute: Span, item: ItemId, top: bool) -> ParseResult<()> {
        let kind = self.builder.item(item).kind;
        if top
            && matches!(
                kind,
                ItemKind::Field { .. } | ItemKind::Import(_) | ItemKind::DefTypeDef { .. }
            )
        {
            self.builder.mark_shadowable(item);
            return Ok(());
        }
        // A meta is its definition's: a definition that shadows another
        // brings its own.
        if matches!(kind, ItemKind::DefMeta { .. }) {
            return Err(self.error("`#[shadowable]` does not apply to a `defmeta`", attribute));
        }
        let mut error = Diagnostic::error(MISPLACED_SHADOWABLE, attribute);
        if matches!(kind, ItemKind::Let { .. }) {
            error = error.with_note(LET_SHADOWS_NOTE);
        }
        Err(self.report(error))
    }

    /// Parses one item. Returns it and its height: the taller of its
    /// expression and the positions of its path's index segments, plus two
    /// levels for each segment of its path after the first, or its
    /// annotation's if that is taller.
    fn parse_item(&mut self) -> ParseResult<(ItemId, u32)> {
        let token = self.peek();
        let name_token = self.peek_at(1);
        if token.kind == TokenKind::Fn
            && let TokenKind::Ident(name) = name_token.kind
        {
            return self.parse_named_fn(token, (name, name_token.span));
        }
        if token.kind == TokenKind::Def {
            return self.parse_def(token);
        }
        if token.kind == TokenKind::Mut {
            let note = self.starts_field(1).then_some(MUT_FIELD_NOTE);
            return Err(self.misplaced_mut(token, note));
        }
        let (kind, value) = match token.kind {
            TokenKind::Let => {
                let (kind, value, ty_height, path_shape) = self.parse_let(token)?;
                return self.finish_item(token, (kind, value), ty_height, path_shape);
            }
            TokenKind::Break | TokenKind::Continue | TokenKind::Return => {
                return self.parse_jump(token);
            }
            TokenKind::Label(_) => return Err(self.misplaced_label(token)),
            TokenKind::Eq => {
                self.advance();
                let value = self.parse_value()?;
                (ItemKind::Result(value), value)
            }
            TokenKind::DotDotDot => {
                self.advance();
                let value = self.parse_value()?;
                (ItemKind::Spread(value), value)
            }
            _ if self.starts_path_item() => {
                let (kind, value, path, ty_height) = self.parse_path_item()?;
                return self.finish_item(token, (kind, value), ty_height, path);
            }
            _ => {
                let expr = self.parse_expr(0)?;
                let operator = self.peek();
                let rebind = match operator.kind {
                    TokenKind::Eq => false,
                    TokenKind::ColonEq => true,
                    _ => {
                        let kind = (ItemKind::Expr(expr), expr);
                        return self.finish_item(token, kind, 0, PathShape::SINGLE);
                    }
                };
                return Err(self.not_a_path(expr, operator, rebind));
            }
        };
        self.finish_item(token, (kind, value), 0, PathShape::SINGLE)
    }

    /// Parses a `let` item, `let PATH (: TYPE)? = EXPR` or `let mut NAME (:
    /// TYPE)? = EXPR`, starting at its `let`, `token`. Returns its kind, its
    /// value, its annotation's height (0 if it has none) and its path's
    /// shape.
    fn parse_let(&mut self, token: Token) -> ParseResult<(ItemKind, ExprId, u32, PathShape)> {
        self.advance();
        self.skip_newlines();
        let mutable = self.peek().kind == TokenKind::Mut;
        let keyword = if mutable {
            let keyword = self.peek();
            self.advance();
            self.skip_newlines();
            keyword
        } else {
            token
        };
        let (path, shape) = self.parse_let_path(keyword, mutable)?;
        let (ty, ty_height) = if self.peek().kind == TokenKind::Colon {
            self.advance();
            self.skip_newlines();
            let (ty, height) = self.parse_type()?;
            (Some(ty), height)
        } else {
            (None, 0)
        };
        let eq = self.peek();
        if eq.kind != TokenKind::Eq {
            return Err(self.error("expected `=` after variable name", eq.span));
        }
        self.advance();
        let value = self.parse_value()?;
        let kind = ItemKind::Let {
            path,
            ty,
            value,
            mutable,
            function: false,
        };
        Ok((kind, value, ty_height, shape))
    }

    /// Parses a `break`, `continue` or `return` item, starting at its
    /// keyword, `keyword`: the keyword, its label, if one is written
    /// directly after it, and its value, if one begins on the same line,
    /// since a newline, a `;` or a `}` after the keyword and the label means
    /// there is none. Returns it and its height: its value's, or 0 without
    /// one.
    fn parse_jump(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let label = self.parse_label(keyword)?;
        let value = match self.peek().kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => None,
            _ => Some(self.parse_expr(0)?),
        };
        let end = match (value, label) {
            (Some(value), _) => self.span_of(value),
            (None, Some(label)) => label.span,
            (None, None) => keyword.span,
        };
        let span = Span::join(keyword.span, end);
        let height = value.map_or(0, |value| self.height_of(value));
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let jump = Jump { label, value };
        let kind = match keyword.kind {
            TokenKind::Break => ItemKind::Break(jump),
            TokenKind::Return => ItemKind::Return(jump),
            _ => ItemKind::Continue(jump),
        };
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, height))
    }

    /// Parses the label of the `for`, `do`, `break`, `continue` or `return`
    /// `keyword`, or of the block whose `{` it is, just consumed, if one
    /// follows it: one written directly after it, with no space, or the
    /// error "a label is written directly after `for`, with no space" (or
    /// the keyword's, or `{`) at a label with space before it.
    fn parse_label(&mut self, keyword: Token) -> ParseResult<Option<Label>> {
        let token = self.peek();
        let TokenKind::Label(name) = token.kind else {
            return Ok(None);
        };
        if token.span.start != keyword.span.end {
            let written = keyword.kind.symbol().unwrap_or("its keyword");
            return Err(self.error(
                format!("a label is written directly after `{written}`, with no space"),
                token.span,
            ));
        }
        self.advance();
        Ok(Some(Label {
            name,
            span: token.span,
        }))
    }

    /// Reports `token`, a label where none is allowed: anywhere but directly
    /// after `for`, `do`, `break`, `continue`, `return` or a block's `{`.
    fn misplaced_label(&mut self, token: Token) -> Reported {
        self.error(MISPLACED_LABEL, token.span)
    }

    /// Reports `token`, a `{` where an expression starts in a `for` header,
    /// which begins the body (see [Trailing blocks](#trailing-blocks)).
    fn header_brace(&mut self, token: Token) -> Reported {
        self.report(Diagnostic::error(HEADER_BRACE, token.span).with_note(HEADER_BRACE_NOTE))
    }

    /// Whether the tokens from `ahead` places after the current one begin a
    /// field item: a path, a name or a plain string literal and then `.`
    /// and a name as many times as they follow, and then `=`.
    fn starts_field(&self, ahead: usize) -> bool {
        self.path_end(ahead, false)
            .is_some_and(|end| self.peek_at(end).kind == TokenKind::Eq)
    }

    /// Whether the current token begins a field item or a rebind whose
    /// path is written as one: a name or a plain string literal, then `.`
    /// and a name or `[`, anything and the matching `]` as many times as
    /// they follow, and then `=` or `:=`, or `:` and an annotation followed
    /// by `=` ([`Parser::annotated`]). Such an item is read by
    /// [`Parser::parse_path_item`]; any other item that turns out to be
    /// followed by `=` or `:=` is not a path (see [`Parser::not_a_path`]).
    /// The tokens between brackets are only counted here, and parsed as
    /// the position afterwards, so an item starting with an index is looked
    /// through twice.
    fn starts_path_item(&self) -> bool {
        self.path_end(0, true)
            .is_some_and(|end| match self.peek_at(end).kind {
                TokenKind::Eq | TokenKind::ColonEq => true,
                TokenKind::Colon => self.annotated(end + 1),
                _ => false,
            })
    }

    /// Whether the tokens from `ahead` places after the current one, which
    /// follow a path and its `:`, are an annotation followed by the `=` of
    /// an annotated field, rather than the type of an ascription (see
    /// [Paths](#paths)): an `=` comes before the item can end, outside any
    /// brackets. The item ends at a `;`, a `:=`, a closing bracket that
    /// opened before it or the end of input, or a newline outside brackets
    /// that does not follow `:`, `|`, `<`, `->` or `->>`, after which a
    /// type goes on. The tokens are only counted here.
    fn annotated(&self, ahead: usize) -> bool {
        let mut depth = 0_usize;
        let mut previous = TokenKind::Colon;
        for at in ahead.. {
            let kind = self.peek_at(at).kind;
            match kind {
                TokenKind::Eq if depth == 0 => return true,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    let Some(outer) = depth.checked_sub(1) else {
                        return false;
                    };
                    depth = outer;
                }
                TokenKind::Newline if depth > 0 => continue,
                TokenKind::Newline => {
                    let goes_on = matches!(
                        previous,
                        TokenKind::Colon
                            | TokenKind::Pipe
                            | TokenKind::Lt
                            | TokenKind::Arrow
                            | TokenKind::MemoArrow
                    );
                    if goes_on {
                        continue;
                    }
                    return false;
                }
                TokenKind::Semi | TokenKind::ColonEq if depth == 0 => return false,
                TokenKind::Eof => return false,
                _ => {}
            }
            previous = kind;
        }
        false
    }

    /// How many places after the current token the tokens from `ahead`
    /// places after it stop spelling a path: a name or a plain string
    /// literal, then `.` and a name and, if `indexes`, `[` and everything up
    /// to the matching `]`, as many times as they follow. `None` if they do
    /// not start with a name, or a `[` is never closed.
    fn path_end(&self, ahead: usize, indexes: bool) -> Option<usize> {
        let is_name = |token: Token| matches!(token.kind, TokenKind::Ident(_) | TokenKind::Str(_));
        let mut at = ahead;
        if !is_name(self.peek_at(at)) {
            return None;
        }
        at += 1;
        loop {
            match self.peek_at(at).kind {
                TokenKind::Dot if is_name(self.peek_at(at + 1)) => at += 2,
                TokenKind::LBracket if indexes => at = self.after_brackets(at)?,
                _ => return Some(at),
            }
        }
    }

    /// How many places after the current token the token after the `]`
    /// matching the `[` `open` places after it is, counting brackets alone;
    /// `None` if the input ends first.
    fn after_brackets(&self, open: usize) -> Option<usize> {
        let mut depth = 0_usize;
        let mut at = open;
        loop {
            match self.peek_at(at).kind {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(at + 1);
                    }
                }
                TokenKind::Eof => return None,
                _ => {}
            }
            at += 1;
        }
    }

    /// Reports `token`, a `mut` where it is not allowed, with `note` if one
    /// says what to write instead.
    fn misplaced_mut(&mut self, token: Token, note: Option<&str>) -> Reported {
        let error = Diagnostic::error(MISPLACED_MUT, token.span);
        self.report(match note {
            Some(note) => error.with_note(note),
            None => error,
        })
    }

    /// Parses a public function item, `def PATH(...) ...`, starting at its
    /// `def` keyword, `keyword`. The path is an identifier, then `.` and an
    /// identifier as many times as they follow, directly followed by the
    /// parameters: a string where it starts is "a `def` is named by an
    /// identifier, not a string", a quoted or an index segment after it "a
    /// `def`'s path is made of identifiers", and anything else where a name
    /// is expected an error at it. A path of one name is the named function
    /// item's ([`Parser::parse_named_fn`]); a longer one is the dotted field
    /// of that path, marked as written as a `def` in the same way.
    fn parse_def(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        let name_token = self.peek_at(1);
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) | TokenKind::StrStart(_) => {
                return Err(self.error(DEF_STRING_NAME, name_token.span));
            }
            _ => {
                let found = self.describe(name_token);
                return Err(self.report(Diagnostic::error(
                    format!("expected the function's name after `def`, found {found}"),
                    name_token.span,
                )));
            }
        };
        let first = PathSegment {
            kind: SegmentKind::Name(name),
            span: name_token.span,
        };
        // A name on its line after a name, `def package x`: the definition
        // of `x` made with the def-type `package`.
        if self.is_name_after(name_token.span, self.peek_at(2)) {
            self.advance();
            self.advance();
            return self.parse_def_type_def(keyword, &[first]);
        }
        if !matches!(self.peek_at(2).kind, TokenKind::Dot | TokenKind::LBracket) {
            return self.parse_named_fn(keyword, (name, name_token.span));
        }
        self.advance();
        self.advance();
        // Kept apart until the function is parsed: the items of its body
        // read paths of their own.
        let mut segments = vec![first];
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    let token = self.peek();
                    match token.kind {
                        TokenKind::Ident(name) => {
                            self.advance();
                            segments.push(PathSegment {
                                kind: SegmentKind::Name(name),
                                span: token.span,
                            });
                        }
                        TokenKind::Str(_) => return Err(self.error(DEF_PATH, token.span)),
                        TokenKind::StrStart(first) => {
                            let literal = self.parse_interpolation(first)?;
                            let span = self.span_of(literal);
                            return Err(self.error(DEF_PATH, span));
                        }
                        _ => {
                            let found = self.describe(token);
                            return Err(self.error(
                                format!("expected the function's name after `.`, found {found}"),
                                error_span(token),
                            ));
                        }
                    }
                }
                TokenKind::LBracket => {
                    let (_, span) = self.parse_position()?;
                    return Err(self.error(DEF_PATH, span));
                }
                _ => break,
            }
        }
        if let Some(last) = segments.last()
            && self.is_name_after(last.span, self.peek())
        {
            return self.parse_def_type_def(keyword, &segments);
        }
        let value = self.parse_fn(keyword)?;
        let base = self.path.len();
        self.path.extend(segments);
        let (path, shape) = self.finish_path(base);
        let kind = ItemKind::Field {
            path,
            ty: None,
            value,
            function: true,
        };
        self.finish_item(keyword, (kind, value), 0, shape)
    }

    /// Whether `token` is an identifier on the line of `before`, the span of
    /// the name or path before it: what makes the two a def-type and a path
    /// (see [Def-types](#def-types)). The lexer emits no newline inside
    /// parentheses, so the source between them says.
    fn is_name_after(&self, before: Span, token: Token) -> bool {
        matches!(token.kind, TokenKind::Ident(_))
            && before.end <= token.span.start
            && !self
                .source
                .slice(Span::new(before.end, token.span.start))
                .contains('\n')
    }

    /// Allocates the expression that reads the path `segments`, a name and
    /// a field access for each segment after it, each spanning the path up
    /// to its segment, and returns the last: a def-type, `DT`, as it is
    /// written, or the read of a definition (see [Def-types](#def-types)).
    fn path_expr(&mut self, segments: &[PathSegment]) -> ParseResult<ExprId> {
        let mut read: Option<ExprId> = None;
        for (index, segment) in segments.iter().enumerate() {
            let Some(name) = segment.name() else {
                continue;
            };
            let span = path_span(&segments[..=index]);
            read = Some(match read {
                None => self.alloc(ExprKind::Name(name), span, LEAF_HEIGHT)?,
                Some(expr) => {
                    let height = self.height_of(expr) + 1;
                    self.alloc(ExprKind::Field { expr, name }, span, height)?
                }
            });
        }
        read.ok_or_else(|| self.error("expected a name", Span::new(0, 0)))
    }

    /// Reads the path of a definition or a reference made with a def-type,
    /// `PATH`, starting at its first identifier, the current token:
    /// segments joined by `.`, each an identifier, or an identifier with the
    /// tokens glued to it that make a name with `+` in it
    /// ([`Parser::path_segment`]), or, after the first, one that starts with
    /// a digit, from an integer or a float, split at its `.`
    /// ([`Parser::number_segments`]). A quoted segment, or an index, is "a
    /// `def`'s path is made of identifiers", but that an index ends the path
    /// of a `reference`, to index what it reads. Returns its segments.
    fn parse_def_type_path(&mut self, reference: bool) -> ParseResult<Vec<PathSegment>> {
        let mut segments = Vec::new();
        loop {
            let token = self.peek();
            match token.kind {
                TokenKind::Ident(name) => {
                    self.advance();
                    let segment = self.path_segment(token, name);
                    segments.push(segment);
                }
                // A segment after the first may start with a digit: an
                // integer, or a float split at its `.`.
                TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Duration(_)
                | TokenKind::Size(_)
                    if !segments.is_empty() =>
                {
                    self.advance();
                    self.number_segments(token, &mut segments);
                }
                _ => {
                    let found = self.describe(token);
                    return Err(self.error(
                        format!("expected the definition's name after `.`, found {found}"),
                        error_span(token),
                    ));
                }
            }
            match (self.peek().kind, self.peek_at(1).kind) {
                (
                    TokenKind::Dot,
                    TokenKind::Ident(_)
                    | TokenKind::Int(_)
                    | TokenKind::Float(_)
                    | TokenKind::Duration(_)
                    | TokenKind::Size(_),
                ) => {
                    self.advance();
                }
                (TokenKind::Dot, TokenKind::Str(_) | TokenKind::StrStart(_)) => {
                    self.advance();
                    let token = self.peek();
                    let span = match token.kind {
                        TokenKind::StrStart(first) => {
                            let literal = self.parse_interpolation(first)?;
                            self.span_of(literal)
                        }
                        _ => token.span,
                    };
                    return Err(self.error(DEF_PATH, span));
                }
                // A reference's path ends before an index, which indexes
                // what the reference reads: a list of its definition's
                // meta, `package x.versions[0]` (see the checker's "Metas").
                (TokenKind::LBracket, _) if !reference => {
                    let (_, span) = self.parse_position()?;
                    return Err(self.error(DEF_PATH, span));
                }
                _ => return Ok(segments),
            }
        }
    }

    /// The segment of a definition's path that starts at the identifier
    /// `first`, of name `name`, just consumed (see [Def-types](#def-types)):
    /// the identifier alone, unless a `+` directly follows it, when the
    /// segment is the run of the tokens glued to it ([`Parser::glued_run`]),
    /// consumed, and its name the run's source text, interned, so that
    /// `gmp-c++` is one name. `gmp-c++`, `a+b`, `c++11` and
    /// `gcc-libstdc++-devel` (lexed as `gcc-libstdc`, `+`, `+`, `-`,
    /// `devel`) are each one segment; `a + 1`, with spaces, is not.
    fn path_segment(&mut self, first: Token, name: Name) -> PathSegment {
        let (glued, end) = if self.peek().kind == TokenKind::Plus {
            self.glued_run(first.span.end)
        } else {
            (0, first.span.end)
        };
        if glued == 0 {
            return PathSegment {
                kind: SegmentKind::Name(name),
                span: first.span,
            };
        }
        self.pos += glued;
        self.written_segment(Span::new(first.span.start, end))
    }

    /// The segments of a definition's path that start at the number literal
    /// `first`, an integer or a float, just consumed, after the path's first
    /// segment (see [Def-types](#def-types)), pushed onto `segments`: a
    /// float with a fraction is split at its `.`, as if it were an integer,
    /// `.` and an integer, into the digits before it, one segment, and those
    /// after it, which start the next; anything else starts one whole
    /// (`24`, `1_0`, `1e5`). The segment it starts is the run of it and the
    /// tokens glued to it, consumed ([`Parser::glued_run`]), whatever
    /// follows, named by the run's source text, interned: `1.83` is `1` and
    /// `83`, `24rc1` (`24` and `rc1`) one segment, and `1.83rc1` `1` and
    /// `83rc1`.
    fn number_segments(&mut self, first: Token, segments: &mut Vec<PathSegment>) {
        let mut start = first.span.start;
        let dot = self
            .source
            .slice(first.span)
            .find('.')
            .and_then(|at| u32::try_from(at).ok());
        if let Some(dot) = dot {
            segments.push(self.written_segment(Span::new(start, start + dot)));
            start += dot + 1;
        }
        let (glued, end) = self.glued_run(first.span.end);
        self.pos += glued;
        segments.push(self.written_segment(Span::new(start, end)));
    }

    /// How many tokens, from the current one on, are glued to a segment of a
    /// definition's path whose run so far ends at `end`, and where the run
    /// then ends (see [Def-types](#def-types)). A token is glued when it
    /// directly follows the one before, nothing between them, and its text
    /// is made only of ASCII letters, digits, `_`, `+` and `-`, a `-` that
    /// ends one being glued only with a token glued after it: so the run is
    /// a maximal one of those characters in which every `-` is directly
    /// followed by a letter, a digit, `_` or `+` (the lexer allowing no
    /// `--`), and that does not end with `-`.
    fn glued_run(&self, end: u32) -> (usize, u32) {
        let (mut glued, mut glued_end) = (0, end);
        let (mut ahead, mut end) = (0, end);
        loop {
            let token = self.peek_at(ahead);
            let text = self.source.slice(token.span);
            if token.span.start != end
                || text.is_empty()
                || !text
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
            {
                break;
            }
            ahead += 1;
            end = token.span.end;
            if !text.ends_with('-') {
                (glued, glued_end) = (ahead, end);
            }
        }
        (glued, glued_end)
    }

    /// The segment of a definition's path of span `span`, named by its
    /// source text, interned.
    fn written_segment(&mut self, span: Span) -> PathSegment {
        PathSegment {
            kind: SegmentKind::Name(self.names.intern(self.source.slice(span))),
            span,
        }
    }

    /// Parses a definition made with a def-type, `def DT PATH(PARAMS)
    /// BODY`, starting at its path, its `def`, `keyword`, and its def-type,
    /// `dt`, read (see [Def-types](#def-types)): the parameters by name
    /// alone, each a type the def-type gives it, no return type, and the
    /// body. A parameter's type written is "the parameter types of a
    /// `package` definition come from `package`", at its `:`, and a return
    /// marker "the result type of a `package` definition comes from
    /// `package`", at the marker.
    fn parse_def_type_def(
        &mut self,
        keyword: Token,
        dt: &[PathSegment],
    ) -> ParseResult<(ItemId, u32)> {
        let shown = self.show_segments(dt);
        let dt_expr = self.path_expr(dt)?;
        let segments = self.parse_def_type_path(false)?;
        // The read of its meta, outside its function's subtree as its `DT`'s
        // is, which the workspace names once it knows its def-type.
        let meta_read = self.unresolved_meta_read(self.span_of(dt_expr), &segments)?;
        let open = self.peek();
        if open.kind != TokenKind::LParen {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `(` after the definition's path, found {found}"),
                error_span(open),
            ));
        }
        self.advance();
        let (params, close) = self.parse_definition_params(open, &shown)?;
        if self.meta_marker().is_some() {
            let span = Span::join(self.peek().span, self.peek_at(1).span);
            return Err(self.error(MISPLACED_META, span));
        }
        let marker = self.peek();
        if matches!(
            marker.kind,
            TokenKind::Colon | TokenKind::Arrow | TokenKind::MemoArrow
        ) {
            return Err(self.error(
                format!("the result type of a `{shown}` definition comes from `{shown}`"),
                marker.span,
            ));
        }
        let brace = self.peek();
        if brace.kind != TokenKind::LBrace {
            let found = self.describe(brace);
            return Err(self.error(
                format!("expected `{{` to start the definition's body, found {found}"),
                error_span(brace),
            ));
        }
        let body = self.parse_block()?;
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        // From its path, after its `DT`, whose expression comes before its
        // body's in the arena ([`Ast::subtree`]).
        let span = Span::join(path_span(&segments), self.span_of(body));
        let function = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            // The def-type's to say: what is written here means nothing.
            effect: Effect::Pure,
            marker: self.span_of(dt_expr),
            body,
        });
        let value = self.alloc(ExprKind::Fn(function), span, height)?;
        let base = self.path.len();
        self.path.extend(segments);
        let (path, mut shape) = self.finish_path(base);
        // Its leaf is under its namespace's root, one segment more.
        shape.segments = shape.segments.saturating_add(1);
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Definition,
            dt: dt_expr,
            path,
            read: None,
            key: None,
            meta: None,
            meta_read: Some(meta_read),
            span: Span::join(open.span, close.span),
        });
        self.finish_item(
            keyword,
            (ItemKind::DefTypeDef { def, value }, value),
            0,
            shape,
        )
    }

    /// Parses the parameters of a definition made with the def-type
    /// `shown`, as it is written, after their `(`, `open`, through their
    /// `)`: names alone, separated by `,`, each with a type expression of
    /// [`TypeExprKind::Invalid`] that nothing reads, the def-type's to give
    /// (see [Def-types](#def-types)). Returns them, with the `)`. A
    /// parameter that fails is skipped to its `,`, and the list goes on, to
    /// fail at its end (see [Errors and recovery](#errors-and-recovery)).
    fn parse_definition_params(
        &mut self,
        open: Token,
        shown: &str,
    ) -> ParseResult<(crate::ast::ParamRange, Token)> {
        let base = self.param_stack.len();
        let mut failed = None;
        let close = loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                break token;
            }
            match self.parse_definition_param(open, shown) {
                Ok(Some(close)) => break close,
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RParen) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        let params = self.builder.push_params(&self.param_stack[base..]);
        self.param_stack.truncate(base);
        Ok((params, close))
    }

    /// Parses a parameter of a definition made with the def-type `shown`
    /// whose parameter list `open` opens, a name alone, and the `,` or the
    /// `)` after it. Returns the `)` if it ends the list.
    fn parse_definition_param(&mut self, open: Token, shown: &str) -> ParseResult<Option<Token>> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => return Err(self.error(PLAIN_VARIABLE_NAME, token.span)),
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => {
                return Err(self.misplaced_mut(token, Some(MUT_PARAM_NOTE)));
            }
            _ => {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected a parameter name, found {found}"),
                    error_span(token),
                ));
            }
        };
        self.advance();
        let colon = self.peek();
        if colon.kind == TokenKind::Colon {
            return Err(self.error(
                format!("the parameter types of a `{shown}` definition come from `{shown}`"),
                colon.span,
            ));
        }
        let ty = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Invalid,
            span: token.span,
        });
        self.param_stack.push(Param {
            name,
            span: token.span,
            ty,
            default: None,
            default_fn: None,
        });
        let separator = self.peek();
        Ok(self.list_separator(open)?.then_some(separator))
    }

    /// Parses a def-type reference, `DT PATH`, whose def-type, `dt`, has
    /// been parsed and whose path starts at the current token, an identifier
    /// on its line (see [Def-types](#def-types)): the path, and the read of
    /// the definition, a name standing for the def-type's namespace until
    /// the workspace names it, with a field access for each segment.
    fn parse_def_type_ref(&mut self, dt: ExprId) -> ParseResult<ExprId> {
        let segments = self.parse_def_type_path(true)?;
        let base = self.path.len();
        self.path.extend(segments.iter().copied());
        let (path, _) = self.finish_path(base);
        // The namespace's name, which no program can write; the workspace
        // replaces it with the namespace's key.
        let namespace = self.names.intern(UNRESOLVED_NAMESPACE);
        let mut read_path = Vec::with_capacity(segments.len() + 1);
        if let Some(first) = segments.first() {
            read_path.push(PathSegment {
                kind: SegmentKind::Name(namespace),
                span: Span::new(first.span.start, first.span.start),
            });
        }
        read_path.extend(segments.iter().copied());
        let read = self.path_expr(&read_path)?;
        let span = Span::join(self.span_of(dt), path_span(&segments));
        let height = self.height_of(dt).max(self.height_of(read)) + 1;
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Reference,
            dt,
            path,
            read: Some(read),
            key: None,
            meta: None,
            meta_read: None,
            span,
        });
        self.alloc(ExprKind::DefTypeRef(def), span, height)
    }

    /// Allocates the read of the meta of the definition whose def-type is
    /// written at `dt` and whose path is `segments`, before the workspace
    /// names what it reads (see [Def-types](#def-types)): a name standing
    /// for the namespace of the metas, written nowhere, at the start of
    /// `DT`, with one field access, spanning `DT` through the path, for the
    /// meta's leaf, which is the path as one name; both stand for nothing
    /// until the workspace names them ([`Ast::set_def_type_key`]). Before the
    /// definition's function in the arena, and starting before it, as its
    /// `DT` does ([`Ast::subtree`]).
    fn unresolved_meta_read(&mut self, dt: Span, segments: &[PathSegment]) -> ParseResult<ExprId> {
        let unresolved = self.names.intern(UNRESOLVED_NAMESPACE);
        self.path_expr(&[
            PathSegment {
                kind: SegmentKind::Name(unresolved),
                span: Span::new(dt.start, dt.start),
            },
            PathSegment {
                kind: SegmentKind::Name(unresolved),
                span: path_span(segments),
            },
        ])
    }

    /// Whether the current token begins a definition's meta, `defmeta DT
    /// PATH { ... }` or `defmeta DT PATH = EXPR` (see
    /// [Def-types](#def-types)): the name `defmeta`, then on its line a name
    /// or a dotted path, then on its line an identifier. `defmeta` is a name
    /// everywhere else: two names in a row after it never parsed before.
    fn starts_defmeta(&self) -> bool {
        let keyword = self.peek();
        if !matches!(keyword.kind, TokenKind::Ident(name) if self.names.get(name) == DEFMETA) {
            return false;
        }
        if !self.is_name_after(keyword.span, self.peek_at(1)) {
            return false;
        }
        // The def-type's path: identifiers joined by `.`.
        let mut last = 1;
        while self.peek_at(last + 1).kind == TokenKind::Dot
            && matches!(self.peek_at(last + 2).kind, TokenKind::Ident(_))
        {
            last += 2;
        }
        self.is_name_after(self.peek_at(last).span, self.peek_at(last + 1))
    }

    /// Parses a definition's meta, `defmeta DT PATH { ... }` or `defmeta DT
    /// PATH = EXPR`, starting at its `defmeta`, `keyword`, which
    /// [`Parser::starts_defmeta`] has found (see [Def-types](#def-types)):
    /// its def-type, its path, as a definition's is, and its body, the block
    /// or the expression, which makes a function of no parameters whose
    /// result type and marker are the meta's its def-type declares. Anything
    /// but `{` or `=` after the path is "expected `{` or `=` after the meta's
    /// path".
    fn parse_defmeta(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let mut dt = Vec::new();
        while let token = self.peek()
            && let TokenKind::Ident(name) = token.kind
        {
            dt.push(PathSegment {
                kind: SegmentKind::Name(name),
                span: token.span,
            });
            self.advance();
            if self.peek().kind != TokenKind::Dot
                || !matches!(self.peek_at(1).kind, TokenKind::Ident(_))
            {
                break;
            }
            self.advance();
        }
        let dt_expr = self.path_expr(&dt)?;
        let segments = self.parse_def_type_path(false)?;
        let token = self.peek();
        let body = match token.kind {
            TokenKind::LBrace => self.parse_block()?,
            // The expression, as the result of a block written nowhere.
            TokenKind::Eq => {
                self.advance();
                self.skip_newlines();
                let value = self.parse_expr(0)?;
                let span = self.span_of(value);
                let height = self.height_of(value);
                let result = self.builder.alloc_item(Item {
                    kind: ItemKind::Result(value),
                    span: Span::join(token.span, span),
                });
                let items = self.builder.push_item_list(&[result]);
                self.alloc(
                    ExprKind::Block { items, label: None },
                    span,
                    height.saturating_add(1),
                )?
            }
            _ => {
                if self.meta_marker().is_some() {
                    let span = Span::join(token.span, self.peek_at(1).span);
                    return Err(self.error(MISPLACED_META, span));
                }
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected `{{` or `=` after the meta's path, found {found}"),
                    error_span(token),
                ));
            }
        };
        let height = self.height_of(body).max(LEAF_HEIGHT) + 1;
        let span = Span::join(path_span(&segments), self.span_of(body));
        let params = self.builder.push_params(&[]);
        let function = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            // The meta's to say, as its def-type declares it.
            effect: Effect::Pure,
            marker: self.span_of(dt_expr),
            body,
        });
        let value = self.alloc(ExprKind::Fn(function), span, height)?;
        let path_at = Span::new(keyword.span.start, path_span(&segments).end);
        let base = self.path.len();
        self.path.extend(segments);
        let (path, mut shape) = self.finish_path(base);
        // Its leaf is the path, as one name, under its namespace's root.
        shape.segments = 2;
        let def = self.builder.push_def_type(DefTypeUse {
            role: DefTypeRole::Meta,
            dt: dt_expr,
            path,
            read: None,
            key: None,
            meta: None,
            meta_read: None,
            span: path_at,
        });
        self.finish_item(keyword, (ItemKind::DefMeta { def, value }, value), 0, shape)
    }

    /// Whether `expr` is a def-type as a reference writes it, a name or a
    /// chain of field accesses directly on one (see [Def-types](#def-types)).
    fn is_path_expr(&self, expr: ExprId) -> bool {
        let mut at = expr;
        loop {
            match self.builder.get(at).kind {
                ExprKind::Name(_) => return true,
                ExprKind::Field { expr, .. } => at = expr,
                _ => return false,
            }
        }
    }

    /// The path `segments`, of names, as it is written.
    fn show_segments(&self, segments: &[PathSegment]) -> String {
        segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| self.names.get(name))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Parses a named function item, `fn NAME(...) ...` or `def NAME(...)
    /// ...`, starting at its keyword, `keyword`, followed by `name` and its
    /// span. After `fn`, a `let` of the name, and after `def`, a field of
    /// the name, whose value is the function expression from `(` on, both
    /// spanning the keyword through the body's `}`; either marked as
    /// written as a named function, not as `let NAME = fn` or `NAME = fn`.
    fn parse_named_fn(
        &mut self,
        keyword: Token,
        (name, name_span): (Name, Span),
    ) -> ParseResult<(ItemId, u32)> {
        self.advance();
        self.advance();
        let value = self.parse_fn(keyword)?;
        let base = self.path.len();
        self.path.push(PathSegment {
            kind: SegmentKind::Name(name),
            span: name_span,
        });
        let (path, shape) = self.finish_path(base);
        let kind = if keyword.kind == TokenKind::Def {
            ItemKind::Field {
                path,
                ty: None,
                value,
                function: true,
            }
        } else {
            ItemKind::Let {
                path,
                ty: None,
                value,
                mutable: false,
                function: true,
            }
        };
        self.finish_item(keyword, (kind, value), 0, shape)
    }

    /// Parses an import item of the file's body, starting at its keyword,
    /// `keyword` (see [Imports](#imports)).
    fn parse_import(&mut self, keyword: Token) -> ParseResult<ItemId> {
        self.advance();
        let first = self.peek();
        let mut members = Vec::new();
        if first.kind == TokenKind::Star {
            self.advance();
            members.push(ImportMember::All(first.span));
        } else {
            let mut end = keyword.span;
            if self.is_from(first) {
                self.advance();
                return Err(self.expected_path_after_from());
            }
            let base = self.path.len();
            let path_span = self.parse_module_path(first, "after `import`")?;
            end = end.join(path_span);
            let alias = self.parse_import_alias(&mut end)?;
            let next = self.peek();
            if next.kind != TokenKind::Comma && !self.is_from(next) {
                if alias.is_none() {
                    self.unnamed_import(base)?;
                }
                let path = self.builder.push_path(&self.path[base..]);
                self.path.truncate(base);
                let decl = ImportDecl {
                    path,
                    form: ImportForm::Whole(alias),
                };
                return Ok(self.finish_import(decl, keyword.span.join(end)));
            }
            let segments = &self.path[base..];
            let first_segment = segments.first().copied();
            let single = segments.len() == 1
                && first_segment.is_some_and(|segment| {
                    segment
                        .name()
                        .is_some_and(|name| lexer::is_identifier(self.names.get(name)))
                });
            self.path.truncate(base);
            if !single {
                return Err(self.error(SINGLE_MEMBER, path_span));
            }
            if let Some(PathSegment {
                kind: SegmentKind::Name(name),
                span,
            }) = first_segment
            {
                members.push(ImportMember::Named {
                    name: ImportName { name, span },
                    alias,
                });
            }
        }
        self.parse_import_members(&mut members)?;
        let path_start = self.peek();
        let names_a_path = matches!(
            path_start.kind,
            TokenKind::Ident(_) | TokenKind::Str(_) | TokenKind::StrStart(_)
        );
        if !names_a_path || self.is_from(path_start) {
            return Err(self.expected_path_after_from());
        }
        let base = self.path.len();
        let path_span = self.parse_module_path(path_start, "after `from`")?;
        let path = self.builder.push_path(&self.path[base..]);
        self.path.truncate(base);
        let range = self.builder.push_import_members(&members);
        let decl = ImportDecl {
            path,
            form: ImportForm::Members(range),
        };
        Ok(self.finish_import(decl, keyword.span.join(path_span)))
    }

    /// Whether the current token begins a `type` item: the identifier
    /// `type`, the contextual keyword, directly followed on its line by an
    /// identifier (see [Types](#types)).
    fn starts_type_item(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(name) if self.names.get(name) == TYPE)
            && matches!(self.peek_at(1).kind, TokenKind::Ident(_))
    }

    /// Parses a `type` item of the file's body, `type NAME = TYPE`,
    /// starting at its `type`, `keyword`, which [`Parser::starts_type_item`]
    /// has found to be followed by an identifier. Returns it and its height,
    /// its type's. Type parameters, `type Box<T> = ...`, are an error at
    /// their `<`.
    fn parse_type_item(&mut self, keyword: Token) -> ParseResult<(ItemId, u32)> {
        self.advance();
        let name_token = self.peek();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(self.error("expected the type's name after `type`", name_token.span));
        };
        self.advance();
        let eq = self.peek();
        match eq.kind {
            TokenKind::Eq => {}
            TokenKind::Lt => return Err(self.error(TYPE_PARAMETERS, eq.span)),
            _ => {
                let found = self.describe(eq);
                return Err(self.error(
                    format!("expected `=` after the type's name, found {found}"),
                    error_span(eq),
                ));
            }
        }
        self.advance();
        self.skip_newlines();
        let (ty, height) = self.parse_type()?;
        let span = Span::join(keyword.span, self.type_span(ty));
        let item = self.builder.alloc_item(Item {
            kind: ItemKind::Type {
                name,
                name_span: name_token.span,
                ty,
            },
            span,
        });
        Ok((item, height))
    }

    /// Reads the rest of a `from` import's member list after its first
    /// member, which is in `members`, through its `from`, adding each.
    fn parse_import_members(&mut self, members: &mut Vec<ImportMember>) -> ParseResult<()> {
        loop {
            let next = self.peek();
            if self.is_from(next) {
                self.advance();
                return Ok(());
            }
            if next.kind != TokenKind::Comma {
                if let Some(star) = members.iter().find_map(|member| match member {
                    ImportMember::All(span) => Some(*span),
                    ImportMember::Named { .. } => None,
                }) {
                    return Err(self.error("`import *` needs `from` and a path", star));
                }
                let found = self.describe(next);
                return Err(self.error(format!("expected `,` or `from`, found {found}"), next.span));
            }
            self.advance();
            let member = self.peek();
            if member.kind == TokenKind::Star {
                self.advance();
                if members.iter().any(|m| matches!(m, ImportMember::All(_))) {
                    return Err(self.error("`*` is imported more than once", member.span));
                }
                members.push(ImportMember::All(member.span));
                continue;
            }
            let TokenKind::Ident(name) = member.kind else {
                let found = self.describe(member);
                return Err(self.error(
                    format!("expected a name to import after `,`, found {found}"),
                    member.span,
                ));
            };
            if self.is_from(member) {
                let found = self.describe(member);
                return Err(self.error(
                    format!("expected a name to import after `,`, found {found}"),
                    member.span,
                ));
            }
            if self.module_run().0 > 1 {
                let base = self.path.len();
                let span = self.parse_module_path(member, "after `,`")?;
                self.path.truncate(base);
                return Err(self.error(SINGLE_MEMBER, span));
            }
            self.advance();
            let mut member_end = member.span;
            let alias = self.parse_import_alias(&mut member_end)?;
            members.push(ImportMember::Named {
                name: ImportName {
                    name,
                    span: member.span,
                },
                alias,
            });
        }
    }

    /// Reads a module path, segments separated by `/`, starting at `first`,
    /// which is the next token, onto the path stack (the caller moves or
    /// discards it), and returns the span it covers (see [Imports](#imports)).
    /// The path is the run of `first` and every token glued to it
    /// ([`Parser::module_run`]), split at each `/`; each segment is named by
    /// its source text, interned, so `pkgs/net.busybox.busybox` is the two
    /// segments `pkgs` and `net.busybox.busybox`. `after` says where the
    /// path is, for the error when `first` cannot start one.
    fn parse_module_path(&mut self, first: Token, after: &str) -> ParseResult<Span> {
        match first.kind {
            TokenKind::Str(_) | TokenKind::StrStart(_) => {
                return Err(self.error(QUOTED_MODULE_PATH, first.span));
            }
            TokenKind::Ident(_) if !self.is_from(first) => {}
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Duration(_)
            | TokenKind::Size(_) => {}
            _ => {
                let found = self.describe(first);
                return Err(self.error(
                    format!("expected a module path {after}, found {found}"),
                    first.span,
                ));
            }
        }
        let (glued, end) = self.module_run();
        let span = Span::new(first.span.start, end);
        let text = self.source.slice(span).to_owned();
        let mut start = span.start;
        let parts = text.split('/').count();
        for (index, part) in text.split('/').enumerate() {
            let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
            let segment = Span::new(start, start.saturating_add(len));
            if part.is_empty() {
                // The run starts with a name or a number, so an empty
                // segment follows a `/`: the `/` that ends the path, or the
                // second of two in a row.
                let slash = if index + 1 == parts {
                    Span::new(start.saturating_sub(1), start)
                } else {
                    Span::new(start, start + 1)
                };
                return Err(self.error(EMPTY_MODULE_SEGMENT, slash));
            }
            if !lexer::is_module_segment(part) {
                return Err(self.error(
                    format!("`{part}` cannot name a module: {MODULE_SEGMENT}"),
                    segment,
                ));
            }
            self.path.push(PathSegment {
                kind: SegmentKind::Name(self.names.intern(part)),
                span: segment,
            });
            start = segment.end.saturating_add(1);
        }
        self.pos += glued;
        Ok(span)
    }

    /// How many tokens, from the current one on, make up the run of a module
    /// path, and where the run ends (see [Imports](#imports)): the current
    /// token, and every token after it that directly follows the one before,
    /// nothing between them, and whose text is made only of ASCII letters,
    /// digits, `_`, `-`, `+`, `.` and `/`.
    fn module_run(&self) -> (usize, u32) {
        let first = self.peek();
        let (mut count, mut end) = (1, first.span.end);
        loop {
            let token = self.peek_at(count);
            let text = self.source.slice(token.span);
            if token.span.start != end
                || text.is_empty()
                || !text.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.' | b'/')
                })
            {
                return (count, end);
            }
            count += 1;
            end = token.span.end;
        }
    }

    /// Checks that a whole import with no `as`, whose path is on the path
    /// stack from `base`, can be bound as its path's last segment: that the
    /// segment is a name, as `helpers` is and `net.busybox.busybox` is not
    /// (see [Imports](#imports)).
    fn unnamed_import(&mut self, base: usize) -> ParseResult<()> {
        let Some(&last) = self.path[base..].last() else {
            return Ok(());
        };
        let Some(name) = last.name() else {
            return Ok(());
        };
        let text = self.names.get(name);
        if lexer::is_identifier(text) {
            return Ok(());
        }
        let message = format!("`{text}` is not a name, so the import needs one");
        let note = format!("write `as NAME` after the path to name what `{text}` imports");
        let mut error = Diagnostic::error(message, last.span).with_note(note);
        // A path in the old form, its segments separated by `.`, is one
        // segment of dots now.
        if text.split('.').all(lexer::is_identifier) {
            let slashed = text.replace('.', "/");
            error = error.with_note(format!(
                "a module path separates its segments with `/`: if `{text}` is meant as \
                 directories, write `{slashed}`"
            ));
        }
        Err(self.report(error))
    }

    /// Reads `as NAME` after an import's path or member, if the next token
    /// is `as`, widening `end` to cover it.
    fn parse_import_alias(&mut self, end: &mut Span) -> ParseResult<Option<ImportName>> {
        if self.peek().kind != TokenKind::As {
            return Ok(None);
        }
        self.advance();
        let token = self.peek();
        match token.kind {
            TokenKind::Ident(name) if !self.is_from(token) => {
                self.advance();
                *end = end.join(token.span);
                Ok(Some(ImportName {
                    name,
                    span: token.span,
                }))
            }
            _ => {
                let found = self.describe(token);
                Err(self.error(
                    format!("expected a name after `as`, found {found}"),
                    token.span,
                ))
            }
        }
    }

    /// Whether `token` is the identifier `from`, which is the keyword
    /// wherever it appears in an import item.
    fn is_from(&self, token: Token) -> bool {
        matches!(token.kind, TokenKind::Ident(name) if self.names.get(name) == "from")
    }

    /// Reports "expected a path after `from`" at the next token.
    fn expected_path_after_from(&mut self) -> Reported {
        let token = self.peek();
        let found = self.describe(token);
        self.error(
            format!("expected a path after `from`, found {found}"),
            token.span,
        )
    }

    /// Allocates the import item `decl`, spanning `span`.
    fn finish_import(&mut self, decl: ImportDecl, span: Span) -> ItemId {
        let id = self.builder.push_import(decl);
        self.builder.alloc_item(Item {
            kind: ItemKind::Import(id),
            span,
        })
    }

    /// Allocates the item of `kind`, starting at `token` and ending with its
    /// expression `value`, whose annotation, if any, is `ty_height` tall and
    /// whose path has the shape `path`. Returns it and its height, failing
    /// if that exceeds [`MAX_DEPTH`].
    fn finish_item(
        &mut self,
        token: Token,
        (kind, value): (ItemKind, ExprId),
        ty_height: u32,
        path: PathShape,
    ) -> ParseResult<(ItemId, u32)> {
        let span = Span::join(token.span, self.span_of(value));
        let path_height = path.segments.saturating_sub(1).saturating_mul(2);
        let height = self
            .height_of(value)
            .max(path.positions)
            .saturating_add(path_height)
            .max(ty_height);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let item = self.builder.alloc_item(Item { kind, span });
        Ok((item, height))
    }

    /// Parses the path after `let`, or after the `mut` of a `let mut`,
    /// `keyword`, as `mutable` says, starting at its first segment: an
    /// identifier, then `.` and a name, plain or quoted, as many times as
    /// they follow. Returns it and its shape.
    ///
    /// An index segment is read as in any path, and then reported at its
    /// brackets: "a `let` path cannot index a list", with a note writing the
    /// path as a `:=`. A `let mut`'s path must be a single name: a dotted one
    /// is "`let mut` needs a plain name", at the path.
    fn parse_let_path(
        &mut self,
        keyword: Token,
        mutable: bool,
    ) -> ParseResult<(PathRange, PathShape)> {
        let name_token = self.peek();
        let name = match name_token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => {
                return Err(self.error(PLAIN_VARIABLE_NAME, name_token.span));
            }
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(name_token, None)),
            TokenKind::Label(_) => return Err(self.misplaced_label(name_token)),
            _ => {
                let after = self.describe(keyword);
                return Err(self.error(format!("expected a name after {after}"), name_token.span));
            }
        };
        self.advance();
        let base = self.path.len();
        self.path.push(PathSegment {
            kind: SegmentKind::Name(name),
            span: name_token.span,
        });
        self.parse_segments()?;
        if let Some(error) = self.indexed_path(base, LET_INDEX) {
            return Err(error);
        }
        if mutable && self.path.len() - base > 1 {
            return Err(self.report(
                Diagnostic::error(DOTTED_MUT, path_span(&self.path[base..]))
                    .with_note(DOTTED_MUT_NOTE),
            ));
        }
        Ok(self.finish_path(base))
    }

    /// Parses a field item, `PATH = EXPR` or `PATH: TYPE = EXPR`, or a
    /// rebind, `PATH := EXPR`, whose tokens [`Parser::starts_path_item`] has
    /// found to begin with a path, starting at the path's first segment.
    /// Returns its kind, its value, its path's shape and its annotation's
    /// height (0 if it has none).
    ///
    /// The path's first segment is checked after the rest of the path is
    /// read, as when the path was read back from an expression: a field's
    /// name may be a plain string literal but not a raw one, and a
    /// variable's must be an identifier. Only a rebind's path may have index
    /// segments: a field's is reported at the first, "a field's path cannot
    /// index a list", with a note writing the path as a `:=`. An annotation
    /// is followed by `=`: "expected `=` after the field's type" otherwise.
    fn parse_path_item(&mut self) -> ParseResult<(ItemKind, ExprId, PathShape, u32)> {
        let first = self.peek();
        self.advance();
        let base = self.path.len();
        let positions = self.parse_segments()?;
        let (ty, ty_height) = if self.peek().kind == TokenKind::Colon {
            self.advance();
            self.skip_newlines();
            let (ty, height) = self.parse_type()?;
            let eq = self.peek();
            if eq.kind != TokenKind::Eq {
                let found = self.describe(eq);
                return Err(self.error(
                    format!("expected `=` after the field's type, found {found}"),
                    error_span(eq),
                ));
            }
            (Some(ty), height)
        } else {
            (None, 0)
        };
        let operator = self.peek();
        let rebind = operator.kind == TokenKind::ColonEq;
        let name = match first.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) if rebind => return Err(self.error(PLAIN_VARIABLE_NAME, first.span)),
            TokenKind::Str(text) => self.quoted_name(first, text)?,
            _ => {
                let found = self.describe(operator);
                return Err(self.error(
                    format!("expected end of item, found {found}"),
                    operator.span,
                ));
            }
        };
        self.path.insert(
            base,
            PathSegment {
                kind: SegmentKind::Name(name),
                span: first.span,
            },
        );
        if !rebind && let Some(error) = self.indexed_path(base, FIELD_INDEX) {
            return Err(error);
        }
        let (path, mut shape) = self.finish_path(base);
        shape.positions = positions;
        self.advance();
        let value = self.parse_value()?;
        let kind = if rebind {
            ItemKind::Rebind { path, value }
        } else {
            ItemKind::Field {
                path,
                ty,
                value,
                function: false,
            }
        };
        Ok((kind, value, shape, ty_height))
    }

    /// Parses the segments of a path after its first, onto `path`, while
    /// they follow: `.` and a name, plain or quoted, or an index, `[`, a
    /// position and `]`, which, as in an expression, must start on the line
    /// the path is on, a newline ending the path. Returns the height of the
    /// tallest position, 0 if there is none.
    fn parse_segments(&mut self) -> ParseResult<u32> {
        let mut tallest = 0;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    let segment = self.peek();
                    let Some(name) = self.field_name()? else {
                        return Err(
                            self.error("expected a field name after `.`", error_span(segment))
                        );
                    };
                    self.path.push(PathSegment {
                        kind: SegmentKind::Name(name),
                        span: segment.span,
                    });
                }
                TokenKind::LBracket => {
                    let (position, span) = self.parse_position()?;
                    tallest = tallest.max(self.height_of(position));
                    let written = self.source.slice(self.span_of(position));
                    let text = if written.contains('\n') {
                        self.strings.push(&one_line(written))
                    } else {
                        self.strings.push(written)
                    };
                    self.path.push(PathSegment {
                        kind: SegmentKind::Index { position, text },
                        span,
                    });
                }
                _ => return Ok(tallest),
            }
        }
    }

    /// The error `message` at the first index segment of the path being
    /// read from `base` on, if it has one, which may not: "a `let` path
    /// cannot index a list" or "a field's path cannot index a list", with a
    /// note writing the path as it is written, as a `:=`.
    fn indexed_path(&mut self, base: usize, message: &str) -> Option<Reported> {
        let path = self.path.get(base..)?;
        let index = path.iter().find(|segment| segment.position().is_some())?;
        let error = Diagnostic::error(message, index.span).with_note(format!(
            "update the element with `{} := ...`",
            one_line(self.source.slice(path_span(path)))
        ));
        Some(self.report(error))
    }

    /// The error for `expr`, parsed at the start of an item and followed by
    /// `operator`, the `=` of a field item or the `:=` of a rebind as
    /// `rebind` says, which is not a path, since [`Parser::starts_path_item`]
    /// found none: "expected end of item" at the operator, unless it is a
    /// chain of field accesses on a literal that is not a name, a raw or
    /// interpolated literal for a field and any literal for a variable.
    fn not_a_path(&mut self, expr: ExprId, operator: Token, rebind: bool) -> Reported {
        let mut node = expr;
        loop {
            let inner = *self.builder.get(node);
            match inner.kind {
                ExprKind::Field { expr, .. } => node = expr,
                ExprKind::Str(_) | ExprKind::Interp(_) if rebind => {
                    return self.error(PLAIN_VARIABLE_NAME, inner.span);
                }
                ExprKind::Interp(_) => return self.error(PLAIN_FIELD_NAME, inner.span),
                _ => {
                    let found = self.describe(operator);
                    return self.error(
                        format!("expected end of item, found {found}"),
                        operator.span,
                    );
                }
            }
        }
    }

    /// Moves the path being read, the segments of `path` from `base` on, to
    /// the builder's path pool, returning its range and its shape, with no
    /// positions: the caller knows how tall they are.
    fn finish_path(&mut self, base: usize) -> (PathRange, PathShape) {
        let segments = self.path.get(base..).unwrap_or_default();
        let shape = PathShape {
            segments: u32::try_from(segments.len()).unwrap_or(u32::MAX),
            positions: 0,
        };
        let range = self.builder.push_path(segments);
        self.path.truncate(base);
        (range, shape)
    }

    /// The name a plain string literal `token`, of text `text`, gives a
    /// field: its text, interned. A raw literal is an error.
    fn quoted_name(&mut self, token: Token, text: StrId) -> ParseResult<Name> {
        if self.source.slice(token.span).starts_with('r') {
            return Err(self.error(PLAIN_FIELD_NAME, token.span));
        }
        // A name of the form of a def-type's namespace key, or of a meta key,
        // could reach its definitions or their metas: no source may write
        // one (see [Def-types](#def-types)).
        let text = self.strings.get(text);
        if crate::names::is_namespace_key(text) {
            let message = format!(
                "a field cannot be named `{}`: names of that form are def-types' namespaces",
                crate::strings::FieldName(text)
            );
            return Err(self.error(message, token.span));
        }
        self.builder.push_quoted(token.span);
        Ok(self.names.intern(text))
    }

    /// Parses the name of a field after `.` or in a record type, the
    /// current token: an identifier or a plain string literal. Returns
    /// `None`, having consumed nothing, if the token is neither; a raw or
    /// interpolated literal is an error, and so is a `mut`.
    fn field_name(&mut self) -> ParseResult<Option<Name>> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(text) => self.quoted_name(token, text)?,
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_FIELD_NAME, span));
            }
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            _ => return Ok(None),
        };
        self.advance();
        Ok(Some(name))
    }

    /// Checks that the item just parsed is followed by a separator or the
    /// end of its body.
    fn expect_end_of_item(&mut self) -> ParseResult<()> {
        let token = self.peek();
        match token.kind {
            TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof => Ok(()),
            // `a -b`: the user most likely meant a subtraction. The lexer
            // only makes a `Neg` when an operand follows directly, so show
            // the `-` together with that operand's first token.
            TokenKind::Neg => {
                let span = Span::join(token.span, self.peek_at(1).span);
                let found = self.source.slice(span);
                Err(self.report(
                    Diagnostic::error(format!("expected end of item, found `{found}`"), span)
                        .with_note("write `a - b` with spaces for subtraction"),
                ))
            }
            TokenKind::Mut => Err(self.misplaced_mut(token, None)),
            TokenKind::Label(_) => Err(self.misplaced_label(token)),
            _ => {
                let found = self.describe(token);
                Err(self.error(format!("expected end of item, found {found}"), token.span))
            }
        }
    }

    /// Parses a type expression: a name, an applied type, a record type, a
    /// function type or a parenthesised type, or a union of such types.
    /// Returns it and its height: 1 for a name, one more than its argument
    /// for an applied type, one more than its tallest field's type for a
    /// record type, one more than the tallest of its parameter and return
    /// types for a function type, one more than the type inside for a
    /// parenthesised type, and one more than the taller of its two sides
    /// for a union.
    fn parse_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let (mut ty, mut height) = self.parse_type_member()?;
        // A union is left-associative and built by this loop, not by
        // recursion, so its height is checked here.
        while self.peek().kind == TokenKind::Pipe {
            let bar = self.peek().span;
            self.advance();
            self.skip_newlines();
            let (rhs, rhs_height) = self.parse_type_member()?;
            let span = Span::join(self.type_span(ty), self.type_span(rhs));
            height = height.max(rhs_height).saturating_add(1);
            if height as usize > MAX_DEPTH {
                return Err(self.too_deep(span));
            }
            ty = self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::Union { lhs: ty, rhs, bar },
                span,
            });
        }
        Ok((ty, height))
    }

    /// Parses a type that is not a union, unless in parentheses: a name, an
    /// applied type, a record type, a function type or a parenthesised
    /// type, followed by any number of `[]`, each making a list type of the
    /// type before it. Returns it and its height, as [`Parser::parse_type`]
    /// does, one more for each `[]`.
    fn parse_type_member(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let (mut ty, mut height) = self.parse_type_atom()?;
        // Repeated `[]`s are built by this loop, not by recursion, so their
        // height is checked here.
        while self.peek().kind == TokenKind::LBracket && self.peek_at(1).kind == TokenKind::RBracket
        {
            let close = self.peek_at(1);
            self.advance();
            self.advance();
            let span = Span::join(self.type_span(ty), close.span);
            height = height.saturating_add(1);
            if height as usize > MAX_DEPTH {
                return Err(self.too_deep(span));
            }
            ty = self.builder.alloc_type(TypeExpr {
                kind: TypeExprKind::List(ty),
                span,
            });
        }
        Ok((ty, height))
    }

    /// Parses a type that is neither a union nor a list type written `T[]`,
    /// unless in parentheses: a name, an applied type, a record type, a
    /// function type or a parenthesised type. Returns it and its height, as
    /// [`Parser::parse_type`] does.
    fn parse_type_atom(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let token = self.peek();
        match token.kind {
            TokenKind::Ident(name) if self.peek_at(1).kind == TokenKind::Lt => {
                // Applied types nest by recursion, so they count their
                // depth.
                self.enter()?;
                let applied = self.parse_applied_type(name, token.span);
                self.depth -= 1;
                applied
            }
            TokenKind::Ident(_) if self.peek_at(1).kind == TokenKind::Dot => self.parse_type_path(),
            TokenKind::Ident(name) => {
                self.advance();
                let id = self.builder.alloc_type(TypeExpr {
                    kind: TypeExprKind::Named(name),
                    span: token.span,
                });
                Ok((id, LEAF_HEIGHT))
            }
            // Record and function types nest by recursion, so they count
            // their depth.
            TokenKind::LBrace => {
                self.enter()?;
                let record = self.parse_record_type();
                self.depth -= 1;
                record
            }
            TokenKind::Fn => {
                self.enter()?;
                let function = self.parse_fn_type();
                self.depth -= 1;
                function
            }
            // Parenthesised types nest by recursion too.
            TokenKind::LParen => {
                self.enter()?;
                let inner = self.parse_paren_type();
                self.depth -= 1;
                inner
            }
            TokenKind::Mut => Err(self.misplaced_mut(token, None)),
            _ => Err(self.error("expected a type", error_span(token))),
        }
    }

    /// Parses a qualified type name, `a.b` or `a.b.c`, starting at its first
    /// name, an identifier directly followed by `.`: identifiers separated by
    /// `.`, each `.` followed by an identifier ("expected a type's name after
    /// `.`" otherwise). Returns it, a leaf one level tall.
    fn parse_type_path(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let base = self.path.len();
        let mut token = self.peek();
        loop {
            let TokenKind::Ident(name) = token.kind else {
                self.path.truncate(base);
                return Err(self.error("expected a type's name after `.`", error_span(token)));
            };
            self.advance();
            self.path.push(PathSegment {
                kind: SegmentKind::Name(name),
                span: token.span,
            });
            if self.peek().kind != TokenKind::Dot {
                break;
            }
            self.advance();
            token = self.peek();
        }
        let span = path_span(self.path.get(base..).unwrap_or_default());
        let (path, _) = self.finish_path(base);
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Path(path),
            span,
        });
        Ok((id, LEAF_HEIGHT))
    }

    /// Parses a parenthesised type, starting at its `(`: the `(`, newlines,
    /// the type, and the `)`. Returns the type inside itself, with a height
    /// one more than its own, since the parentheses leave no node of their
    /// own.
    fn parse_paren_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let open = self.peek();
        self.advance();
        self.skip_newlines();
        let (inner, inner_height) = self.parse_type()?;
        let close = self.peek();
        if close.kind != TokenKind::RParen {
            return Err(self.report(
                Diagnostic::error("expected `)` to close the type", error_span(close))
                    .with_context(open.span, "unclosed `(`"),
            ));
        }
        self.advance();
        let height = inner_height.saturating_add(1);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(Span::join(open.span, close.span)));
        }
        Ok((inner, height))
    }

    /// Parses the applied type whose name, `name` at `name_span`, is the
    /// current token and is followed by `<`: the `<`, newlines, the type
    /// argument, and the `>`, which must follow the argument directly.
    fn parse_applied_type(
        &mut self,
        name: Name,
        name_span: Span,
    ) -> ParseResult<(TypeExprId, u32)> {
        self.advance();
        let open = self.peek();
        self.advance();
        self.skip_newlines();
        let (arg, arg_height) = self.parse_type()?;
        let close = self.peek();
        if close.kind != TokenKind::Gt {
            return Err(self.report(
                Diagnostic::error("expected `>` to close the type argument", error_span(close))
                    .with_context(open.span, "unclosed `<`"),
            ));
        }
        self.advance();
        let span = Span::join(name_span, close.span);
        let height = arg_height.saturating_add(1);
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Applied {
                name,
                span: name_span,
                arg,
            },
            span,
        });
        Ok((id, height))
    }

    /// Parses a function type, starting at its `fn`: `(`, parameter types
    /// separated by `,` with an optional trailing `,`, each `NAME: TYPE` or
    /// `TYPE`, `)`, the marker `:` or `->`, and the return type.
    fn parse_fn_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let keyword = self.peek();
        self.advance();
        let open = self.expect_fn_paren()?;
        let base = self.param_ty_stack.len();
        let mut tallest = 0;
        // A parameter that fails is skipped to its `,`, and the list goes
        // on, to fail at its end (see [Errors and
        // recovery](#errors-and-recovery)).
        let mut failed = None;
        loop {
            let start = self.pos;
            if self.peek().kind == TokenKind::RParen {
                self.advance();
                break;
            }
            match self.parse_param_ty(open, &mut tallest) {
                Ok(true) => break,
                Ok(false) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RParen) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        }
        if let Some(reported) = failed {
            return Err(reported);
        }
        let params = self.builder.push_param_tys(&self.param_ty_stack[base..]);
        self.param_ty_stack.truncate(base);
        let (effect, _) = self.return_marker()?;
        let (ret, ret_height) = self.parse_type()?;
        // The return type, a union included, and then its meta, if `META`
        // and a marker follow it.
        let (meta, end, meta_height) = match self.meta_marker() {
            Some(meta_effect) => {
                self.advance();
                self.advance();
                self.skip_newlines();
                let (ty, height) = self.parse_type()?;
                let meta = MetaTy {
                    effect: meta_effect,
                    ty,
                };
                (Some(meta), ty, height)
            }
            None => (None, ret, 0),
        };
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Fn {
                params,
                ret,
                effect,
                meta,
            },
            span: Span::join(keyword.span, self.type_span(end)),
        });
        Ok((id, tallest.max(ret_height).max(meta_height) + 1))
    }

    /// Parses a parameter of a function type whose parameter list `open`
    /// opens, `NAME: TYPE` or `TYPE`, and the `,` or the `)` after it,
    /// raising `tallest` to the height of its type. Returns whether the `)`
    /// ends the list.
    fn parse_param_ty(&mut self, open: Token, tallest: &mut u32) -> ParseResult<bool> {
        let token = self.peek();
        let name = match (token.kind, self.peek_at(1).kind) {
            (TokenKind::Ident(name), TokenKind::Colon) => {
                self.advance();
                self.advance();
                Some(name)
            }
            _ => None,
        };
        let (ty, height) = self.parse_type()?;
        *tallest = (*tallest).max(height);
        // Only a function expression's parameter has a default.
        let eq = self.peek();
        if eq.kind == TokenKind::Eq {
            return Err(self.error(MISPLACED_DEFAULT, eq.span));
        }
        self.param_ty_stack.push(ParamTy { name, ty });
        self.list_separator(open)
    }

    /// The effect of the meta whose marker the current token begins, if it
    /// does: `META` followed by `:`, `->>` or `->` (see [Types](#types)).
    /// `META` followed by anything else is a name.
    fn meta_marker(&self) -> Option<Effect> {
        let TokenKind::Ident(name) = self.peek().kind else {
            return None;
        };
        if self.names.get(name) != META {
            return None;
        }
        match self.peek_at(1).kind {
            TokenKind::Colon => Some(Effect::Pure),
            TokenKind::MemoArrow => Some(Effect::Memo),
            TokenKind::Arrow => Some(Effect::Impure),
            _ => None,
        }
    }

    /// Consumes the `(` after `fn`, returning it, or reports what is there
    /// instead.
    fn expect_fn_paren(&mut self) -> ParseResult<Token> {
        let open = self.peek();
        if open.kind == TokenKind::Mut {
            return Err(self.misplaced_mut(open, None));
        }
        if open.kind != TokenKind::LParen {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `(` after `fn`, found {found}"),
                error_span(open),
            ));
        }
        self.advance();
        Ok(open)
    }

    /// After an entry of a parenthesised list opened by `open`, consumes a
    /// `,` or the closing `)`, returning whether it was the `)`; anything
    /// else is an error.
    fn list_separator(&mut self, open: Token) -> ParseResult<bool> {
        let separator = self.peek();
        match separator.kind {
            TokenKind::Comma => {
                self.advance();
                Ok(false)
            }
            TokenKind::RParen => {
                self.advance();
                Ok(true)
            }
            _ => {
                let found = self.describe(separator);
                Err(self.report_close(
                    Diagnostic::error(
                        format!("expected `,` or `)`, found {found}"),
                        error_span(separator),
                    )
                    .with_context(open.span, "unclosed `(`"),
                ))
            }
        }
    }

    /// The field access of `record` whose `.`, `dot`, is followed by no
    /// name but by `next`: "expected a field name after `.`", and, as an
    /// editor leaves one half-typed, if `next` may end it there
    /// ([`Parser::ends_unfinished`]), a [`ExprKind::NoField`] of the record,
    /// spanning it through the `.`; else the error.
    fn no_field(&mut self, record: ExprId, dot: Token, next: Token) -> ParseResult<ExprId> {
        let reported = self.error("expected a field name after `.`", error_span(next));
        if !self.ends_unfinished(next.kind) {
            return Err(reported);
        }
        let span = Span::join(self.span_of(record), dot.span);
        let height = self.height_of(record) + 1;
        self.alloc(ExprKind::NoField(record), span, height)
    }

    /// Whether a field access whose `.` is followed by the token of `kind`,
    /// no name, ends there as an editor leaves one half-typed, `a.` (see
    /// [Errors and recovery](#errors-and-recovery)): what may end an
    /// expression where it is, in the innermost bracket open, the lexer's
    /// way ([`nest`]): the end of a line or of the input, a `;` or the `}`
    /// of a block (which closes a `(` or `[` open in it too), the `)` or `]`
    /// of the bracket open, or a `,` in one. Costs a walk of the tokens
    /// before it, on an error only.
    fn ends_unfinished(&self, kind: TokenKind) -> bool {
        let mut nesting = Nesting::default();
        for index in 0..self.pos {
            nest(&mut nesting, self.token_at(index).kind);
        }
        let open = &nesting.open;
        let inner = open.last().copied();
        match kind {
            TokenKind::Eof => true,
            TokenKind::Newline | TokenKind::Semi => matches!(inner, None | Some(Open::Brace)),
            TokenKind::RBrace => {
                inner != Some(Open::Str)
                    && open
                        .get(nesting.strings.last().map_or(0, |&at| at + 1)..)
                        .is_some_and(|inside| inside.contains(&Open::Brace))
            }
            TokenKind::RParen => inner == Some(Open::Paren),
            TokenKind::RBracket => inner == Some(Open::Square),
            TokenKind::Comma => matches!(inner, Some(Open::Paren | Open::Square)),
            _ => false,
        }
    }

    /// Reports `diagnostic`, of a list's separator or a closing bracket
    /// missing at a token, unless an error is reported at that token
    /// already: a field access's with no name before it, `f(a.` at the end
    /// of the input or before the `}` around it, whose error is there, and
    /// the bracket is reported unclosed by it.
    fn report_close(&mut self, diagnostic: Diagnostic) -> Reported {
        if self
            .diagnostics
            .last()
            .is_some_and(|last| last.span == diagnostic.span)
        {
            return Reported;
        }
        self.report(diagnostic)
    }

    /// Consumes the return marker of a function or function type, `:` for a
    /// pure one, `->>` for a memoised one and `->` for an impure one, and
    /// the newlines after it. Returns the effect it declares, and the
    /// marker's span.
    fn return_marker(&mut self) -> ParseResult<(Effect, Span)> {
        let marker = self.peek();
        let effect = match marker.kind {
            TokenKind::Colon => Effect::Pure,
            TokenKind::MemoArrow => Effect::Memo,
            TokenKind::Arrow => Effect::Impure,
            _ => {
                return Err(self.report(
                    Diagnostic::error(MISSING_RETURN_TYPE, error_span(marker))
                        .with_note(RETURN_TYPE_NOTE),
                ));
            }
        };
        self.advance();
        self.skip_newlines();
        Ok((effect, marker.span))
    }

    /// Parses a record type, starting at its `{`. Newlines anywhere inside
    /// are skipped.
    fn parse_record_type(&mut self) -> ParseResult<(TypeExprId, u32)> {
        let open = self.peek();
        self.advance();
        let base = self.field_ty_stack.len();
        let mut tallest = 0;
        // A field that fails is skipped to its `,`, and the type goes on, to
        // fail at its end (see [Errors and recovery](#errors-and-recovery)).
        let mut failed = None;
        let close = loop {
            self.skip_newlines();
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RBrace {
                self.advance();
                break token;
            }
            match self.parse_field_ty(&mut tallest) {
                Ok(Some(close)) => break close,
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RBrace) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        let fields = self.builder.push_field_tys(&self.field_ty_stack[base..]);
        self.field_ty_stack.truncate(base);
        let id = self.builder.alloc_type(TypeExpr {
            kind: TypeExprKind::Record(fields),
            span: Span::join(open.span, close.span),
        });
        Ok((id, tallest + 1))
    }

    /// Parses an entry of a record type, a field, `NAME: TYPE` with its
    /// default if it has one, a spread, `...TYPE`, or a rest-field, `*:
    /// TYPE`, and the `,` or the `}` after it, newlines skipped, raising
    /// `tallest` to the height of its type and its default. Returns the `}`
    /// if it ends the type. A rest-field has no default: an `=` after its
    /// type is "a rest-field cannot have a default".
    fn parse_field_ty(&mut self, tallest: &mut u32) -> ParseResult<Option<Token>> {
        let token = self.peek();
        let kind = match token.kind {
            // A spread, `...TYPE`.
            TokenKind::DotDotDot => {
                self.advance();
                self.skip_newlines();
                FieldTyKind::Spread
            }
            // A rest-field, `*: TYPE`.
            TokenKind::Star => {
                self.advance();
                self.skip_newlines();
                self.expect_field_colon("expected `:` after `*`")?;
                FieldTyKind::Rest
            }
            _ => {
                let Some(name) = self.field_name()? else {
                    return Err(
                        self.error("expected a field name in record type", error_span(token))
                    );
                };
                self.skip_newlines();
                self.expect_field_colon("expected `:` after field name")?;
                FieldTyKind::Field(name)
            }
        };
        let (ty, height) = self.parse_type()?;
        *tallest = (*tallest).max(height);
        self.skip_newlines();
        // A field's default; a spread has none, nor may a rest-field.
        let default = match kind {
            FieldTyKind::Field(_) => self.parse_field_default()?,
            FieldTyKind::Rest => {
                let eq = self.peek();
                if eq.kind == TokenKind::Eq {
                    return Err(self.error("a rest-field cannot have a default", error_span(eq)));
                }
                None
            }
            FieldTyKind::Spread => None,
        };
        if let Some(default) = default {
            *tallest = (*tallest).max(self.height_of(default));
        }
        self.field_ty_stack.push(FieldTy {
            kind,
            span: token.span,
            ty,
            default: default.map(FieldDefault::Expr),
        });
        self.skip_newlines();
        let separator = self.peek();
        match separator.kind {
            // A `}` after it ends the type at the top of the loop.
            TokenKind::Comma => {
                self.advance();
                Ok(None)
            }
            TokenKind::RBrace => {
                self.advance();
                Ok(Some(separator))
            }
            _ => Err(self.error("expected `,` or `}`", error_span(separator))),
        }
    }

    /// Consumes the `:` after a record type's field name or `*`, and the
    /// newlines after it; `message` at the current token if it is not one.
    fn expect_field_colon(&mut self, message: &str) -> ParseResult<()> {
        let colon = self.peek();
        if colon.kind != TokenKind::Colon {
            return Err(self.error(message, error_span(colon)));
        }
        self.advance();
        self.skip_newlines();
        Ok(())
    }

    /// Parses a field's default in a record type, `= EXPR`, if the current
    /// token, after the field's type and any newlines, is its `=`: the
    /// newlines after the `=`, and the expression, any expression, parsed
    /// at minimum binding power 0 as a parameter's default is, inside the
    /// type's braces, where a `{` after an expression on its line starts a
    /// trailing block; `None` if no `=` follows.
    fn parse_field_default(&mut self) -> ParseResult<Option<ExprId>> {
        if self.peek().kind != TokenKind::Eq {
            return Ok(None);
        }
        self.advance();
        self.skip_newlines();
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let default = self.parse_expr(0);
        self.braces = outer;
        default.map(Some)
    }

    /// Parses an expression whose binary operators all have a left binding
    /// power of at least `min_bp`.
    fn parse_expr(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        self.enter()?;
        let result = self.parse_expr_inner(min_bp);
        self.depth -= 1;
        result
    }

    /// Records one more level of recursion, failing if it would exceed
    /// [`MAX_DEPTH`]. Every successful call is paired with a decrement by
    /// its caller.
    fn enter(&mut self) -> ParseResult<()> {
        if self.depth >= MAX_DEPTH {
            let span = self.peek().span;
            return Err(self.too_deep(span));
        }
        self.depth += 1;
        Ok(())
    }

    fn parse_expr_inner(&mut self, min_bp: u8) -> ParseResult<ExprId> {
        let mut lhs = self.parse_prefix()?;
        loop {
            let token = self.peek();
            let starts_call = match token.kind {
                TokenKind::LParen => true,
                TokenKind::LBrace => self.braces == Braces::Free,
                _ => false,
            };
            if starts_call {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_call(lhs)?;
                continue;
            }
            if token.kind == TokenKind::LBracket {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_index(lhs)?;
                continue;
            }
            if token.kind == TokenKind::Dot {
                if FIELD_BP < min_bp {
                    break;
                }
                self.advance();
                let name_token = self.peek();
                let Some(name) = self.field_name()? else {
                    lhs = self.no_field(lhs, token, name_token)?;
                    continue;
                };
                let span = Span::join(self.span_of(lhs), name_token.span);
                let height = self.height_of(lhs) + 1;
                lhs = self.alloc(ExprKind::Field { expr: lhs, name }, span, height)?;
                continue;
            }
            if token.kind == TokenKind::Colon {
                if ASCRIBE_BP < min_bp {
                    break;
                }
                self.advance();
                self.skip_newlines();
                let (ty, ty_height) = self.parse_type()?;
                let span = Span::join(self.span_of(lhs), self.type_span(ty));
                let height = self.height_of(lhs).max(ty_height) + 1;
                lhs = self.alloc(ExprKind::Ascribe { expr: lhs, ty }, span, height)?;
                continue;
            }
            if matches!(token.kind, TokenKind::Is | TokenKind::As) {
                let is = token.kind == TokenKind::Is;
                if (if is { IS_BP } else { ASCRIBE_BP }) < min_bp {
                    break;
                }
                lhs = self.parse_type_operator(lhs, is)?;
                continue;
            }
            // A name or a dotted path, then on its line another: a def-type
            // reference, which binds as a field access does.
            if self.is_name_after(self.span_of(lhs), token) && self.is_path_expr(lhs) {
                if FIELD_BP < min_bp {
                    break;
                }
                lhs = self.parse_def_type_ref(lhs)?;
                continue;
            }
            let Some(op) = binary_op(token.kind) else {
                break;
            };
            let (left_bp, right_bp) = binding_power(op);
            if left_bp < min_bp {
                break;
            }
            let op_start = token.span.start;
            self.advance();
            self.skip_newlines();
            let rhs = self.parse_expr(right_bp)?;
            let span = Span::join(self.span_of(lhs), self.span_of(rhs));
            let height = self.height_of(lhs).max(self.height_of(rhs)) + 1;
            lhs = self.alloc(
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                span,
                height,
            )?;
            // The right operand stopped before any further comparison, since
            // its minimum is above comparison's left power; one following
            // directly would chain.
            if op.is_comparison() {
                self.not_chained()?;
            }
            // Likewise for a range, whose right operand stops before any
            // further range.
            if op.is_range() {
                let next = self.peek();
                if let Some(next_op) = binary_op(next.kind).filter(|next| next.is_range()) {
                    let message = format!("`{}` cannot be chained", next_op.symbol());
                    return Err(self.error(message, next.span));
                }
            }
        }
        Ok(lhs)
    }

    /// Parses the postfix `is TYPE`, if `is`, or `as TYPE` otherwise, whose
    /// keyword is the next token, applied to `lhs`: the type is parsed as
    /// an ascription's is, newlines before it skipped. An `is` binds as a
    /// comparison and, like one, may not be followed by a comparison or
    /// another `is` (see [`Parser::not_chained`]).
    fn parse_type_operator(&mut self, lhs: ExprId, is: bool) -> ParseResult<ExprId> {
        self.advance();
        self.skip_newlines();
        let (ty, ty_height) = self.parse_type()?;
        let span = Span::join(self.span_of(lhs), self.type_span(ty));
        let height = self.height_of(lhs).max(ty_height) + 1;
        let kind = if is {
            ExprKind::Is { expr: lhs, ty }
        } else {
            ExprKind::As { expr: lhs, ty }
        };
        let expr = self.alloc(kind, span, height)?;
        if is {
            self.not_chained()?;
        }
        Ok(expr)
    }

    /// After a comparison or an `is`, whose right side stops before any
    /// further comparison or `is`: reports one that follows directly,
    /// "comparison operators cannot be chained", since `a < b < c` would
    /// otherwise mean `(a < b) < c`.
    fn not_chained(&mut self) -> ParseResult<()> {
        let next = self.peek();
        if next.kind == TokenKind::Is || binary_op(next.kind).is_some_and(BinaryOp::is_comparison) {
            return Err(self.report(
                Diagnostic::error("comparison operators cannot be chained", next.span)
                    .with_note("write `a < b and b < c`, or add parentheses"),
            ));
        }
        Ok(())
    }

    /// Parses a literal, a name, a string, a parenthesised expression, a
    /// block, a list literal, an `if`, a `for`, a function expression, a
    /// `lazy` or a prefix operator application. Postfix operators, `.`, calls and
    /// indexes included, are applied by the caller's loop. A `...` here is
    /// not the start of a list's element, and so an error.
    fn parse_prefix(&mut self) -> ParseResult<ExprId> {
        let token = self.peek();
        let kind = match token.kind {
            TokenKind::Int(value) => self.unitless(token, ExprKind::Int(value))?,
            TokenKind::Float(value) => self.unitless(token, ExprKind::Float(value))?,
            TokenKind::Duration(value) => ExprKind::Duration(value),
            TokenKind::Size(value) => ExprKind::Size(value),
            TokenKind::True => ExprKind::Bool(true),
            TokenKind::False => ExprKind::Bool(false),
            TokenKind::Str(id) => ExprKind::Str(id),
            TokenKind::Ident(name) => ExprKind::Name(name),
            TokenKind::StrStart(first) => return self.parse_interpolation(first),
            TokenKind::If => return self.parse_if(),
            TokenKind::For => return self.parse_for(),
            TokenKind::Do => return self.parse_do(),
            TokenKind::Label(_) => return Err(self.misplaced_label(token)),
            TokenKind::LBrace if self.braces == Braces::Header => {
                return Err(self.header_brace(token));
            }
            TokenKind::Fn => return self.parse_fn_expr(),
            TokenKind::Lazy => return self.parse_lazy(),
            TokenKind::Neg => return self.parse_unary(UnaryOp::Neg, PREFIX_BP),
            TokenKind::Await => return self.parse_unary(UnaryOp::Await, PREFIX_BP),
            TokenKind::Not => return self.parse_unary(UnaryOp::Not, NOT_BP),
            TokenKind::LParen => return self.parse_paren(token),
            TokenKind::LBrace => return self.parse_block(),
            TokenKind::LBracket => return self.parse_list(),
            TokenKind::DotDotDot => return Err(self.error(MISPLACED_SPREAD, token.span)),
            TokenKind::Mut => return Err(self.misplaced_mut(token, None)),
            // A lexical error, the lexer's to report: what it is part of
            // fails silently.
            TokenKind::Error => return Err(self.error("invalid token", token.span)),
            TokenKind::Minus
            | TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::SlashSlash
            | TokenKind::Percent
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::EqEqEq
            | TokenKind::BangEqEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Else
            | TokenKind::In
            | TokenKind::Is
            | TokenKind::As
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Return
            | TokenKind::DotDot
            | TokenKind::DotDotEq
            | TokenKind::Let
            | TokenKind::Def
            | TokenKind::Import
            | TokenKind::Colon
            | TokenKind::ColonEq
            | TokenKind::Arrow
            | TokenKind::MemoArrow
            | TokenKind::Eq
            | TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::RBracket
            | TokenKind::StrMid(_)
            | TokenKind::StrEnd(_)
            | TokenKind::Semi
            | TokenKind::Dot
            | TokenKind::Comma
            | TokenKind::Pipe
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_)
            | TokenKind::Newline
            | TokenKind::Eof => {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected expression, found {found}"),
                    error_span(token),
                ));
            }
        };
        self.advance();
        self.alloc(kind, token.span, LEAF_HEIGHT)
    }

    /// Parses a function expression in expression position, starting at
    /// its `fn`. A name after the `fn` is an error here: a named function
    /// is an item.
    fn parse_fn_expr(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let next = self.peek();
        if let TokenKind::Ident(_) = next.kind {
            let found = self.source.slice(next.span);
            return Err(self.report(
                Diagnostic::error(
                    format!("expected `(` after `fn`, found identifier `{found}`"),
                    next.span,
                )
                .with_note(
                    "a named function `fn NAME(...)` is an item; write it on a line of its own, \
                     or drop the name",
                ),
            ));
        }
        self.parse_fn(keyword)
    }

    /// Parses the rest of a function expression whose `fn` keyword,
    /// `keyword`, has been consumed, from its `(`: the parameters, the
    /// return marker and type, and the body, which must start directly
    /// after the return type. The expression spans `keyword` through the
    /// body.
    fn parse_fn(&mut self, keyword: Token) -> ParseResult<ExprId> {
        let open = self.expect_fn_paren()?;
        let base = self.param_stack.len();
        let mut tallest = 0;
        // A parameter that fails is skipped to its `,`, and the list goes
        // on, to fail at its end (see [Errors and
        // recovery](#errors-and-recovery)).
        let mut failed = None;
        loop {
            let start = self.pos;
            if self.peek().kind == TokenKind::RParen {
                self.advance();
                break;
            }
            match self.parse_param(open, &mut tallest) {
                Ok(true) => break,
                Ok(false) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RParen) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        }
        if let Some(reported) = failed {
            return Err(reported);
        }
        let own: Vec<Param> = self.param_stack.drain(base..).collect();
        let (effect, marker) = self.return_marker()?;
        let own = self.with_default_fns(own, (effect, marker));
        let params = self.builder.push_params(&own);
        let (ret, ret_height) = self.parse_type()?;
        // Only a function type has a meta.
        if self.meta_marker().is_some() {
            let span = Span::join(self.peek().span, self.peek_at(1).span);
            return Err(self.error(MISPLACED_META, span));
        }
        let brace = self.peek();
        if brace.kind != TokenKind::LBrace {
            return Err(self.error("expected `{` to start the function body", error_span(brace)));
        }
        let body = self.parse_block()?;
        let height = tallest.max(ret_height).max(self.height_of(body)) + 1;
        let span = Span::join(keyword.span, self.span_of(body));
        let id = self.builder.push_fn(FnDecl {
            params,
            ret: Some(ret),
            effect,
            marker,
            body,
        });
        self.alloc(ExprKind::Fn(id), span, height)
    }

    /// The parameters `params` of a function expression of effect `effect`,
    /// each whose default is no constant ([`crate::ast::is_constant`]) given
    /// its default function (see [Functions and calls](#functions-and-calls)):
    /// a function of the parameters before it, as they are written, with no
    /// defaults of their own, whose result type is the parameter's, whose
    /// effect is the function's, a memoised one's impure, whose marker is
    /// the function's, `marker`, so that an error of its effect points where
    /// the function declares it, and whose body is the default.
    fn with_default_fns(
        &mut self,
        mut params: Vec<Param>,
        (effect, marker): (Effect, Span),
    ) -> Vec<Param> {
        let effect = match effect {
            Effect::Pure => Effect::Pure,
            Effect::Memo | Effect::Impure => Effect::Impure,
        };
        for at in 0..params.len() {
            let Some(default) = params.get(at).and_then(|param| param.default) else {
                continue;
            };
            if crate::ast::is_constant(&self.builder, default) {
                continue;
            }
            let earlier: Vec<Param> = params
                .iter()
                .take(at)
                .map(|param| Param {
                    default: None,
                    default_fn: None,
                    ..*param
                })
                .collect();
            let ty = params.get(at).map(|param| param.ty);
            let earlier = self.builder.push_params(&earlier);
            let function = self.builder.push_fn(FnDecl {
                params: earlier,
                ret: ty,
                effect,
                marker,
                body: default,
            });
            if let Some(param) = params.get_mut(at) {
                param.default_fn = Some(function);
            }
        }
        params
    }

    /// Parses a parameter of a function expression whose parameter list
    /// `open` opens, `NAME: TYPE` and its default, if it has one, and the
    /// `,` or the `)` after it, raising `tallest` to the height of its type
    /// and its default. Returns whether the `)` ends the list.
    fn parse_param(&mut self, open: Token, tallest: &mut u32) -> ParseResult<bool> {
        let token = self.peek();
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Str(_) => return Err(self.error(PLAIN_VARIABLE_NAME, token.span)),
            TokenKind::StrStart(first) => {
                let literal = self.parse_interpolation(first)?;
                let span = self.span_of(literal);
                return Err(self.error(PLAIN_VARIABLE_NAME, span));
            }
            TokenKind::Mut => {
                return Err(self.misplaced_mut(token, Some(MUT_PARAM_NOTE)));
            }
            _ => {
                let found = self.describe(token);
                return Err(self.error(
                    format!("expected a parameter name, found {found}"),
                    error_span(token),
                ));
            }
        };
        self.advance();
        let colon = self.peek();
        if colon.kind != TokenKind::Colon {
            let message = format!(
                "expected `:` and a type after parameter `{}`",
                self.names.get(name)
            );
            return Err(self.report(
                Diagnostic::error(message, error_span(colon))
                    .with_note("parameters must be annotated"),
            ));
        }
        self.advance();
        let (ty, height) = self.parse_type()?;
        *tallest = (*tallest).max(height);
        let default = self.parse_default()?;
        if let Some(default) = default {
            *tallest = (*tallest).max(self.height_of(default));
        }
        self.param_stack.push(Param {
            name,
            span: Span::join(token.span, self.type_span(ty)),
            ty,
            default,
            default_fn: None,
        });
        self.list_separator(open)
    }

    /// Parses a parameter's default, `= EXPR`, if the current token, after
    /// the parameter's type, is its `=`: the expression, any expression,
    /// parsed at minimum binding power 0 as an argument is, inside the
    /// parameters' parentheses; `None` if no `=` follows.
    fn parse_default(&mut self) -> ParseResult<Option<ExprId>> {
        if self.peek().kind != TokenKind::Eq {
            return Ok(None);
        }
        self.advance();
        // A default is inside the parameters' parentheses.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let default = self.parse_expr(0);
        self.braces = outer;
        default.map(Some)
    }

    /// Parses `lazy EXPR`, starting at its `lazy` keyword: the operand,
    /// parsed at minimum binding power 0, becomes the body of a function
    /// with no parameters and no return type, impure, whose marker is the
    /// keyword. The expression spans the keyword through the operand.
    fn parse_lazy(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let body = self.parse_expr(0)?;
        let height = self.height_of(body) + 1;
        let span = Span::join(keyword.span, self.span_of(body));
        let params = self.builder.push_params(&[]);
        let id = self.builder.push_fn(FnDecl {
            params,
            ret: None,
            effect: Effect::Impure,
            marker: keyword.span,
            body,
        });
        self.alloc(ExprKind::Lazy(id), span, height)
    }

    /// Parses the call of `callee`, starting at its `(` or, for a call with
    /// only a trailing block, its `{`: the arguments and any trailing
    /// block. The trailing block must start on the line the argument list
    /// ends on, which the lexer's newline tokens ensure.
    fn parse_call(&mut self, callee: ExprId) -> ParseResult<ExprId> {
        let base = self.arg_stack.len();
        let mut tallest = self.height_of(callee);
        let open = self.peek();
        let mut end = open.span;
        if open.kind == TokenKind::LParen {
            self.advance();
            // An argument list is inside parentheses.
            let outer = std::mem::replace(&mut self.braces, Braces::Free);
            let close = self.parse_arguments(open, &mut tallest);
            self.braces = outer;
            let Some(close) = close? else {
                // Never closed: what was parsed of it is kept, for an
                // editor, through the last token it went on to.
                let args = self.builder.push_args(&self.arg_stack[base..]);
                self.arg_stack.truncate(base);
                let last = self.token_at(self.pos.saturating_sub(1)).span;
                let last = if last.end > open.span.end {
                    last
                } else {
                    open.span
                };
                let span = Span::join(self.span_of(callee), last);
                return self.alloc(ExprKind::Unclosed { callee, args }, span, tallest + 1);
            };
            end = close;
        }
        if self.peek().kind == TokenKind::LBrace && self.braces == Braces::Free {
            let block = self.parse_block()?;
            tallest = tallest.max(self.height_of(block));
            end = self.span_of(block);
            self.arg_stack.push(Arg {
                kind: ArgKind::Trailing,
                value: block,
            });
        }
        let args = self.builder.push_args(&self.arg_stack[base..]);
        self.arg_stack.truncate(base);
        let span = Span::join(self.span_of(callee), end);
        self.alloc(ExprKind::Call { callee, args }, span, tallest + 1)
    }

    /// Parses the arguments of a call after its `(`, `open`, through its
    /// `)`, whose span it returns, pushing each on `arg_stack` and raising
    /// `tallest` to the tallest's height. An identifier directly followed
    /// by `=` names its argument. An argument that fails is skipped to its
    /// `,`, nothing of it left in the tree, and the list goes on, to fail at
    /// its end (see [Errors and recovery](#errors-and-recovery)); but a list
    /// that ends where no `)` is, at the end of the input or at the `}`
    /// around it, which closes it, is the unclosed call's, `None`, the
    /// arguments parsed pushed.
    fn parse_arguments(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Option<Span>> {
        let mut failed = None;
        // Whether a named argument has been read: after one, a name alone
        // is the shorthand, and anything else positional an error.
        let mut named = false;
        loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RParen {
                self.advance();
                return failed.map_or(Ok(Some(token.span)), Err);
            }
            let marks = self.marks();
            let (reported, kept) = match self.parse_argument(open, tallest, &mut named) {
                Ok(Some(close)) => return failed.map_or(Ok(Some(close)), Err),
                Ok(None) => continue,
                Err(Entry::Separator(reported)) => (reported, true),
                Err(Entry::Value(reported)) => (reported, false),
            };
            if !kept {
                self.restore(marks);
            }
            // As `recover_entry`, knowing whether the list ended at its `)`.
            if self.deep {
                return Err(reported);
            }
            match self.skip(
                start,
                Sync::Entry {
                    close: TokenKind::RParen,
                },
            ) {
                Stop::Separator => self.advance(),
                Stop::Close => {
                    self.advance();
                    return Err(reported);
                }
                Stop::Outer => return Ok(None),
            }
            failed = Some(reported);
        }
    }

    /// Parses an argument of a call whose argument list `open` opens, and
    /// the `,` or the `)` after it, raising `tallest` to its height, and
    /// `named` if it is named. Returns the span of the `)` if it ends the
    /// list; if what fails is the separator, the argument is pushed. After a
    /// named argument (`named`), a name alone is the shorthand for `NAME =
    /// NAME` (see [Functions and calls](#functions-and-calls)).
    fn parse_argument(
        &mut self,
        open: Token,
        tallest: &mut u32,
        named: &mut bool,
    ) -> Result<Option<Span>, Entry> {
        let token = self.peek();
        let next = self.peek_at(1).kind;
        let kind = match (token.kind, next) {
            (TokenKind::Ident(name), TokenKind::Eq) => {
                self.advance();
                self.advance();
                ArgKind::Named {
                    name,
                    span: token.span,
                    shorthand: false,
                }
            }
            (TokenKind::Ident(name), TokenKind::Comma | TokenKind::RParen) if *named => {
                ArgKind::Named {
                    name,
                    span: token.span,
                    shorthand: true,
                }
            }
            _ => ArgKind::Positional,
        };
        *named |= matches!(kind, ArgKind::Named { .. });
        let value = self.parse_expr(0).map_err(Entry::Value)?;
        *tallest = (*tallest).max(self.height_of(value));
        self.arg_stack.push(Arg { kind, value });
        let close = self.peek();
        let closed = self.list_separator(open).map_err(Entry::Separator)?;
        Ok(closed.then_some(close.span))
    }

    /// Parses a list literal, starting at its `[`: its elements, each an
    /// expression or `...` and one, separated by `,` with an optional
    /// trailing `,`, through its `]`.
    fn parse_list(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        let base = self.element_stack.len();
        let mut tallest = 0;
        // Elements are inside brackets.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let close = self.parse_elements(open, &mut tallest);
        self.braces = outer;
        let close = close?;
        let elements = self.builder.push_elements(&self.element_stack[base..]);
        self.element_stack.truncate(base);
        self.alloc(
            ExprKind::List(elements),
            Span::join(open.span, close),
            tallest + 1,
        )
    }

    /// Parses the elements of a list literal after its `[`, `open`, through
    /// its `]`, whose span it returns, pushing each on `element_stack` and
    /// raising `tallest` to the tallest's height, a spread's being its
    /// expression's. An element that fails is skipped to its `,`, and the
    /// list goes on, to fail at its end (see [Errors and
    /// recovery](#errors-and-recovery)).
    fn parse_elements(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Span> {
        let mut failed = None;
        loop {
            let start = self.pos;
            let token = self.peek();
            if token.kind == TokenKind::RBracket {
                self.advance();
                return failed.map_or(Ok(token.span), Err);
            }
            match self.parse_element(open, tallest) {
                Ok(Some(close)) => return failed.map_or(Ok(close), Err),
                Ok(None) => {}
                Err(reported) => {
                    if self.recover_entry(start, TokenKind::RBracket) {
                        return Err(reported);
                    }
                    failed = Some(reported);
                }
            }
        }
    }

    /// Parses an element of a list literal that `open` opens, and the `,`
    /// or the `]` after it, raising `tallest` to its height. Returns the
    /// span of the `]` if it ends the list.
    fn parse_element(&mut self, open: Token, tallest: &mut u32) -> ParseResult<Option<Span>> {
        let token = self.peek();
        let spread = if token.kind == TokenKind::DotDotDot {
            self.advance();
            Some(token.span)
        } else {
            None
        };
        let value = self.parse_expr(0)?;
        *tallest = (*tallest).max(self.height_of(value));
        self.element_stack.push(Element { value, spread });
        let separator = self.peek();
        match separator.kind {
            TokenKind::Comma => {
                self.advance();
                Ok(None)
            }
            TokenKind::RBracket => {
                self.advance();
                Ok(Some(separator.span))
            }
            _ => {
                let found = self.describe(separator);
                Err(self.report_close(
                    Diagnostic::error(
                        format!("expected `,` or `]`, found {found}"),
                        error_span(separator),
                    )
                    .with_context(open.span, "unclosed `[`"),
                ))
            }
        }
    }

    /// Parses the index of `list`, starting at its `[` (see
    /// [`Parser::parse_position`]). The index spans `list` through the `]`.
    fn parse_index(&mut self, list: ExprId) -> ParseResult<ExprId> {
        let (index, brackets) = self.parse_position()?;
        let height = self.height_of(list).max(self.height_of(index)) + 1;
        let span = Span::join(self.span_of(list), brackets);
        self.alloc(ExprKind::Index { expr: list, index }, span, height)
    }

    /// Parses the brackets of an index, of an expression or of a path,
    /// starting at the `[`: the position, parsed at minimum binding power
    /// 0, and the `]`. Returns the position and the span of the brackets.
    fn parse_position(&mut self) -> ParseResult<(ExprId, Span)> {
        let open = self.peek();
        self.advance();
        // The position is inside brackets.
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let index = self.parse_expr(0);
        self.braces = outer;
        let index = index?;
        let close = self.peek();
        if close.kind != TokenKind::RBracket {
            let found = self.describe(close);
            return Err(self.report_close(
                Diagnostic::error(format!("expected `]`, found {found}"), error_span(close))
                    .with_context(open.span, "unclosed `[`"),
            ));
        }
        self.advance();
        Ok((index, Span::join(open.span, close.span)))
    }

    /// Parses a parenthesised expression, starting at its `(`, `open`, the
    /// current token.
    fn parse_paren(&mut self, open: Token) -> ParseResult<ExprId> {
        self.advance();
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let inner = self.parse_expr(0);
        self.braces = outer;
        let inner = inner?;
        let close = self.peek();
        if close.kind != TokenKind::RParen {
            let found = self.describe(close);
            return Err(self.report_close(
                Diagnostic::error(format!("expected `)`, found {found}"), error_span(close))
                    .with_context(open.span, "unclosed `(`"),
            ));
        }
        self.advance();
        let height = self.height_of(inner) + 1;
        self.alloc(
            ExprKind::Paren(inner),
            Span::join(open.span, close.span),
            height,
        )
    }

    /// Parses a prefix operator `op`, the current token, and its operand,
    /// parsed with minimum binding power `operand_bp`.
    fn parse_unary(&mut self, op: UnaryOp, operand_bp: u8) -> ParseResult<ExprId> {
        let token = self.peek();
        self.advance();
        let operand = self.parse_expr(operand_bp)?;
        let span = Span::join(token.span, self.span_of(operand));
        let height = self.height_of(operand) + 1;
        self.alloc(ExprKind::Unary { op, operand }, span, height)
    }

    /// Parses a block `{ ... }`, starting at its `{`, with its label, if one
    /// is written directly after the `{`. A record block, one with a field
    /// or a spread item, cannot be labelled: "a record block cannot be
    /// labelled", at the label.
    fn parse_block(&mut self) -> ParseResult<ExprId> {
        let open = self.peek();
        self.advance();
        let label = self.parse_label(open)?;
        self.enter()?;
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let (items, tallest) = self.parse_body(Some(open.span));
        self.braces = outer;
        self.depth -= 1;
        let label = match label {
            Some(label) if self.is_record_body(items) => {
                return Err(self.error(LABELLED_RECORD, label.span));
            }
            Some(label) => Some(self.builder.push_label(label)),
            None => None,
        };
        // `parse_body` returns at the matching `}`, or at the end of the
        // input, which it has reported.
        let close = self.peek();
        if close.kind == TokenKind::RBrace {
            self.advance();
        }
        self.alloc(
            ExprKind::Block { items, label },
            Span::join(open.span, close.span),
            tallest + 1,
        )
    }

    /// Whether the body `items`, just parsed, has a field or a spread item,
    /// which makes it a record block.
    fn is_record_body(&self, items: ItemRange) -> bool {
        self.builder.items(items).iter().any(|&id| {
            matches!(
                self.builder.item(id).kind,
                ItemKind::Field { .. } | ItemKind::Spread(_)
            )
        })
    }

    /// Parses an `if` expression, starting at its `if`.
    fn parse_if(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        // A `{` after the condition is the `if`'s block, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.braces, Braces::NoTrailing);
        let cond = self.parse_expr(0);
        self.braces = outer;
        let cond = cond?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            return Err(self.error("expected `{` after `if` condition", error_span(open)));
        }
        let then = self.parse_block()?;
        // An `else` may start a later line. If none does, the newlines are
        // put back to end the item.
        let after_then = self.pos;
        self.skip_newlines();
        let (else_, has_else) = if self.peek().kind == TokenKind::Else {
            self.advance();
            let next = self.peek();
            let else_ = match next.kind {
                TokenKind::LBrace => self.parse_block()?,
                // `else if` recurses directly rather than through
                // `parse_expr`, so it counts its own depth.
                TokenKind::If => {
                    self.enter()?;
                    let nested = self.parse_if();
                    self.depth -= 1;
                    nested?
                }
                _ => {
                    return Err(self.error("expected `{` or `if` after `else`", error_span(next)));
                }
            };
            (else_, true)
        } else {
            self.pos = after_then;
            let end = self.span_of(then).end;
            let empty = self.builder.push_item_list(&[]);
            let block = ExprKind::Block {
                items: empty,
                label: None,
            };
            let else_ = self.alloc(block, Span::new(end, end), LEAF_HEIGHT)?;
            (else_, false)
        };
        let span = Span::join(keyword.span, self.span_of(else_));
        let height = self
            .height_of(cond)
            .max(self.height_of(then))
            .max(self.height_of(else_))
            + 1;
        self.alloc(
            ExprKind::If {
                cond,
                then,
                else_,
                has_else,
            },
            span,
            height,
        )
    }

    /// Parses a `for` expression, starting at its `for`, and its label, if
    /// it has one: then, as what follows says (see [Loops](#loops)), an
    /// iteration, `for BLOCK`, or a header and a body. The expression spans
    /// `for` through the body.
    fn parse_for(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let label = self.parse_label(keyword)?;
        let next = self.peek();
        match (next.kind, self.peek_at(1).kind) {
            (TokenKind::Ident(name), TokenKind::In) => {
                self.advance();
                self.advance();
                self.parse_each(keyword, label, (name, next.span))
            }
            (TokenKind::LBrace, _) => {
                let body = self.parse_block()?;
                let empty = self.builder.push_item_list(&[]);
                let head = ForHead::While {
                    before: empty,
                    cond: None,
                    after: empty,
                };
                let height = self.height_of(body) + 1;
                self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
            }
            _ => self.parse_header(keyword, label),
        }
    }

    /// Parses the rest of an iteration, `for NAME in ITERABLE (if GUARD)?
    /// BLOCK`, whose keyword, label and variable's name with its span, `for
    /// NAME in` having been consumed, are `keyword`, `label` and `name`: the
    /// iterable, an optional guard, `if` and an expression, and the body, a
    /// block. The iterable and the guard are parsed as an `if`'s condition
    /// is, at minimum binding power 0 with no trailing block, so a `{` after
    /// either is the body; an `if` after the complete iterable, which no
    /// expression continues with, begins the guard.
    fn parse_each(
        &mut self,
        keyword: Token,
        label: Option<Label>,
        (name, name_span): (Name, Span),
    ) -> ParseResult<ExprId> {
        // A `{` after the iterable or the guard is the body, not a trailing
        // block of a call.
        let outer = std::mem::replace(&mut self.braces, Braces::NoTrailing);
        let parts = self.parse_for_head();
        self.braces = outer;
        let (iterable, guard) = parts?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            let message = match guard {
                Some(_) => "expected `{` after `for` guard",
                None => "expected `{` or `if` after `for` iterable",
            };
            return Err(self.error(message, error_span(open)));
        }
        let body = self.parse_block()?;
        let height = self
            .height_of(iterable)
            .max(guard.map_or(0, |guard| self.height_of(guard)))
            .max(self.height_of(body))
            + 1;
        let head = ForHead::Each {
            name,
            name_span,
            iterable,
            guard,
        };
        self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
    }

    /// Allocates the loop whose keyword is `keyword`, with `label`, `head`
    /// and `body`, spanning `keyword` through `end`, of `height`.
    fn finish_loop(
        &mut self,
        keyword: Token,
        (label, head, body): (Option<Label>, ForHead, ExprId),
        end: Span,
        height: u32,
    ) -> ParseResult<ExprId> {
        let span = Span::join(keyword.span, end);
        let id = self.builder.push_for(ForDecl { label, head, body });
        self.alloc(ExprKind::For(id), span, height)
    }

    /// Parses the header and the body of a condition loop whose keyword
    /// and label are `keyword` and `label`, starting at the header's first
    /// part (see [Loops](#loops)). The header's expressions are parsed with
    /// [`Braces::Header`], so that a `{` anywhere in it, but inside
    /// brackets, is the body.
    fn parse_header(&mut self, keyword: Token, label: Option<Label>) -> ParseResult<ExprId> {
        let outer = std::mem::replace(&mut self.braces, Braces::Header);
        // An error in a part is the header's (see [Errors and
        // recovery](#errors-and-recovery)).
        let recovery = std::mem::replace(&mut self.recovery, Recovery::None);
        let header = self.parse_header_parts();
        self.recovery = recovery;
        self.braces = outer;
        let (head, tallest) = header?;
        // The header ends only at a `{`.
        let body = self.parse_block()?;
        let height = tallest.max(self.height_of(body)) + 1;
        self.finish_loop(keyword, (label, head, body), self.span_of(body), height)
    }

    /// Parses the parts of a `for` header, up to the `{` of the body, not
    /// consumed: items and one condition, separated by `;`, after each of
    /// which newlines are skipped. Returns the loop's head and the height of
    /// its tallest part.
    fn parse_header_parts(&mut self) -> ParseResult<(ForHead, u32)> {
        let base = self.item_stack.len();
        let first = self.peek();
        let mut cond: Option<ExprId> = None;
        // Where the items after the condition start on the item stack.
        let mut split = base;
        let mut tallest = 0;
        // The span of the last part parsed.
        let mut last;
        let mut parts = 0;
        loop {
            let start = self.pos;
            let token = self.peek();
            // An attribute applies to no part of a header.
            if matches!(
                token.kind,
                TokenKind::Attribute(_) | TokenKind::InnerAttribute(_)
            ) {
                return Err(self.misplaced_attribute(token));
            }
            if token.kind == TokenKind::Let || self.starts_path_item() {
                let (item, height) = self.parse_header_item(token, cond.is_some())?;
                self.item_stack.push(item);
                tallest = tallest.max(height);
                last = self.builder.item(item).span;
            } else {
                let expr = self.parse_expr(0)?;
                if let Some(first) = cond {
                    return Err(self.report(
                        Diagnostic::error("a `for` header has one condition", self.span_of(expr))
                            .with_context(self.span_of(first), "the condition is here"),
                    ));
                }
                cond = Some(expr);
                split = self.item_stack.len();
                tallest = tallest.max(self.height_of(expr));
                last = self.span_of(expr);
            }
            parts += 1;
            match self.peek().kind {
                TokenKind::Semi => {
                    self.advance();
                    self.skip_newlines();
                }
                TokenKind::LBrace => break,
                _ => return Err(self.header_error(start, parts == 1 && cond.is_some())),
            }
        }
        let Some(cond) = cond else {
            return Err(self.report(
                Diagnostic::error(
                    "a `for` header needs a condition",
                    Span::join(first.span, last),
                )
                .with_note("write `true` to loop until a `break`"),
            ));
        };
        let before = self.builder.push_item_list(&self.item_stack[base..split]);
        let after = self.builder.push_item_list(&self.item_stack[split..]);
        self.item_stack.truncate(base);
        let head = ForHead::While {
            before,
            cond: Some(cond),
            after,
        };
        Ok((head, tallest))
    }

    /// Parses an item of a `for` header, starting at `token`, a `let` or
    /// the first segment of a path, after the header's condition if
    /// `after_cond`: a `let`, only before the condition, or a `:=`. Returns
    /// it and its height.
    fn parse_header_item(&mut self, token: Token, after_cond: bool) -> ParseResult<(ItemId, u32)> {
        let (item, height) = if token.kind == TokenKind::Let {
            let (kind, value, ty_height, shape) = self.parse_let(token)?;
            self.finish_item(token, (kind, value), ty_height, shape)?
        } else {
            let (kind, value, shape, ty_height) = self.parse_path_item()?;
            self.finish_item(token, (kind, value), ty_height, shape)?
        };
        let Item { kind, span } = *self.builder.item(item);
        match kind {
            ItemKind::Let { .. } if after_cond => Err(self.report(
                Diagnostic::error(
                    "a `let` after the condition would be new on every iteration and seen by \
                     nothing",
                    span,
                )
                .with_note("declare it before the condition and rebind it here with `:=`"),
            )),
            ItemKind::Field { .. } => Err(self.report(
                Diagnostic::error("a `for` header cannot define a field", span)
                    .with_note("declare a variable with `let`, or rebind one with `:=`"),
            )),
            _ => Ok((item, height)),
        }
    }

    /// The error for the token after a part of a `for` header that is
    /// neither `;` nor the body's `{`, the part having started at token
    /// number `start`. When the part is the header's only part so far and
    /// an expression (`alone`), it may be meant as the variable of an
    /// iteration: followed by `in`, it is "expected a name after `for`" at
    /// the part, unless it starts with a name, when it is "expected `in`
    /// after the `for` variable" at the token after the name, or a string
    /// literal, when it is the error for a variable's name that is not
    /// plain; and a lone name followed by anything on its line is
    /// "expected `in` after the `for` variable" at what follows. Anything
    /// else is "expected `;` or `{` in a `for` header".
    fn header_error(&mut self, start: usize, alone: bool) -> Reported {
        let token = self.peek();
        let found = self.describe(token);
        let (first, second) = (self.token_at(start), self.token_at(start + 1));
        let after_name = |token: Token| {
            format!(
                "expected `in` after the `for` variable, found {}",
                self.describe(token)
            )
        };
        if alone && token.kind == TokenKind::In {
            let part = Span::join(first.span, self.token_at(self.pos - 1).span);
            return match first.kind {
                TokenKind::Ident(_) => self.error(after_name(second), error_span(second)),
                TokenKind::Str(_) | TokenKind::StrStart(_) => self.error(PLAIN_VARIABLE_NAME, part),
                _ => self.error("expected a name after `for`", part),
            };
        }
        let lone_name = matches!(first.kind, TokenKind::Ident(_)) && self.pos == start + 1;
        if alone && lone_name && !matches!(token.kind, TokenKind::Newline | TokenKind::Eof) {
            return self.error(after_name(token), error_span(token));
        }
        self.error(
            format!("expected `;` or `{{` in a `for` header, found {found}"),
            error_span(token),
        )
    }

    /// Parses a `do` loop, starting at its `do`: its label, if it has one,
    /// its body, a block, and `for`, which must follow the body's `}` on
    /// the same line, and the condition, an ordinary expression at minimum
    /// binding power 0, which ends as an expression statement does. Only a
    /// condition may follow the `for`: a label, a `let` or a name followed
    /// by `in` there is an error. The expression spans `do` through the
    /// condition.
    fn parse_do(&mut self) -> ParseResult<ExprId> {
        let keyword = self.peek();
        self.advance();
        let label = self.parse_label(keyword)?;
        let open = self.peek();
        if open.kind != TokenKind::LBrace {
            let found = self.describe(open);
            return Err(self.error(
                format!("expected `{{` after `do`, found {found}"),
                error_span(open),
            ));
        }
        let body = self.parse_block()?;
        let for_token = self.peek();
        if for_token.kind != TokenKind::For {
            return Err(self.error(MISSING_DO_FOR, error_span(for_token)));
        }
        self.advance();
        let next = self.peek();
        let misplaced = match next.kind {
            TokenKind::Label(_) | TokenKind::Let => Some(next.span),
            TokenKind::Ident(_) if self.peek_at(1).kind == TokenKind::In => {
                Some(Span::join(next.span, self.peek_at(1).span))
            }
            _ => None,
        };
        if let Some(span) = misplaced {
            return Err(
                self.report(Diagnostic::error(DO_CONDITION_ONLY, span).with_note(
                    "a `do` loop runs its body, then its condition, and again while it is true",
                )),
            );
        }
        let cond = self.parse_expr(0)?;
        let height = self.height_of(body).max(self.height_of(cond)) + 1;
        let head = ForHead::Do { cond };
        self.finish_loop(keyword, (label, head, body), self.span_of(cond), height)
    }

    /// Parses the iterable of a `for` and its guard, if an `if` follows the
    /// iterable, each at minimum binding power 0.
    fn parse_for_head(&mut self) -> ParseResult<(ExprId, Option<ExprId>)> {
        let iterable = self.parse_expr(0)?;
        if self.peek().kind != TokenKind::If {
            return Ok((iterable, None));
        }
        self.advance();
        let guard = self.parse_expr(0)?;
        Ok((iterable, Some(guard)))
    }

    /// Parses an interpolated string, starting at its
    /// [`TokenKind::StrStart`], whose text is `first`.
    fn parse_interpolation(&mut self, first: StrId) -> ParseResult<ExprId> {
        let start = self.peek();
        self.advance();
        let base = self.part_stack.len();
        self.part_stack.push(StrPart::Lit(first));
        let mut tallest = 0;
        // The `{` the current interpolation opened with: the last byte of
        // the token before it.
        let mut open = start.span;
        // An expression that fails is skipped to the piece after it, and the
        // string goes on, to fail at its end (see [Errors and
        // recovery](#errors-and-recovery)).
        let mut failed = None;
        let end = loop {
            let part = self.pos;
            if let Err(reported) = self.parse_interpolated(open, &mut tallest) {
                if self.deep || self.skip(part, Sync::Interpolation) != Stop::Close {
                    return Err(reported);
                }
                failed = Some(reported);
            }
            let token = self.peek();
            match token.kind {
                TokenKind::StrMid(text) => {
                    self.advance();
                    self.part_stack.push(StrPart::Lit(text));
                    open = token.span;
                }
                TokenKind::StrEnd(text) => {
                    self.advance();
                    self.part_stack.push(StrPart::Lit(text));
                    break token.span;
                }
                // Neither follows an expression that parsed.
                _ => return Err(failed.unwrap_or_else(|| self.error("expected `}`", token.span))),
            }
        };
        if let Some(reported) = failed {
            return Err(reported);
        }
        let parts = self.builder.push_parts(&self.part_stack[base..]);
        self.part_stack.truncate(base);
        self.alloc(
            ExprKind::Interp(parts),
            Span::join(start.span, end),
            tallest + 1,
        )
    }

    /// Parses the expression of an interpolation of a string, opened by the
    /// last byte of `open`, at minimum binding power 0, which the string's
    /// piece after it must follow, raising `tallest` to its height.
    fn parse_interpolated(&mut self, open: Span, tallest: &mut u32) -> ParseResult<()> {
        let outer = std::mem::replace(&mut self.braces, Braces::Free);
        let expr = self.parse_expr(0);
        self.braces = outer;
        let expr = expr?;
        *tallest = (*tallest).max(self.height_of(expr));
        self.part_stack.push(StrPart::Expr(expr));
        let token = self.peek();
        if matches!(token.kind, TokenKind::StrMid(_) | TokenKind::StrEnd(_)) {
            return Ok(());
        }
        let found = self.describe(token);
        let brace = Span::new(open.end.saturating_sub(1), open.end);
        Err(self.report(
            Diagnostic::error(format!("expected `}}`, found {found}"), token.span)
                .with_context(brace, "unclosed `{`"),
        ))
    }

    /// Allocates a node of the given `height`, failing if that height
    /// exceeds [`MAX_DEPTH`].
    fn alloc(&mut self, kind: ExprKind, span: Span, height: u32) -> ParseResult<ExprId> {
        if height as usize > MAX_DEPTH {
            return Err(self.too_deep(span));
        }
        let id = self.builder.alloc(Expr { kind, span });
        self.heights.push(height);
        Ok(id)
    }

    fn height_of(&self, id: ExprId) -> u32 {
        // Every id came from `alloc`, which pushes a height for each node.
        self.heights.get(id.index()).copied().unwrap_or(LEAF_HEIGHT)
    }

    fn too_deep(&mut self, span: Span) -> Reported {
        self.deep = true;
        self.report(
            Diagnostic::error("expression nested too deeply", span)
                .with_note(format!("the limit is {MAX_DEPTH} levels")),
        )
    }

    /// Where the parser's stacks and the builder's arenas are, with its
    /// depth and what a `{` means, for [`Parser::restore`].
    fn marks(&self) -> Marks {
        Marks {
            builder: self.builder.mark(),
            heights: self.heights.len(),
            items: self.item_stack.len(),
            parts: self.part_stack.len(),
            field_tys: self.field_ty_stack.len(),
            params: self.param_stack.len(),
            param_tys: self.param_ty_stack.len(),
            args: self.arg_stack.len(),
            elements: self.element_stack.len(),
            path: self.path.len(),
            depth: self.depth,
            braces: self.braces,
            diagnostics: self.diagnostics.len(),
            syntax: self.builder.syntax(),
            unclosed: self.unclosed,
        }
    }

    /// Puts the parser's stacks and the builder's arenas back where
    /// `marks` says they were, with its depth and what a `{` means:
    /// forgets everything parsed since.
    fn restore(&mut self, marks: Marks) {
        self.builder.truncate(marks.builder);
        self.heights.truncate(marks.heights);
        self.item_stack.truncate(marks.items);
        self.part_stack.truncate(marks.parts);
        self.field_ty_stack.truncate(marks.field_tys);
        self.param_stack.truncate(marks.params);
        self.param_ty_stack.truncate(marks.param_tys);
        self.arg_stack.truncate(marks.args);
        self.element_stack.truncate(marks.elements);
        self.path.truncate(marks.path);
        self.depth = marks.depth;
        self.braces = marks.braces;
    }

    /// After what started at token number `start` failed, `marks` saying
    /// where the parser was then: forgets what it parsed, skips to its end
    /// as `sync` says ([`Parser::skip`]), and returns the span of the
    /// source skipped, from `start`; an empty span at `start` if nothing
    /// was.
    fn recover(&mut self, start: usize, marks: Marks, sync: Sync) -> Span {
        self.deep = false;
        self.restore(marks);
        self.skip(start, sync);
        let first = self.token_at(start).span;
        if self.pos > start {
            Span::join(first, self.token_at(self.pos - 1).span)
        } else {
            Span::new(first.start, first.start)
        }
    }

    /// The error item standing for the item of a body that started at
    /// token number `start` and failed, `marks` saying where the parser
    /// was then: spanning the item and what is skipped after it, as `sync`
    /// says (see [Errors and recovery](#errors-and-recovery)).
    fn error_item(&mut self, start: usize, marks: Marks, sync: Sync) -> ItemId {
        let span = self.recover(start, marks, sync);
        self.builder.alloc_item(Item {
            kind: ItemKind::Error,
            span,
        })
    }

    /// Parses the value of an item of a body, after its `=`, `:=` or
    /// `...`: newlines, then any expression, at minimum binding power 0,
    /// which must be followed by the end of the item. Having gone on to a
    /// line that is an item of its own ([`Parser::item_line`]), it is
    /// parsed again up to that line. If it fails, or something else
    /// follows it, the value is an error node spanning it and what is
    /// skipped to the item's end (see [Errors and
    /// recovery](#errors-and-recovery)), so that the item still declares
    /// what it declares; in a `for` header, which does not recover, it
    /// fails, and the end of the item is not looked for.
    fn parse_value(&mut self) -> ParseResult<ExprId> {
        let Recovery::Body { top } = self.recovery else {
            self.skip_newlines();
            return self.parse_expr(0);
        };
        let start = self.pos;
        let marks = self.marks();
        if let Some(value) = self.attempt_value() {
            return Ok(value);
        }
        // Gone on to a line that is an item of its own: parsed again to the
        // end of its own line, and the next line left for the next item.
        if let Some(line) = self.item_line(start) {
            self.rewind(start, marks);
            let outer = self.line_end.replace(line);
            let value = self.attempt_value();
            self.line_end = outer;
            if let Some(value) = value {
                return Ok(value);
            }
        }
        let span = self.recover(start, marks, Sync::Item { top });
        let id = self.builder.alloc(Expr {
            kind: ExprKind::Error,
            span,
        });
        self.heights.push(LEAF_HEIGHT);
        Ok(id)
    }

    /// Parses a value, as [`Parser::parse_value`] does, and the end of its
    /// item; `None` if either fails.
    fn attempt_value(&mut self) -> Option<ExprId> {
        self.skip_newlines();
        let value = self.parse_expr(0).ok()?;
        self.expect_end_of_item().ok()?;
        Some(value)
    }

    /// After the entry of a list closed by `close` that started at token
    /// number `start` failed: skips to its end, and past the `,` after it,
    /// and returns whether the list has ended there, past `close` or before
    /// what closes something around it (see [Errors and
    /// recovery](#errors-and-recovery)).
    fn recover_entry(&mut self, start: usize, close: TokenKind) -> bool {
        // Nesting too deep is no entry's error: the item's recovers from it,
        // once, rather than every list it is in, each skipping what is
        // nested in it again.
        if self.deep {
            return true;
        }
        match self.skip(start, Sync::Entry { close }) {
            Stop::Separator => {
                self.advance();
                false
            }
            Stop::Close => {
                self.advance();
                true
            }
            Stop::Outer => true,
        }
    }

    /// Skips the source of what started at token number `start` and failed,
    /// from the current token to where `sync` says it ends, following the
    /// brackets as the lexer does ([`nest`]) from `start` on: at a
    /// separator, or a closing token, with no bracket opened since `start`
    /// still open, or at the end of the input. The token it stops at is not
    /// consumed. A closing bracket of nothing opened since `start` that
    /// does not end it is skipped, as the lexer ignores it, but a `}` of
    /// the file's body, which closes no block, makes the tree
    /// [`Syntax::Unbalanced`].
    fn skip(&mut self, start: usize, sync: Sync) -> Stop {
        let mut open = Nesting::default();
        for index in start..self.pos {
            nest(&mut open, self.token_at(index).kind);
        }
        loop {
            let kind = self.peek().kind;
            if kind == TokenKind::Eof {
                return Stop::Outer;
            }
            if open.open.is_empty() {
                match (sync, kind) {
                    (Sync::Item { .. }, TokenKind::Newline | TokenKind::Semi)
                    | (Sync::Entry { .. }, TokenKind::Comma) => return Stop::Separator,
                    _ => {}
                }
            }
            if nest(&mut open, kind) {
                match (sync, kind) {
                    (Sync::Entry { close }, _) if kind == close => return Stop::Close,
                    (
                        Sync::Entry { .. },
                        TokenKind::RBrace | TokenKind::StrMid(_) | TokenKind::StrEnd(_),
                    )
                    | (Sync::Interpolation, TokenKind::RBrace) => return Stop::Outer,
                    (Sync::Item { top: false }, TokenKind::RBrace)
                    | (Sync::Interpolation, TokenKind::StrMid(_) | TokenKind::StrEnd(_)) => {
                        return Stop::Close;
                    }
                    (Sync::Item { top: true }, TokenKind::RBrace) => {
                        self.builder.set_syntax(Syntax::Unbalanced);
                    }
                    _ => {}
                }
            }
            self.advance();
        }
    }

    /// After the item, or the value of an item, that started at token number
    /// `start` failed, the parser being where it failed: the first newline
    /// token it went on past, as it does after an operator, `=` or `:`, with
    /// no bracket opened since `start` left open, after which an item
    /// starts ([`Parser::starts_item`]): the end of the item, if it does not
    /// go on to the next line, so that the next line, which an editor's
    /// half-typed `x = ` above it leaves alone, is the next item. `None` if
    /// there is none. Costs nothing but on an error.
    fn item_line(&mut self, start: usize) -> Option<usize> {
        let mut open = Nesting::default();
        for index in start..self.pos {
            let kind = self.token_at(index).kind;
            if kind == TokenKind::Newline && open.open.is_empty() && self.starts_item(index + 1) {
                return Some(index);
            }
            nest(&mut open, kind);
        }
        None
    }

    /// Whether the token number `index` starts what is surely an item, not
    /// the rest of an expression: a `let`, a `fn` and its name, a `def`, an
    /// `import`, a `type`, a `defmeta`, a `return`, `break` or `continue`,
    /// an attribute, or a field or a rebind, a path followed by `=`, `:=`
    /// or an annotation and `=`.
    fn starts_item(&mut self, index: usize) -> bool {
        let token = self.token_at(index);
        match token.kind {
            TokenKind::Let
            | TokenKind::Def
            | TokenKind::Import
            | TokenKind::Return
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Attribute(_)
            | TokenKind::InnerAttribute(_) => true,
            TokenKind::Fn => matches!(self.token_at(index + 1).kind, TokenKind::Ident(_)),
            _ => {
                let at = std::mem::replace(&mut self.pos, index);
                let item =
                    self.starts_type_item() || self.starts_defmeta() || self.starts_path_item();
                self.pos = at;
                item
            }
        }
    }

    /// Puts the parser back where `marks` says it was, at token number
    /// `start`, as if what it parsed since had never been: its errors
    /// forgotten too, and what they made of the tree. For parsing it again,
    /// once more than the first time is known of where it ends.
    fn rewind(&mut self, start: usize, marks: Marks) {
        self.restore(marks);
        self.diagnostics.truncate(marks.diagnostics);
        self.builder.reset_syntax(marks.syntax);
        self.unclosed = marks.unclosed;
        self.deep = false;
        self.pos = start;
    }

    /// Skips item separators: newlines and `;`.
    fn skip_separators(&mut self) {
        while matches!(self.peek().kind, TokenKind::Newline | TokenKind::Semi) {
            self.advance();
        }
    }

    /// Skips newlines where an item continues on the next line.
    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::Newline && self.line_end != Some(self.pos) {
            self.advance();
        }
    }

    /// The current token. Past the end of a token slice that lacks a final
    /// `Eof`, this synthesises one at the end of the source.
    fn peek(&self) -> Token {
        self.peek_at(0)
    }

    /// The token `ahead` places after the current one, or a synthesised
    /// `Eof` past the end.
    fn peek_at(&self, ahead: usize) -> Token {
        self.token_at(self.pos + ahead)
    }

    /// The token at `index`, or a synthesised `Eof` past the end.
    fn token_at(&self, index: usize) -> Token {
        self.tokens.get(index).copied().unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.source.len(), self.source.len()),
        })
    }

    /// Moves past the current token. Never moves past `Eof`, which callers
    /// never consume.
    fn advance(&mut self) {
        self.pos += 1;
    }

    fn span_of(&self, id: ExprId) -> Span {
        self.builder.get(id).span
    }

    fn type_span(&self, id: TypeExprId) -> Span {
        self.builder.type_expr(id).span
    }

    /// Describes a token for "found ..." messages: literals and names by
    /// their source text, operators and keywords by their symbol, both in
    /// backticks. The piece of a string after an interpolation is found
    /// where an expression's tokens end, so it is described by the `}` that
    /// ends the interpolation, as [`error_span`] points at.
    fn describe(&self, token: Token) -> String {
        match (token.kind, token.kind.symbol()) {
            (_, Some(symbol)) => format!("`{symbol}`"),
            (TokenKind::Eof, None) => "end of input".to_owned(),
            (TokenKind::Newline, None) => "newline".to_owned(),
            (TokenKind::StrMid(_) | TokenKind::StrEnd(_), None) => "`}`".to_owned(),
            (_, None) => format!("`{}`", self.source.slice(token.span)),
        }
    }

    /// `kind`, the literal of the number token `number`, unless a name is
    /// glued to it, as in `30sec`: the lexer makes a number glued to a unit
    /// a duration or a size, and leaves it a number before any other name,
    /// so that a module path's `pkgs/7zip` is still a path. Where an
    /// expression is expected, that name is "unknown unit `sec`", noted with
    /// the units there are (see [Expressions](#expressions)).
    fn unitless(&mut self, number: Token, kind: ExprKind) -> ParseResult<ExprKind> {
        let next = self.peek_at(1);
        if next.span.start != number.span.end || !matches!(next.kind, TokenKind::Ident(_)) {
            return Ok(kind);
        }
        let name = self.source.slice(next.span);
        let error = Diagnostic::error(format!("unknown unit `{name}`"), next.span)
            .with_note(lexer::UNITS_NOTE);
        Err(self.report(error))
    }

    /// Records an error in the sink and returns the proof that it was
    /// recorded.
    fn error(&mut self, message: impl Into<String>, span: Span) -> Reported {
        self.report(Diagnostic::error(message, span))
    }

    /// Records a diagnostic in the sink and returns the proof that it was
    /// recorded, the tree then being [`Syntax::Recovered`] at least. Not one
    /// at a [`TokenKind::Error`], which is the lexer's to report, nor one
    /// after a lexical error that runs to the end of the input, after which
    /// nothing was lexed; nor past [`MAX_ERRORS`] and one more.
    fn report(&mut self, diagnostic: Diagnostic) -> Reported {
        self.builder.set_syntax(Syntax::Recovered);
        let token = self.peek();
        let lexical = (token.kind == TokenKind::Error && diagnostic.span.start == token.span.start)
            || diagnostic.span.start >= self.quiet_from;
        if !lexical && self.diagnostics.len() <= MAX_ERRORS {
            self.diagnostics.push(diagnostic);
        }
        Reported
    }
}

/// Follows the token of `kind` in the brackets `open`, those opened since a
/// skip began, innermost last, as the lexer follows its own (see
/// [`crate::lexer`]): an opening bracket, or the start of an interpolated
/// string, is pushed; a `)` or `]` closes the bracket of its kind on top of
/// the innermost interpolation's, or nothing; a `}` closes the nearest `{`
/// of the innermost interpolation, with everything opened after it; and the
/// piece of a string after an interpolation closes everything opened in
/// it, its last piece the string too. Returns whether the token closes
/// something opened before the skip began: a closing bracket or piece with
/// nothing of its own open, or a `}` with no `{` open outside an
/// interpolation.
fn nest(nesting: &mut Nesting, kind: TokenKind) -> bool {
    let Nesting { open, strings } = nesting;
    // Where the innermost interpolation's brackets start.
    let context = strings.last().map_or(0, |&at| at + 1);
    match kind {
        TokenKind::LParen => open.push(Open::Paren),
        TokenKind::LBracket => open.push(Open::Square),
        TokenKind::LBrace => open.push(Open::Brace),
        TokenKind::StrStart(_) => {
            strings.push(open.len());
            open.push(Open::Str);
        }
        TokenKind::RParen | TokenKind::RBracket => {
            let bracket = if kind == TokenKind::RParen {
                Open::Paren
            } else {
                Open::Square
            };
            if open.len() > context && open.last() == Some(&bracket) {
                open.pop();
            } else {
                return open.is_empty();
            }
        }
        TokenKind::RBrace => {
            let brace = open
                .get(context..)
                .and_then(|inner| inner.iter().rposition(|&bracket| bracket == Open::Brace));
            match brace {
                Some(at) => open.truncate(context + at),
                None => return context == 0,
            }
        }
        TokenKind::StrMid(_) | TokenKind::StrEnd(_) => {
            let Some(at) = context.checked_sub(1) else {
                return true;
            };
            if matches!(kind, TokenKind::StrMid(_)) {
                open.truncate(context);
            } else {
                open.truncate(at);
                strings.pop();
            }
        }
        _ => {}
    }
    false
}

/// The brackets opened in the source a skip goes over, innermost last
/// ([`nest`]), with where each interpolated string among them is, so that
/// the innermost is found at once.
#[derive(Debug, Default)]
struct Nesting {
    /// The brackets open, innermost last.
    open: Vec<Open>,
    /// The index in `open` of each [`Open::Str`], innermost last.
    strings: Vec<usize>,
}

/// The binary operator a token denotes, if any. `Neg` is not one: a `-`
/// directly followed by its operand is always a negation.
fn binary_op(kind: TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Plus => Some(BinaryOp::Add),
        TokenKind::Minus => Some(BinaryOp::Sub),
        TokenKind::Star => Some(BinaryOp::Mul),
        TokenKind::Slash => Some(BinaryOp::Div),
        TokenKind::SlashSlash => Some(BinaryOp::FloorDiv),
        TokenKind::Percent => Some(BinaryOp::Rem),
        TokenKind::EqEq => Some(BinaryOp::Eq),
        TokenKind::BangEq => Some(BinaryOp::Ne),
        TokenKind::EqEqEq => Some(BinaryOp::Same),
        TokenKind::BangEqEq => Some(BinaryOp::NotSame),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::Le => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::Ge => Some(BinaryOp::Ge),
        TokenKind::And => Some(BinaryOp::And),
        TokenKind::Or => Some(BinaryOp::Or),
        TokenKind::DotDot => Some(BinaryOp::Range),
        TokenKind::DotDotEq => Some(BinaryOp::RangeInclusive),
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::Duration(_)
        | TokenKind::Size(_)
        | TokenKind::Str(_)
        | TokenKind::StrStart(_)
        | TokenKind::StrMid(_)
        | TokenKind::StrEnd(_)
        | TokenKind::Ident(_)
        | TokenKind::Attribute(_)
        | TokenKind::InnerAttribute(_)
        | TokenKind::Let
        | TokenKind::Mut
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Not
        | TokenKind::Fn
        | TokenKind::Def
        | TokenKind::Import
        | TokenKind::Lazy
        | TokenKind::Await
        | TokenKind::For
        | TokenKind::Do
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Return
        | TokenKind::Label(_)
        | TokenKind::In
        | TokenKind::Is
        | TokenKind::As
        | TokenKind::Pipe
        | TokenKind::Neg
        | TokenKind::Colon
        | TokenKind::ColonEq
        | TokenKind::Arrow
        | TokenKind::MemoArrow
        | TokenKind::Eq
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::Semi
        | TokenKind::Dot
        | TokenKind::DotDotDot
        | TokenKind::Comma
        | TokenKind::Newline
        | TokenKind::Error
        | TokenKind::Eof => None,
    }
}

/// `(left, right)` binding powers of a binary operator. Right is left + 1,
/// which makes every level left-associative, except comparison and the
/// ranges, which [`Parser::parse_expr`] makes non-associative. The ranges
/// sit between comparison, whose right power is their left one, and `+`
/// and `-`, whose left power is their right one.
fn binding_power(op: BinaryOp) -> (u8, u8) {
    match op {
        BinaryOp::Or => (1, 2),
        BinaryOp::And => (3, 4),
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Same
        | BinaryOp::NotSame
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge => (5, 6),
        BinaryOp::Range | BinaryOp::RangeInclusive => (6, 7),
        BinaryOp::Add | BinaryOp::Sub => (7, 8),
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::FloorDiv | BinaryOp::Rem => (9, 10),
    }
}

/// `text`, the source of a position or a path, on one line: each line break,
/// with the spaces around it, is one space, so that a path shown in a
/// diagnostic or a dump stays on its line.
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where to point an error about finding `token`: its span, except for the
/// piece of a string after an interpolation, where it is the `}` ending the
/// interpolation, not the text after it.
fn error_span(token: Token) -> Span {
    match token.kind {
        TokenKind::StrMid(_) | TokenKind::StrEnd(_) => {
            Span::new(token.span.start, token.span.start + 1)
        }
        _ => token.span,
    }
}

#[cfg(test)]
mod tests;
