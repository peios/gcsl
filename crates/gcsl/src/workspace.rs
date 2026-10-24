//! A workspace: the files of a directory tree, each directory's files
//! sharing one scope, and every file's dotted paths merged into one view.
//!
//! A *workspace* is the directory tree under a directory that holds a file
//! named `workspace.gcsl`; finding it and reading its files is the CLI's,
//! and this module reads no file: it is given the files as
//! [`SourceFile`]s named by their paths relative to the root, `/`
//! separated, with the root's absolute path, which only the `fileroot`
//! module tells (see [The standard library](#the-standard-library)), and
//! answers with diagnostics or values.
//!
//! # Directory scope
//!
//! The *declarations* of a file are its top-level fields and `def`s, the
//! items slices 23 and 24 made order-free ([`crate::mentions`]). Each
//! directory has a tree of them, the union of its files' field trees
//! ([`Node`]): a leaf is owned by the file that declares it, and an
//! interior node belongs to every file with a leaf under it (its
//! *contributors*), so dotted paths merge across files. A file sees its own
//! private names (`let`, `let mut`, `fn`), the tree of its own directory,
//! and the trees of the directories above it, up to the root; never a
//! sibling's or a subdirectory's, but by an import. A reference means its
//! longest declared prefix in the file's view (below), never in the file's
//! own tree alone: [`crate::mentions::Mentions::resolve`] is given a
//! [`FileScope`], whose numbering keeps the file's own nodes and functions
//! as [`crate::mentions::find`] numbers them and puts every other node
//! after them.
//!
//! One leaf path declared in two files of a directory, and a leaf under a
//! leaf or over an interior node of another file of it, are errors, found
//! while the directory's tree is merged ([`scan`]), unless the earlier is
//! shadowable (see [Shadowable declarations](#shadowable-declarations)):
//! the directory's tree has the winner of each leaf, which every file of
//! the directory sees.
//!
//! A `def` named by a dotted path, `def dev.peios.hello()`, is a leaf like
//! any dotted field's (see [`crate::mentions`], "Dotted `def`s"): it merges
//! under its prefixes with every other file's leaves, in its directory, in
//! every view and through every import, conflicts only as a leaf does, and
//! is shadowable as any `def`. Its merged node's cell is the one its file
//! stores its closure into, so it is read as any cell is, by its path,
//! from another file or through an import, a record of a prefix read whole
//! holding it among the other leaves. It has no name: it is none of a
//! directory's `def`s by name ([`FileScope`]'s `function` never finds it),
//! so another file's reference to it mentions its node, which makes that
//! file need its own, and never its function; within its own file, a
//! reference to it, or to a node above it, mentions its function too
//! (the file's own tree, [`FileScope`]'s `dotted_defs_under`). Once its
//! file is checked, the workspace carries which of its parameters its body
//! measures, as for a `def` of one name (from its function's resolution,
//! since it has no binding: `computed_cells`), so a call of it from another
//! file is of a known callee.
//!
//! # The view of a file
//!
//! Dotted paths always merge. Every file has a *view*: one tree, merged
//! node by node from its *sources*, the tree of its own directory, the
//! tree of each directory above it up to the root, the tree of each
//! `import * from PATH` it has (a directory's merged tree or a file's own
//! tree), and each member import's node at its bound name. An interior
//! node and an interior node of one name merge, their children merged in
//! turn; a leaf (a `def` among them) and anything else of one name is a
//! conflict, except that one cell reached twice is one node (`*` from the
//! file's own directory, from a directory above, from one directory twice,
//! or a file's part of a node also reached whole): merging is idempotent;
//! and except that a leaf shadows what it overlaps, or is shadowed, when
//! the earlier of the two is shadowable (see [Shadowable
//! declarations](#shadowable-declarations)). A whole import (`import PATH
//! [as NAME]`) is not merged: it is a name of its own (see
//! [Imports](#imports)), and one named like a root of the view is still an
//! error, but for a root that is shadowable, which it shadows.
//!
//! Only what several sources have is stored ([`File::view`]): a node one
//! source alone has is that source's node, read as it always was, so a
//! view costs what its overlaps cost, the largest source of each level
//! only looked up in. A node several sources have is a [`ViewNode`] of
//! [`Scan::views`], its parts the sources' nodes, root directory first,
//! the order its children are in (by first definition, each part in its own
//! order); its children several parts have are view nodes too, and a child
//! one part has is that part's child. The file's [`FileScope`] answers a
//! root by the view's node of that name if several sources have it, else
//! the root of the one directory of its chain or the one import that has
//! it; a child of a view node likewise, a child of any other node in that
//! node's own tree. So a leaf of the view is always one cell, of one file,
//! and an interior node of it is either a node of one source, read from its
//! cell as before, or a view node, assembled by the file itself.
//!
//! A view node with a leaf of the file under it is numbered among the
//! file's own field nodes ([`crate::mentions::Mentions::views`]): read
//! whole, it is a node of the file's schedule under slice 24's rule for an
//! interior node, computed at the earlier of "after its last child of the
//! file" and its first mention, the other sources' children computed
//! already, since the file needs their owners. Any other view node is a
//! [`Target::View`], read whole as a record the file assembles as it starts,
//! as an imported record is. Reading a view node whole needs every owner of
//! every leaf under it, in whichever directory or import, but never makes a
//! file need itself; only an import that reaches the file's own nodes by
//! another path (a member renamed, `import com as c from app` inside
//! `app/`) is "`app/main.gcsl` needs itself", as through a whole import.
//! The view is the file's alone: the directory tree committed for the
//! files after it is unchanged, and a file's own record is still built
//! from its own cells only.
//!
//! The scan finds every conflict of every view before any file is
//! checked, once per pair however many views have it, reported at the
//! later of the two sources in the order: the file's own directory first,
//! then each directory upward, then its imports in source order. Two
//! leaves are "field `com.x.lib.name` is defined in two files", "defined
//! here" at both; a leaf and an interior node, "field `com.x` is already
//! defined", the earlier side's whole definition labelled. An import's side
//! is labelled "imported here" at its `*` or member, with an excerpt of the
//! imported definition, "defined here"; a directory above the file's own
//! among the two adds the note "one is in a directory above".
//!
//! # Shadowable declarations
//!
//! A top-level field (a dotted one or a `def` included) or an `import` of
//! any form may be marked `#[shadowable]` (see the parser's
//! [Attributes](crate::parser#attributes)): it may then be shadowed by a
//! later declaration of its name. A LEAF is *shadowable* if its declaration
//! is marked ([`crate::mentions::FieldLeaf::shadowable`],
//! [`Node::shadowable`]), if it reaches a view through a marked import
//! ([`File::marked`], [`ViewPart::marked`]), or if it was shadowable where
//! it was declared: the mark travels with the leaf through directory merges
//! and imports. A node is shadowable when all its leaves are.
//!
//! Every declaration and import reaching a file's view has a place in one
//! STATIC order, the same for every reader: the implicit import of the
//! prelude (below) first of all; then the directories from the root down to
//! the file's own; within one directory, its files in path order, each top
//! to bottom; and a file's imports at their lines in it, among its own
//! declarations, though they take part in its view only (`Rank`). A later
//! declaration shadows every earlier one it OVERLAPS (at its path, at a
//! prefix of it, or under it) if every one of those is shadowable, and
//! conflicts as before if one is not; two interior nodes do not overlap at
//! their own name, and merge. So a plain leaf is shadowed by nothing, of
//! shadowable ones the last stands, and a declaration whose path is above
//! or below a shadowable one replaces it whole: `#[shadowable] d = {j = 4;
//! k = 5}` then `d.j = 8` is `d = {j = 8}`, a record written whole being no
//! tree (defaults meant to be overridden field by field are written dotted,
//! `#[shadowable] d.j = 4`); `#[shadowable] a.x = 1` then `a.y = 2` do not
//! overlap, and merge. Of one cell reached both through a marked import and
//! plainly (`import * from prelude` written, beside the implicit one), the
//! plain one stands, and conflicts as any import does.
//!
//! SHADOWING IS PER VIEW. Within one directory the files' declarations are
//! swept together in the order before its tree is merged (`shadow`, with
//! [`crate::mentions::Sweep`]; within one file, [`crate::mentions::scan`]):
//! a declaration beaten there, in its own directory's tree, is in no tree,
//! a leaf of nothing: its file's mentions are found without it, nothing
//! reads it, it is never run, and it takes no part in cycles or needs, but
//! it is still checked as written (see the checker's "Shadowed
//! declarations"), its reads of cells it alone would need seeing them as
//! erroneous. So within one file the last declaration is the leaf for the
//! whole file, a read above it included, and a shadowing declaration
//! cannot read what it shadows (`#[shadowable] a = 1` then `a = a + 1` is
//! "`a` depends on itself"). A declaration beaten only in SOME views, by
//! an inner directory's declaration or by a file's own later import, which
//! is private, stays its directory's leaf and runs for the views that
//! still see it: only the view where it is beaten has the winner, the
//! view's node of its name being the winner's alone ([`Merged::One`],
//! `shadow_parts`), and a file's own record still has its own cells. A
//! member or `*` import gone from a view is no alias of its file
//! ([`File::aliases`]). An interior node of the file's own directory with
//! leaves both before and after an import of the file that overlaps it
//! would stand only in part, which a view cannot hold: that is the conflict,
//! conservatively.
//!
//! The clash family ignores what is shadowable: a member or `*` import
//! that is clashes with nothing, a whole import named like a shadowable
//! root takes its name in the view silently, and in the file that declares
//! it a top-level `let` (from its item on) or `fn` (in the whole file) of
//! the name of a shadowable root is read in its place, as it was of the
//! prelude's names (see [`FileScope::yields`] and the checker's
//! "Imports").
//!
//! # Order
//!
//! File `A` *needs* file `B` when an item of `A` (an ordered item, a
//! leaf's initialiser, a function's body) mentions a declaration `B` owns:
//! a leaf or a `def` of `B`, or an interior node one of whose leaves is
//! `B`'s. Files are checked and run in an order where each comes after the
//! files it needs, path order breaking ties ([`Scan::order`]); files that
//! need each other are an error, one per cycle. A file runs as a whole
//! before a file that needs it.
//!
//! # Cells and slots
//!
//! Every node of every directory tree is one cell of the workspace's one
//! frame, and its slot is its index in [`Scan::nodes`]: slots `0..` the
//! number of nodes are the workspace's cells, the built-in functions' the
//! first and the standard files' the next (see [The standard
//! library](#the-standard-library)), and every other slot (a
//! file's private names, its temporaries) is allocated from there up by
//! the checker, as it allocates from the first free slot of a session's
//! scope. A file's own field node shares the slot of the merged node of
//! its path ([`File::global`]). An interior node with leaves in several
//! files is computed by the last of them in the order, as a node of that
//! file's schedule ([`File::schedule`]); every other contributor treats it
//! as computed elsewhere. Its point is the earlier of "after its last child
//! in that file" and the first mention of it there, so a whole read of it
//! never finds its slot unwritten. A file's own value, when it has no
//! result, is the record of its own cells only, with a nested record for
//! each prefix it shares with other files, never the shared node: so
//! `app.gcsl`'s value has `com.x.app` and not `com.x.lib`.
//!
//! # A workspace as a session
//!
//! [`check`] scans the workspace and then checks and lowers every file in
//! the order, as the inputs of a [`crate::Session`]: one table of names, of
//! types and of slots, each file's diagnostics named by its input number,
//! its index among the files, the standard files it loads first and the
//! workspace's after them in path order ([`Diagnostic::input`]). After a
//! file is checked, its declarations are committed to its directory's scope
//! for the files after it; its private names (`let`, `let mut`, `fn`) are
//! not, though their slots stay taken. The scope is committed after the check,
//! not after a run, since every file is checked before any runs: any error
//! anywhere stops everything. A file that needs a file whose check failed
//! is not checked, since its errors would follow from the first.
//!
//! [`Workspace::run`] then runs the file asked for, after the files it
//! needs, transitively ([`Scan::needed`]), each as a whole and in the
//! order, on one frame and with one memo table, as session inputs share
//! theirs: a closure or a future of one file used in another reads its
//! file's cells, and a future awaited from several files runs once. Files
//! nothing needs are checked but never run. The effects of a file all
//! happen before those of a file that needs it. A runtime error names the
//! file whose code failed.
//!
//! # Syntax errors
//!
//! A file with lexical or syntax errors is scanned and checked as its
//! parser recovered it (see the parser's [Errors and
//! recovery](crate::parser#errors-and-recovery)), so that one file's typo
//! hides nothing of the others', but nothing runs: the check fails, with
//! every file's syntax errors ([`Scan::syntax_errors`]), and with every
//! other error found that is no cascade of them. Each error is reported
//! once, where it is; what may follow from source that could not be parsed
//! is not:
//!
//! - The file with the errors is checked, its error nodes of type `Error`
//!   and a block with an error item erroneous, silently, and reports its
//!   errors, never its warnings, which may be of what could not be parsed.
//!   It fails, whatever its check finds, and is never lowered, and every
//!   file that needs it is not checked, as a file that needs one whose
//!   check failed is not. A file whose tree is
//!   [`crate::ast::Syntax::Unbalanced`], its braces not matching, is not
//!   checked at all: some of its items may be in the wrong block.
//! - A name that names nothing (an undefined variable, an unknown type, a
//!   record's field it lacks), in a file whose view sees a file with
//!   syntax errors ([`Scan::sees_broken`]: it has them, a file of its
//!   directory or of one above has, or it imports something of one), is no
//!   error, since it may be declared in source that could not be parsed:
//!   the file's check fails quietly instead.
//! - The errors of the scan itself (see [Scan errors](#scan-errors)) are
//!   reported with the syntax errors, but those of a file that sees a file
//!   with syntax errors, as above, or one of whose imports that failed
//!   names a module with one, whose declarations may be missing or
//!   misplaced: the scan recovers from those all the same.
//!
//! Files that parse cleanly and see no file with errors are checked as
//! ever. A session's input with an error is not checked: nothing of it is
//! run or kept. [`check_partial`] leaves what a failed check found
//! ([`Partial`]), and [`index_with_standard`] (and, for a lone file,
//! [`index_lone_with_standard`]) checks as [`check_with_standard`] does,
//! each file's check recording what its names refer to (see
//! [`crate::check`], "Recording"), and indexes what every checked file's
//! check found, failed or not, for a language server ([`crate::index`]); a
//! file that is not checked, unbalanced or needing a file whose check
//! failed, has nothing indexed.
//!
//! # Scan errors
//!
//! The scan's own errors (two files of a directory defining one field, an
//! import of no module, a conflict of a view, a cycle of files, ...) do not
//! stop it either: each stage reports every error it finds, once, and
//! recovers from it, so that the stages after it, and the check, see a
//! workspace they can work with, and every file the errors do not concern is
//! checked as ever ([`Scan::scan_errors`]). Nothing runs: the check fails
//! with every syntax error, every error of the scan and every error the
//! check finds, in path then source order. Only a session's input with an
//! error is not checked at all, as for a syntax error. What may follow from
//! what the scan dropped or poisoned is not reported: a check that reads it
//! finds `Error`, silently, and fails quietly, so that a file that needs it
//! is not checked, as a file that needs one whose check failed is not.
//!
//! - A CONFLICTING FIELD, a leaf of two files of a directory, or a leaf and
//!   an interior node of two (`merge`): the first in path then source order
//!   stands, and every later one, its file's leaves at or under the node it
//!   conflicts at, is dropped ([`File::dropped`]), as a shadowed declaration
//!   is (see [Shadowable declarations](#shadowable-declarations)): in no
//!   tree, needing nothing, checked as written, its own errors reported.
//!   The node the dropped declaration meets the tree at is CONFLICTED
//!   ([`Scan::conflicting`]), and it and each node above it POISONED
//!   ([`Scan::is_poisoned`]): its cell is read as `Error` through an import
//!   or a path, and a root over one is committed as `Error`, so that every
//!   read of it, in any file, is erroneous, and nothing is reported of it
//!   but the conflict. An index finds the conflicted node declared at each
//!   of its definitions, the dropped ones included ([`crate::index`]).
//! - A conflict of a file's view, between directories of its chain or with
//!   an import, a conflict of its view of types, and an import that clashes
//!   with a name of the file or its directory chain: the root of the path
//!   in conflict, or the name, is POISONED IN THE FILE ([`File::poisoned`]):
//!   a read of it there that no binding of a function or an inner block
//!   shadows, as a value or as a type, bare or qualified, is `Error`,
//!   silently. The view keeps what it would otherwise have; the conflicting
//!   types are left out of the file's view of types.
//! - An import of no module, or of a member a module lacks, and an import
//!   from `fileroot` of its members: each name it would bind is poisoned in
//!   the file. A `*` import that fails binds names the scan cannot know: in
//!   a file with one, a name that names nothing is no error, as in a file
//!   that sees a file with syntax errors ([`File::blind`]).
//! - Two `type` declarations of one name in a directory: the first is the
//!   directory's, and both are erroneous, a use of either failing quietly. A
//!   cycle of types makes each of them erroneous, as it always did; the
//!   files' trees are expanded regardless. An import of a type under a
//!   built-in type's name binds nothing.
//! - A meta declared twice in a directory, or with no definition there: it
//!   is dropped, as a conflicting field is, and is not checked.
//! - Files that need each other, or a file that needs itself: each file of
//!   the cycle is not checked ([`Scan::is_cyclic`]), its needs of the other
//!   files of the cycle are forgotten so that the order has every file, and
//!   a file that needs one of them is not checked, as for a failed file; an
//!   index checks them all for their index alone, as it does a file that
//!   needs a file whose check failed.
//! - A file or directory of the root named like a module of the standard
//!   library: reported, and nothing else; an import of the name names the
//!   module, as it always did.
//!
//! Recovery costs nothing on a workspace with no error: the trees are merged
//! again only when a conflict was found.
//!
//! # Imports
//!
//! An `import` gives a file the rest of the workspace, by name. Its path is
//! absolute from the root: each segment but the last a directory, the last
//! a directory or a file's stem (`import lib/x` is `lib/x.gcsl`); the root
//! directory itself has no path. The scan resolves every import of every
//! file, mentioned or not ([`scan_with`]): a path that names nothing is "no
//! module `a/b`", with a note listing what the last directory that exists
//! has (its subdirectories, with `/`, then its file stems), or saying that
//! the directory is a workspace of its own. The library cannot see that
//! from the files alone, so [`scan_with`] and [`check_with`] take the
//! root-relative directories the caller left out as nested workspaces;
//! [`scan`] and [`check`] pass none. A directory and a file of one name are
//! an error once per pair, at the first import that names them.
//!
//! A segment is a directory's or a file's name as it is, dots and all, so
//! `pkgs/net.busybox.busybox` names the directory `net.busybox.busybox`
//! (see the parser's [Imports](crate::parser#imports)). A path whose
//! segments have dots in them, in the old form of a path, `lib.x`, that
//! names nothing but would name a module with each `.` a `/`, is "no module
//! `lib.x`" with a note saying so instead of what the directory has, and the
//! path with slashes, `lib/x`, as its suggestion.
//!
//! An imported name is an [`Alias`] of the file ([`File::aliases`]), naming
//! a [`Target`]: a directory's merged tree without its subdirectories, a
//! file's own tree (or, for a value file, its value), or a root of either
//! (`import x from lib`, and each root of `import * from lib`). A whole
//! import (`import lib`, `import lib/x as f`) is a private top-level name
//! of its file: in scope throughout it, shadowed by nearer names, never in
//! its tree or record, never seen by another file; one clashing with
//! another top-level name of the file, with a member or `*` root of it, or
//! with a root of its directory chain, is an error found by the scan. A
//! member and a root of a `*` merge into the file's view instead ([The
//! view of a file](#the-view-of-a-file)), private to the file too, and
//! clash only with a whole import or a top-level `fn`, `let`, `let mut` or
//! `:=` of their name. `import * from` a value file is "a value file has
//! no members to import". The aliases are a lookup of the [`FileScope`]
//! apart from the merged trees ([`FileScope::alias`]), so a reference
//! through one means its longest declared prefix in the target's tree, as
//! every reference does: the numbers past the merged nodes are targets
//! ([`FileScope::target`]).
//!
//! The needs through an import are per leaf ([`Scan::target_files`]): a
//! leaf, a `def` or an interior node needs the files that own it, a file
//! read whole needs that file, a directory read whole needs every file of
//! it, and a view node read whole every owner of every leaf under it. A
//! reach through an import into a node the file itself owns or contributes
//! to, its own file or directory read whole through a whole import, and a
//! whole read of a directory one of whose files needs this one, are the
//! one-file form of the cycle error, "`app.gcsl` needs itself"; files
//! importing each other are the cycle error of [Order](#order).
//!
//! A reference that ends on a cell reads that cell's slot: a `def`'s is its
//! function's slot, since the merged node of a `def` is never written. A
//! record read whole (a directory, a file, a file's part of a shared node,
//! or a view node without a leaf of the file under it) is assembled by the
//! reading file, once, into a private slot of its own, before its first
//! item ([`crate::check#imports`]). A value file that some file imports has
//! a cell of its own after the merged nodes ([`Scan::value_slot`]), which
//! the file stores its result into as the last node of its top-level
//! block.
//!
//! # Types
//!
//! A `type` item declares a name in its file's namespace of types, apart
//! from its fields and variables (see the checker's "Type declarations").
//! In their own namespace types follow the rules fields follow for
//! directories, views, imports and conflicts, but they are no part of the
//! field trees, whose nodes are cells: a type has no cell, no slot and no
//! IR, needs nothing and is needed by nothing, so a file reading another's
//! types does not need it and types play no part in the order. So the scan
//! keeps them apart, flat, a type's name being one identifier, with the
//! field machinery's order of sources, conflict texts and labels:
//!
//! - Every file's declarations ([`Scan::types`], [`File::types`], found by
//!   [`crate::mentions`]), but one named like a built-in type, and a second
//!   of one name in one file, which the checker reports. A directory's
//!   types are its files' ([`declare_types`]): one name declared in two
//!   files of it is "type `Foo` is declared in two files", labelled
//!   "declared here" at both, as two leaves of one path are.
//! - A member import of a module's type binds it (`import PackageDef from
//!   lib.types`), and of a name that is both a type and a root binds both;
//!   `*` binds every type of the module, besides its roots ([`members`],
//!   [`File::type_imports`]). A directory's types are its files', a file's
//!   its own. `*` from a value file is still an error, but its types may be
//!   imported by member or read qualified.
//! - Each file's view of types ([`File::type_view`], [`type_views`]) merges
//!   its own directory's types, each directory's upward and its imported
//!   types, in that order, the order fields' views report their conflicts
//!   in; and, for a session's input, those it inherits from the earlier
//!   inputs, last, which anything shadows. One declaration reached twice is
//!   one type. Two of one name are "type `Foo` is declared in two files",
//!   once per pair, at the later source, labelled "declared here" at each
//!   declaration and "imported here" at each import, with the note "one is
//!   in a directory above" when one is. Types are never shadowable: a
//!   `#[shadowable]` import brings them as plain types, and the prelude
//!   declares none. A type imported as a built-in type's name is "`Int` is a
//!   built-in type".
//! - A type marked `#[attribute]` is an attribute's declaration (see the
//!   checker's [Attributes](crate::check#attributes)): the store records it
//!   by its number, with what a host knows it by, its file's path without
//!   `.gcsl` and its name ([`TypeStore::mark_attribute`]), so that a session's
//!   later inputs know it too, but one named like a built-in attribute,
//!   "`retry` is a built-in attribute". A use of an attribute is a name of a
//!   type of the file's view, read as any is, and its file's checker asks
//!   whether that type is one (`Context::attribute`).
//! - A qualified name, `t.PackageDef` ([`Scan::qualified_type`]), names
//!   through a whole import of the file (`import lib/types as t`) a type of
//!   the module it names: "`t` has no type `Foo`", with a note listing its
//!   types, if it has none of that name; "unknown type `a.b`" if `a` is no
//!   module the file imports whole; and "`t.Foo` is a type, not a module"
//!   for a path that goes on after a type. A module's name written as a
//!   type, `t`, is "`t` is a module, not a type" ([`Scan::type_links`]).
//! - A generic type, `type Pair<A, B> = ...`, is used applied to as many
//!   type arguments as it has type parameters, `Pair<Int, String>` or
//!   `t.Pair<Int, String>`, which the store records of each declaration
//!   ([`TypeStore::set_arity`]); used bare it is "type `Pair` needs its type
//!   arguments", with too few or too many "type `Pair` takes 2 type
//!   arguments, given 1", and a type that is not generic, applied, "type
//!   `Foo` takes no type argument" ([`Scan::applied_link`]). An attribute's
//!   type is never generic: "attribute `Tag` takes no type parameters".
//!
//! Then the types are defined and the files' trees expanded
//! ([`define_types`]): each declaration's definition is copied into the
//! scan's [`TypeStore`], each name in it of another declared type a
//! reference to it, as the declaring file's view resolves it, an instance
//! of it if applied to arguments, and each of a generic declaration's own
//! type parameters a parameter; the references are a graph, whose cycles are each "type `A` refers to
//! itself", at the first declaration of the cycle in path then source
//! order, labelled at each step ("`B` uses `A` here", in `B`'s file);
//! every declaration is settled, those it refers to first, a reference
//! made what it refers to, an instance what it refers to with its
//! arguments in place of its parameters, and a spread merged, one on a cycle, with a
//! spread's problem or nested deeper than [`crate::MAX_DEPTH`] once
//! expanded erroneous; and each file's tree is expanded
//! ([`Ast::expand_types`]): each name of a declared type in it made a copy
//! of the definition, as if written there, a generic one's with the use's
//! arguments in place of its parameters, each record type with spreads
//! the record of its merged fields. A session keeps its store, each input's
//! declarations after the earlier inputs' ([`check_input`]). The checker
//! then sees no declared type's name; before any file is checked, the
//! driver checks every file's `type` items ([`check_type_items`]), since a
//! file may use a type of a file checked after it, so that what is wrong in
//! a definition is reported once, where it is written, and every file using
//! it fails quietly (see the checker's "Type declarations").
//!
//! # Def-types
//!
//! A def-type, a `def` marked `#[def_type]`, has a *namespace*: the
//! definitions made with it, `def package dev.peios.testpkg() { ... }` (see
//! the checker's "Def-types"). The namespace is keyed by the def-type's
//! identity, the `def` its `DT` names, whatever import or alias names it: a
//! name no source can write, its *key*, `<package#F.I>`, of the def-type's
//! path as it is declared, its file's number (a session's input's number
//! for one an input declares) and its item's (see [`crate::names`],
//! "Namespace keys"). A definition is a leaf of the tree of top-level
//! fields at the path of the key and its own path, so in their namespace
//! definitions merge, conflict and are shadowable exactly as dotted `def`s
//! do among fields, in the files of a directory, the directories below and
//! every view, through every `*` import of a file or a directory, and never
//! meet a field: the key is a root of its own. Conflicts are reported as
//! any leaf's, the path shown as the definition writes it ("field `package
//! x` is defined in two files"). A whole import's record, a directory's or a
//! file's, holds no namespace (the checker's `imported_record` passes the
//! keys by), so definitions are reached by their def-type alone.
//!
//! What a `DT` names is known only once the views are, which are made of
//! the trees the definitions are leaves of. So the scan makes the trees,
//! resolves the imports and makes the views (`scan_trees`), and then finds
//! the key of each definition and reference whose `DT` is not known yet
//! (`name_namespaces`): the leaf the file's view has at its path, if it is a
//! `def` marked `#[def_type]` (`def_type_key`, through [`FileScope`]'s
//! leaf's owner), and for a session's input whose view has nothing of its
//! first name, the def-type of that path an earlier input declared
//! ([`Earlier`]). Each is given its key ([`Ast::set_def_type_key`]), which
//! its file is then taken to spell. If a definition got one, the trees,
//! views and types are forgotten (`unscan`) and made again, once, with the
//! definitions under their keys; a scan with no definition makes them once.
//! A reference's key changes no tree, and its read of the definition is
//! resolved with the other references ([`resolve`]): it needs the
//! definition's file, and its `DT` the def-type's, and a definition's `DT`
//! makes its file need the def-type's.
//!
//! A definition's meta, `defmeta package dev.peios.testpkg { ... }` (see the
//! checker's "Metas"), belongs to the definition of its path made with its
//! def-type in its own directory, in any file of it. So its namespace is the
//! def-type's and the directory's: the meta key of the def-type's key and
//! the directory's number (see [`crate::names`], "Namespace keys"), under
//! which the meta is one leaf, its path one name, so that no two metas of a
//! directory lie one under another and no directory's metas meet another's,
//! in a view or through an import. `name_namespaces` gives each definition
//! and each meta, with its key, its leaf there, and a definition its read of
//! that leaf, which is resolved as a reference's read is: so a definition
//! needs its meta, and a definition's file the file of its meta, which runs
//! before it; two files of a directory each holding a definition whose meta
//! the other holds are files that need each other, the usual error. Before
//! the trees are made again with the metas in them, a directory's metas are
//! paired with its definitions (`unpaired_metas`): a meta whose directory
//! declares another of its def-type and path is "`defmeta package x` is
//! declared twice in this directory", at the later, labelled "first declared
//! here" at the first; one whose directory declares no definition of its
//! def-type and path (one, or several of a `#[shadowable]` chain), "`defmeta
//! package x` has no definition `x` in its directory"; each from its
//! `defmeta` through its path, errors of the scan, which stop it. Then each
//! file records its definitions whose directory declares no meta of their
//! def-type and path ([`File::unpaired`]), which the checker reports if
//! their def-type declares a meta, since only it knows the def-type's type.
//! A definition shadowed in a directory below, which brings its own meta
//! there, is no concern of its own directory's metas.
//!
//! # Def-types by name
//!
//! A program that turns def-types into commands (`pekit package
//! dev.peios.testpkg`) asks a checked workspace for its def-types and
//! invokes one of their definitions by name, as text; nothing here is
//! written in any source, and nothing of it changes what a file, a run or a
//! dump does. [`Workspace::def_types`] lists every `def` marked
//! `#[def_type]` of the workspace's own files (not the standard files'),
//! each by its path as declared ([`DefType`]), with the names of its
//! invocation parameters, as it declares them, and which of them are
//! optional, having a default (`version: String = ""`), and every definition made
//! with it ([`Definition`]): found by a search of every file, whichever
//! views see it, so a definition in a directory the root does not see is
//! found, and one a later one shadows too; grouped by the def-type its `DT`
//! names, its namespace's key (see [Def-types](#def-types)), whatever
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
//! [`Workspace::invoke`] calls the definition as `DT PATH(NAME = VALUE, ...)`
//! would be called from a file of the definition's directory, with its
//! arguments BY NAME, each the name of one of the def-type's invocation
//! parameters and a value ([`Given`]), in any order; each invocation
//! parameter given none takes its default (see the checker's "Calls"), if it
//! has one, whichever others are given: so `def lock(name: String, func:
//! fn(version: String) -> List<Version>, version: String = "")` invoked with
//! none gives `version` `""`, and `def build(name: String, f: fn(): String,
//! version: String = "", target: String)` may be given `target` alone. A
//! value is of one of six types, `String`, `Int`, `Float`, `Bool`,
//! `Duration` and `Size`, and
//! is given only for a parameter of that type as it is declared, an alias of
//! it, or the either shape of an impure def-type's bare one (`version:
//! String` of an impure def-type is `String | Future<String>`, of a memoised
//! one `String`); a parameter of any other type, a union, a `Future`, a
//! record or a list, cannot be given one, and so a def-type with such a
//! parameter that has no default cannot be invoked so, while one with a
//! default is left out and takes it. [`Workspace::arguments`] reads them from
//! text, as a command line gives them, each by its parameter's type: a
//! `String` the text as it is, read as nothing else (no `{` in it
//! interpolates), an `Int` a decimal integer, with `-` before it if
//! negative, in range, a `Float` a decimal number Rust's `f64` reads, finite,
//! a `Bool` `true` or `false`, a `Duration` or a `Size` one of its literals,
//! `30s` or `2GiB`, with `-` before it if negative; it runs nothing, so a host can check every
//! request before it runs any. What is asked for may name nothing: no
//! def-type of the name, no definition of the path, or a path above
//! definitions; or the arguments may not fit the def-type: an invocation
//! parameter that has no default and cannot be given a value, "`build`'s
//! parameter `n` is `List<Int>`, but only `String`, `Int`, `Float`, `Bool`,
//! `Duration` and `Size` parameters can be given here yet", the first such, its type as it
//! is written, whatever is given; then, argument by argument, a name that
//! is no invocation parameter's, "`package` has no invocation parameter
//! `jbos`", noting those it has, one named twice, "`package`'s parameter
//! `version` is given more than once", one given for a parameter that
//! cannot be given a value (the same error as above), or a text that is not
//! a value of its parameter's type, "`x` is not an `Int`, the type of `jobs`
//! of `package org.foo`", noting how one is written; and last the
//! parameters that have no default and are given none, "`deploy` needs
//! `target` and `mode`, which were not given". Each is a [`RequestError`],
//! an error of no source, in that order. Then the
//! definition's file and the def-type's run as a reference to the
//! definition needs them to, each as a whole, with the files they need (its
//! meta's among them, if it has one), unless they have run, as for
//! [`Workspace::run`], and the call is made: an input of its own, built
//! rather than parsed, `<def-type>("PATH", <definition>, NAME = VALUE, ...)`
//! as its result, each value a literal of its kind (a string literal of the
//! text given, an integer, a float, or `true` or `false`), named by its
//! parameter, whose two names no source can
//! write are bound, in a scope of the workspace's slots, as fields of the
//! def-type's cell and the definition's (the slots of their closures), with
//! their types, and the def-type's with what a call of it needs to know of
//! its parameters, as a session's input knows a function of an earlier one,
//! so that the call is of a known callee and gives each optional parameter
//! given none its default. It is checked as an input of no workspace, so that it looks
//! nothing up in any file's view and needs no alias, placed in the
//! definition's directory, so that the def-type's caller (`fileroot.caller`)
//! is that directory, as for a call written there, and the definition's is
//! what the def-type makes it; lowered with an input number after every
//! file's, and run on the workspace's frame and memo tables. It takes no
//! cell and commits nothing, so it changes nothing any file sees, and
//! invoked again it is made and run again, a call like any other, the files
//! not running again. Every diagnostic it returns names a file of the
//! workspace or a standard file: the call has no source, so a diagnostic or
//! a label located in it ("call nested too deeply" at the call, for a
//! def-type's body nested within a few levels of the limit) is located at
//! the definition's path in its file instead, labelled "invoked here".
//!
//! A value a host gets back may hold a reference, `build = build busybox`,
//! which it can call only by name. [`Workspace::definition_of`] names it:
//! the closure of a reference is of a function the program records as made
//! for one (see [`crate::lower`]), which captures the def-type and
//! then the definition, and the definition is found as the definition whose
//! cell holds the very closure captured, its def-type by its path as
//! declared, however the reference names it. Any other value names none,
//! whatever it captures. It runs nothing.
//!
//! # Top-level functions by name
//!
//! A host may give a file of the workspace a meaning of its own (`pekit`
//! its root's `pekit.gcsl`, whose `environment` it tears down with its
//! `teardown`), which nothing here knows of: it asks for the file's
//! top-level fields and `def`s by name, the fields of the file's own record
//! that another file reads as members of its module. A private name (`let`,
//! `fn`) is none, and nor is a dotted path under the name, or a declaration
//! a later one shadows in its own file. [`Workspace::top_level`] tells one's
//! type, as it is written (a `def`'s as `fn(env: Vm) -> {}`), and, for a
//! function, what it may do and how many parameters it takes;
//! [`Workspace::memoised`] whether a memoised (`->>`) function has returned
//! for no arguments in the workspace's memo tables, which every run,
//! invocation and call shares for the workspace's life, as a call of it
//! written in the file's directory would find; [`Workspace::call_composed`]
//! calls a composition of them, `teardown(environment())` for `["teardown",
//! "environment"]`, as if written in a file of the file's directory, after
//! the file and those it needs have run, as the call that invokes a
//! definition is made, built rather than parsed, the functions bound by names
//! no source can write, checked, lowered and run on the workspace's frame
//! and memo tables, so a memoised function called before is not run again;
//! and [`Workspace::check_composed`] checks that call without running
//! anything, which says whether each function takes what the next returns,
//! as the checker says it of a call. Every diagnostic of the call located in
//! it is located at the first function's name instead, labelled "invoked
//! here".
//!
//! # The standard library
//!
//! Two kinds of module are not files of the workspace. `builtins` and
//! `fileroot` are modules implemented in Rust: `builtins` a record of the
//! built-in functions ([`Builtin`]): `len`, `int`, `float`, `exec`,
//! `panic`, `split`, `join`, `starts_with`, `ends_with`, `trim`,
//! `strip_prefix`, `strip_suffix`, `lines`, `fields`, `compare_versions`,
//! the file system's `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`,
//! `fs_temp_dir` and `fs_write`, and `sort_versions` and `show`;
//! `fileroot` where files are (below). The *standard files* are the
//! `.gcsl` files of the repository's `stdlib/` directory, compiled in
//! ([`crate::STANDARD`]): `fs.gcsl`, the file system, whose members are
//! the file system's built-in functions, bound as the prelude binds the
//! others (`exists = builtins.fs_exists`, `write = builtins.fs_write`), so
//! that a call of one is a call of the built-in, its errors located at the
//! call; `path.gcsl`, lexical operations on paths; and `prelude.gcsl`,
//! which binds the other built-in functions, `show` and `len` by the alias
//! rule, and `sh`. Each is
//! importable by a bare name, `import builtins`, `import len from
//! builtins`, `import fileroot`, `import * from prelude`, `import path`,
//! `import fs`, from any file; the names are reserved roots of the path
//! namespace, and a file or directory at the workspace's root named like
//! one of them is an error of the scan, "`builtins` is the name of a
//! built-in module" (and so for `fileroot`), "`prelude` is the name of a
//! standard module" (and so for `path` and `fs`), reported at the start of
//! the file, or of the first file under the directory, whether or not any
//! file imports it. A path of more
//! than one segment never names one, and a file or directory below the
//! root has a path of its own.
//!
//! EACH STANDARD FILE IS A DIRECTORY SCOPE OF ITS OWN, a directory with no
//! directory above, whose path is [`STANDARD_DIR`]: it sees its own
//! declarations, the prelude's through its implicit `#[shadowable] import *
//! from prelude` (below, as for any file), and what it imports, by
//! bare name, `builtins` and the other standard files; nothing else of the
//! standard library and nothing of the workspace, and nothing of the
//! workspace sees its declarations but by an import. So two standard files
//! may declare one top-level name, as the prelude's string `join` and
//! `path`'s `join` do. One standard file importing another is an import
//! like any other, and standard files that need each other are the usual
//! error, which is a bug of the library.
//!
//! ONLY THE STANDARD FILES A RUN NEEDS ARE LOADED: checked, lowered and
//! run, given cells, slots and input numbers. A scan of a workspace or a
//! lone file loads the prelude, always, and each standard file that any
//! file it scans imports, whether or not the file run needs that file,
//! since every file of a workspace is checked: an `import` whose path is
//! one segment naming a standard file, in whatever form (`import path`,
//! `import path as p`, `import join from path`, `import * from path`);
//! and, transitively, each standard file that a standard file it loads
//! imports so. The scan computes the set as it parses (`parse_files`): it
//! parses the prelude, and what the prelude imports, then the workspace's
//! files, then reads the top-level `import` items of each file that parsed
//! and of each standard file as it is parsed in turn (`standard_imports`),
//! and parses each standard file they name that it has not, until none is
//! left; a file that fails to parse imports nothing. A standard file
//! nobody imports is never parsed. A session loads every standard file
//! instead (see [`crate::session`]).
//!
//! The standard files a scan loads are its first files, so they take the
//! first input numbers, in the order of [`crate::STANDARD`], which is path
//! order, each named `<stdlib>/STEM.gcsl`, the workspace's files following
//! them: so `path`, when it is loaded without `fs`, is input 0 and the
//! prelude input 1, `fs` with both is input 0 and they 1 and 2, and a run
//! that loads the prelude alone numbers its files as if there were no other
//! standard file. Since the caller cannot know which are
//! loaded before the scan has parsed the files, [`check_with_standard`] and
//! [`check_lone_with_standard`] return their sources with the check,
//! whether it succeeds or fails, for rendering diagnostics by input number;
//! [`standard_sources`] gives every one, as a session loads them. They are
//! checked and run with the workspace's files, before them in the order
//! (they need no file of the workspace), on the same frame and memo
//! tables; every run runs those loaded first ([`Scan::needed`]). An error
//! in one is reported like any other, and is a bug.
//!
//! The `builtins` module is a directory of its own too, with no file: its
//! tree is one root leaf per built-in function, nodes `0..` of
//! [`Scan::nodes`] in the order of
//! [`Builtin::ALL`], so their cells are the first slots of the frame, which
//! the driver writes, each a [`Value::Builtin`], as it lays the frame out,
//! before any file runs ([`check_with`]). No file computes them, and none
//! needs another for them. Their types are each built-in's: its function
//! type, or for one whose typing is special the opaque type of the alias
//! rule (see [`crate::check`], "Built-in functions"). As it lays the frame
//! out, the driver also builds the one layout of the records `exec` and
//! `exec_live` return (`exec_layout`), its field names interned in the
//! run's names, and gives it to every file it runs, beside the memo tables,
//! so that
//! every such record in the run shares it (see [`crate::eval`], "The
//! built-in functions"); a session gets it with the rest of the standard
//! library's run.
//!
//! A [`Workspace`] OWNS THE TEMPORARY DIRECTORIES that `fs.temp_dir` makes
//! in its runs, the files' and every [`Workspace::invoke`]'s alike (a
//! [`crate::eval::TempDirs`], given to every file it runs as the layout of
//! `exec`'s records is): one parent directory in the system's temporary
//! directory, made by the first call and never before, so that a run that
//! makes none touches nothing, each directory a numbered child of it. As
//! the workspace is dropped, the parent is removed, with everything in it,
//! whether its runs succeeded or failed: so when `gcsl run`, of a workspace
//! or of a lone file, whose run is a workspace's too, or `pekit` is done
//! with it. A process killed before then leaves the directory behind. Its
//! host may choose another directory than the system's temporary directory
//! for the parent, its TEMP ROOT ([`Workspace::set_temp_root`]), before it
//! runs anything: the root is made by the first call if it is missing, and
//! is never removed. A session owns those of its inputs, and of the
//! standard library's run as it starts, as a workspace does (see
//! [`crate::session`]), always in the system's temporary directory.
//!
//! The `fileroot` module has no file, no cell and no value: it is a target
//! of its own ([`Target::Fileroot`]), which only a whole import names,
//! `import fileroot` or `import fileroot as fr`, in a workspace's file, a
//! lone file or a session's input, and whose four members are read only as
//! `fileroot.NAME` through that name (each a target too, numbered after the
//! view's nodes, so that a reference resolves to it as to any member, and
//! mentions it, needing no file): `fileroot.this`, the directory of the
//! file the expression is written in; `fileroot.workspace`, the root;
//! `fileroot.caller`, the directory of the file whose code made the call
//! the running function runs for; and `fileroot.helper()`, after which the
//! running function's calls pass its own caller on (see [`crate::eval`],
//! "Callers and helpers", and the checker's "Imports"). A member or `*`
//! import of it is "`fileroot`'s members are read as `fileroot.caller`",
//! naming the member imported, `caller` for a `*`, and an import of a
//! name it lacks "`fileroot` has no `nope`". The scan looks the members'
//! names up, never interning them, once the files are parsed
//! (`Scan::fileroot_names`), since the names are lent to each file as it
//! is checked.
//!
//! Every path `fileroot` gives is absolute, without a trailing `/` (but `/`
//! itself), and is what the driver resolved for the files: the caller of
//! [`check_with_standard`] gives the workspace's root, which the CLI found
//! above the canonical path of the file it runs (`fs::canonicalize`, once,
//! so that it is canonical too), and [`check_lone_with_standard`] the lone
//! file's directory, which the CLI takes from the file's canonical path; a
//! session's root is the process's working directory as it starts,
//! canonical ([`working_directory`]), which [`check`], [`check_with`],
//! [`check_lone`] and [`crate::run`], told none, take too. Once a scan has
//! parsed its files, the driver *locates* them (`Scan::locate`): it builds
//! the run's table of directories, the root first, always, at index 0, then
//! each other directory with a file, once, by its absolute path, the root
//! joined to its path relative to the root, the standard files' `<stdlib>`
//! once; each file's `fileroot.this` is its directory's, the standard files'
//! `<stdlib>`, which nothing should rely on, and its `fileroot.workspace`
//! the root (a session's input: its root, which is both). The checker
//! gives each file's code its directory's index ([`crate::check::Place`],
//! [`crate::ir::Ir::dir`]), and every run its table, which
//! `fileroot.caller` reads. A session's inputs are each in its root, at
//! index 0 of the table its start built, which no input extends.
//!
//! Every file whose first items do not include `#![no_prelude]`, the
//! standard files included, and every session's input, begins as if with
//! `#[shadowable] import * from prelude`: the parse gives its tree the
//! import, an item written nowhere and kept apart from its body
//! ([`Ast::implicit_import`]), so that no span or line moves; the scan
//! takes it first of all in the static order (see [Shadowable
//! declarations](#shadowable-declarations)), its aliases last among the
//! file's ([`Alias::implicit`]), so that the others keep their numbers. So
//! any declaration or import of one of its names, or of a path below one,
//! shadows the prelude's silently, wherever it is, and the clash family
//! ignores them; the aliases of those it shadows are gone from the file.
//! `prelude.gcsl` is marked `#![no_prelude]` itself, and a file so marked
//! has no prelude name but those it imports. An explicit `import * from
//! prelude` is an ordinary `*`, and conflicts as one.
//!
//! A file outside any workspace (`gcsl run` of a lone file, [`check_lone`])
//! is checked and run as a workspace of one directory holding just it,
//! beside the standard library, loading what it needs as a workspace does:
//! it may import `builtins` and the standard files, and an import of
//! anything else is the checker's error, "`lib` is not a built-in or
//! standard module, and there is no `workspace.gcsl` above this file", with
//! the file's other errors. A session's input is scanned so too
//! (`check_input`), though a session is not a workspace (see
//! [`crate::session`]), beside every standard file, as the session loaded
//! them when it started, so that their cells are numbered as they were
//! there; it inherits the imports of the session's earlier inputs, after
//! its own, last in the static order, but for each it imports again or
//! declares at its top level. An input is no file: one marked
//! `#![no_prelude]` is "`#![no_prelude]` applies only to a file"
//! (`no_prelude_in_input`). Its `#[shadowable]` marks are a file's within
//! it, and change nothing between inputs, a later one shadowing an earlier
//! one's names already.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::mem;
use std::path::PathBuf;
use std::rc::Rc;

use crate::Outcome;
use crate::ast::{
    self, Arg, ArgKind, Ast, AstBuilder, DefTypeId, DefTypeRole, Expr, ExprKind, ImportDecl,
    ImportForm, ImportMember, Item, ItemId, ItemKind, Param, PathSegment, Syntax, TypeExprId,
    TypeExprKind, TypeLink, TypeStore, TypeStoreMark, AttributeId,
};
#[cfg(test)]
use crate::check::ViewRecord;
use crate::check::{
    BUILTIN_ATTRIBUTES, BUILTIN_TYPE_NAMES, Builtin, CheckOutput, Checker, Fileroot, KnownParams,
    Place, Record, Scope, Slot, Types, check_types,
};
use crate::diagnostic::{self, Diagnostic, InternalError, Severity};
use crate::eval::{MemoTables, TempDirs};
use crate::index::{Index, Indexed};
use crate::ir::Ir;
use crate::lexer::{self, TokenKind};
use crate::mentions::{self, FieldTree, Mentions, Need, Resolve, Schedule, Sweep, TypeRead};
use crate::names::{self, Name, Names};
use crate::parser;
use crate::session;
use crate::source::{SourceFile, Span};
use crate::strings::Strings;
use crate::types::{Laziness, TyId, TypeTable};
use crate::value::{Layout, Quantity, Value};

/// The key of a root in [`Scan`]'s index of nodes by parent and name.
const ROOT: u32 = u32::MAX;

/// The path of the standard library's directories, which no path of a
/// workspace can name, `<` being in no name: each standard file is
/// `<stdlib>/STEM.gcsl`, which is how a diagnostic of one names it, in a
/// directory of its own with this path.
pub const STANDARD_DIR: &str = "<stdlib>";

/// The name of the module of built-in functions.
pub(crate) const BUILTINS: &str = "builtins";

/// The stem of the prelude, the standard file every scan loads.
const PRELUDE: &str = "prelude";

/// Every standard file ([`crate::STANDARD`]) as a source, in its order,
/// each named `<stdlib>/STEM.gcsl`: what a session, which loads them all,
/// renders a diagnostic of one against. A run of a workspace or a lone file
/// loads only those it needs: [`check_with_standard`] says which.
#[must_use]
pub fn standard_sources() -> Vec<SourceFile> {
    crate::STANDARD
        .iter()
        .filter_map(|&(stem, text)| standard_source(stem, text))
        .collect()
}

/// The standard file of stem `stem` and text `text` as a source, named
/// `<stdlib>/STEM.gcsl`.
fn standard_source(stem: &str, text: &str) -> Option<SourceFile> {
    SourceFile::new(format!("{STANDARD_DIR}/{stem}.gcsl"), text).ok()
}

/// The standard library a scan is given (see the module documentation,
/// "The standard library"): a table of standard files, and which of them
/// it loads.
#[derive(Clone, Copy, Debug)]
struct Library<'a> {
    /// The standard files, each with its stem, in path order: always
    /// [`crate::STANDARD`] but in tests.
    table: &'a [(&'a str, &'a str)],
    /// Whether the scan loads every standard file, as a session does, or
    /// only the prelude and those its files need.
    every: bool,
}

impl Library<'_> {
    /// The standard library of a workspace or a lone file: only the
    /// standard files it needs.
    const NEEDED: Library<'static> = Library {
        table: &crate::STANDARD,
        every: false,
    };

    /// The standard library of a session: every standard file.
    const EVERY: Library<'static> = Library {
        table: &crate::STANDARD,
        every: true,
    };

    /// The index in the table of the standard file of stem `stem`.
    fn find(&self, stem: &str) -> Option<usize> {
        self.table.iter().position(|&(name, _)| name == stem)
    }

    /// The standard files of the table at `loaded`, its indices, as
    /// sources, by input number.
    fn sources(&self, loaded: &[usize]) -> Vec<SourceFile> {
        loaded
            .iter()
            .filter_map(|&at| self.table.get(at))
            .filter_map(|&(stem, text)| standard_source(stem, text))
            .collect()
    }
}

/// The values of the cells of the built-in functions, the first slots of a
/// frame, in the order of [`Builtin::ALL`]: what the driver writes as it
/// lays the frame out, before any file runs.
fn builtin_cells() -> Vec<Value> {
    Builtin::ALL.into_iter().map(Value::Builtin).collect()
}

/// The layout of the record `exec` returns, and `exec_live`, its fields
/// ([`Builtin::EXEC_FIELDS`]) interned into `names`, the run's, as the
/// checker interns them for `exec`'s type, so that `r.status` reads it:
/// built once per run as the frame is laid out, and shared by every record
/// `exec` or `exec_live` builds in the run, whatever file, input or function
/// calls it (see [`crate::eval`]).
fn exec_layout(names: &mut Names) -> Rc<Layout> {
    Rc::new(Layout {
        names: Builtin::EXEC_FIELDS
            .iter()
            .map(|text| names.intern(text))
            .collect(),
        texts: Builtin::EXEC_FIELDS
            .iter()
            .map(|&text| Box::from(text))
            .collect(),
    })
}

/// One file of a workspace, scanned.
#[derive(Debug)]
pub struct File {
    /// Its path relative to the root.
    pub path: String,
    /// Its directory, an index in [`Scan::dirs`].
    pub dir: u32,
    /// Its tree. Its names table is lent out ([`Ast::replace_names`]); the
    /// workspace's is [`Scan::names`].
    pub ast: Ast,
    /// Every name it spells, in [`Name`] order: its identifiers, and each
    /// plain string literal whose text is a name (a quoted field name).
    /// Its checker looks up no other name, so the scope it is checked from
    /// holds only the committed declarations of these (see [`check`]).
    pub spelled: Vec<Name>,
    /// Its declarations and what each of its items mentions, resolved
    /// against its view ([`FileScope`]).
    pub mentions: Mentions,
    /// For each node of its own field tree, the merged node of its path.
    pub global: Vec<u32>,
    /// For each of its own top-level functions, its index in
    /// [`Scan::functions`].
    pub functions: Vec<u32>,
    /// The names it imports, in source order (a `*` gives one per root),
    /// then those a session's input inherits, then the implicit prelude's;
    /// but a member or `*` import that something shadows, gone from its
    /// view (see the module documentation, "Shadowable declarations").
    pub aliases: Vec<Alias>,
    /// Its view: the nodes several of its sources have, merged.
    pub view: View,
    /// When its field nodes are computed: its own nodes that another file
    /// computes (shared interior nodes it is not the last contributor of)
    /// are not in it; the nodes of its view with a leaf of it under them
    /// are, if it reads them whole ([`Mentions::views`]).
    pub schedule: Schedule,
    /// Its own `type` declarations, by their indices in [`Scan::types`], in
    /// source order (see [Types](#types)).
    pub types: Vec<u32>,
    /// The types it imports by member or `*`, in source order.
    pub type_imports: Vec<TypeImport>,
    /// Its view of types: the declaration each name of a type names in it,
    /// by its number in [`Scan::type_store`].
    pub type_view: HashMap<Name, u32>,
    /// Its definitions made with a def-type whose directory declares no
    /// meta of their path and def-type, by their uses of their def-types,
    /// in source order: an error of each whose def-type declares a meta (see
    /// [Def-types](#def-types)).
    pub unpaired: Vec<DefTypeId>,
    /// The positions among its items of the declarations the scan dropped,
    /// recovering from an error of the scan: a later definition of a field
    /// another file defined first, a meta declared twice or with no
    /// definition. Each is in no tree, as a shadowed declaration is, and
    /// still checked as written (see [Scan errors](#scan-errors)).
    pub dropped: Vec<usize>,
    /// The names it reads as `Error`, silently, recovering from an error of
    /// the scan: what an import that failed binds, a name an import clashes
    /// on, and the root of a conflict of its view or its view of types, in
    /// [`Name`] order (see [Scan errors](#scan-errors)).
    pub poisoned: Vec<Name>,
    /// Whether it imports names the scan cannot know, by a `*` import that
    /// failed, so that a name that names nothing in it is no error (see
    /// [Scan errors](#scan-errors)).
    pub blind: bool,
    /// Its `import` items the scan reported an error of, in source order,
    /// each with the module its path names, if it names one.
    pub failed_imports: Vec<(ItemId, Option<Target>)>,
}

impl File {
    /// Whether `alias`, one of the file's, is imported by an import marked
    /// `#[shadowable]` (the implicit one of the prelude among them), which
    /// makes every leaf it brings shadowable (see the module documentation,
    /// "Shadowable declarations"); an import a session's input inherits is
    /// not, which only the input's declarations shadow
    /// ([`inherit_imports`]).
    #[must_use]
    pub fn marked(&self, alias: &Alias) -> bool {
        alias.item.is_some_and(|item| self.ast.is_shadowable(item))
    }

    /// The file at `path` in directory `dir`, parsed to `ast`, spelling
    /// `spelled`, with its declarations found and nothing else yet.
    fn new(path: &str, dir: u32, ast: Ast, spelled: Vec<Name>) -> Self {
        let mentions = mentions::scan(&ast);
        Self {
            path: path.to_owned(),
            dir,
            ast,
            spelled,
            mentions,
            global: Vec::new(),
            functions: Vec::new(),
            aliases: Vec::new(),
            view: View::default(),
            schedule: Schedule::default(),
            types: Vec::new(),
            type_imports: Vec::new(),
            type_view: HashMap::new(),
            unpaired: Vec::new(),
            dropped: Vec::new(),
            poisoned: Vec::new(),
            blind: false,
            failed_imports: Vec::new(),
        }
    }

    /// Drops the declarations at `positions` among its items (see
    /// [`File::dropped`]): its declarations found again with them shadowed,
    /// as well as those already are.
    fn drop_items(&mut self, positions: &[usize]) {
        self.dropped.extend_from_slice(positions);
        self.dropped.sort_unstable();
        self.dropped.dedup();
        let mut shadowed = self.mentions.shadowed.clone();
        for &position in &self.dropped {
            if let Some(entry) = shadowed.get_mut(position) {
                *entry = true;
            }
        }
        self.mentions = mentions::scan_except(&self.ast, &shadowed);
    }

    /// Makes reads of `name` in it `Error`, silently (see
    /// [`File::poisoned`]).
    fn poison(&mut self, name: Name) {
        if let Err(at) = self.poisoned.binary_search(&name) {
            self.poisoned.insert(at, name);
        }
    }
}

/// A `type` declaration of a file of a scan (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclaredType {
    /// Its file.
    pub file: u32,
    /// Its `type` item.
    pub item: ItemId,
    /// The name it declares.
    pub name: Name,
    /// Where the name is written.
    pub span: Span,
    /// The type it stands for, in its file's tree.
    pub ty: TypeExprId,
}

/// A type a file imports by member, `import NAME from PATH`, or by `*`
/// (see [Types](#types)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeImport {
    /// The name it is bound as.
    pub name: Name,
    /// Where it is bound: the member, the name it is imported as, or the
    /// `*`.
    pub span: Span,
    /// Its declaration, by its index in [`Scan::types`].
    pub decl: u32,
}

/// A directory of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    /// Its path relative to the root, empty for the root.
    pub path: String,
    /// The directory above it; `None` for the root.
    pub parent: Option<u32>,
}

/// A node of a directory's merged tree: a cell of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Its last segment.
    pub name: Name,
    /// The node it is a child of; `None` for a root.
    pub parent: Option<u32>,
    /// Its children, in order of their first file, then of their first
    /// item in it.
    pub children: Vec<u32>,
    /// The directory whose tree it is in.
    pub dir: u32,
    /// The file it is a leaf of; `None` for an interior node.
    pub leaf: Option<u32>,
    /// The files with a leaf at or under it, in path order.
    pub contributors: Vec<u32>,
    /// Whether it is shadowable: a leaf whose declaration is marked
    /// `#[shadowable]`, or an interior node all of whose leaves are (see the
    /// module documentation, "Shadowable declarations"). A built-in
    /// function's is not.
    pub shadowable: bool,
}

/// A top-level function of a file of the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    /// Its file.
    pub file: u32,
    /// Its index among the file's own functions.
    pub index: u32,
}

/// What an imported name aliases (see the module documentation,
/// "Imports").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A directory's merged tree, a synthetic node whose children are the
    /// directory's roots; read whole, the record of every root.
    Dir(u32),
    /// A file's own tree, a synthetic node whose children are its roots;
    /// read whole, its own record, or its value if it is a value file.
    File(u32),
    /// A node of a file's own tree that is not the whole of its merged node
    /// (an interior node other files contribute to): the file and the
    /// file's own node. Read whole, the file's part of it.
    Part {
        /// The file.
        file: u32,
        /// Its own node.
        node: u32,
    },
    /// A merged node, a cell of the workspace, reached through an import.
    Node(u32),
    /// A node of a file's view that several sources have and no leaf of
    /// the file is under, by its index in [`Scan::views`]: read whole, the
    /// record its file assembles of every source's children.
    View(u32),
    /// The `fileroot` module (`None`), which only a whole import names, or
    /// one of its members, reached through it: no cell and no file, its
    /// members read as `fileroot.NAME` only, each as the checker makes it
    /// (see [The standard library](#the-standard-library)).
    Fileroot(Option<Fileroot>),
}

/// A name a file imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The name bound.
    pub name: Name,
    /// Where it is bound: the alias, the member, the `*`, or the path's
    /// last segment; nowhere (`0..0`) for the implicit `import * from
    /// prelude` and for an import a session's input inherits from an
    /// earlier input, whose source is not this one.
    pub span: Span,
    /// The `import` item (the implicit one, [`Ast::implicit_import`],
    /// included); `None` for an import a session's input inherits from an
    /// earlier input.
    pub item: Option<ItemId>,
    /// What it names.
    pub target: Target,
    /// Whether it is a whole import (`import PATH`, `import PATH as NAME`),
    /// a name of its own; a member or a root of a `*` is merged into the
    /// file's view instead. Whether its import is marked `#[shadowable]`,
    /// which makes every leaf it brings shadowable, its file's tree says
    /// ([`File::marked`]).
    pub whole: bool,
    /// Whether it is a root of the implicit `#[shadowable] import * from
    /// prelude`, first of all in the static order (see the module
    /// documentation, "Shadowable declarations").
    pub implicit: bool,
    /// Whether a session's input inherits it from an earlier input, which
    /// the input's declarations shadow and which clashes with nothing (see
    /// [`inherit_imports`]).
    pub inherited: bool,
}

/// One source's node at a node of a file's view (see the module
/// documentation, "The view of a file").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewPart {
    /// The source, root directory first: the directories of the file's
    /// chain from the root (0) down to its own, then its imported members
    /// and `*` roots, by their index in [`File::aliases`] after those.
    pub source: u32,
    /// The node: a merged node, or a file's part of one.
    pub target: Target,
    /// Whether it is a node of a directory of the file's chain, reached by
    /// its own path (an import of such a node at its own path is the same
    /// cell, merged into it).
    pub chain: bool,
    /// Whether it is reached through an import marked `#[shadowable]`,
    /// which makes every leaf at or under it shadowable whatever its
    /// declaration says (see the module documentation, "Shadowable
    /// declarations"). Of one cell reached several ways, it is marked only
    /// if every way is.
    pub marked: bool,
}

impl ViewPart {
    /// The merged node `node` of the directory of the file's chain at
    /// position `source`.
    const fn chain(source: u32, node: u32) -> Self {
        Self {
            source,
            target: Target::Node(node),
            chain: true,
            marked: false,
        }
    }
}

/// A node of a file's view that several sources have, the same cell
/// reached twice taken once (see the module documentation, "The view of a
/// file").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewNode {
    /// Its last segment.
    pub name: Name,
    /// The view node it is a child of, by its index in [`Scan::views`];
    /// `None` for a root of the view.
    pub parent: Option<u32>,
    /// The file whose view it is of.
    pub file: u32,
    /// What it merges, root directory first, each an interior node.
    pub parts: Vec<ViewPart>,
    /// The file's own node of its path, when a leaf of the file is under
    /// it: it is then a node of the file's schedule.
    pub own: Option<u32>,
}

/// What a file's view makes of a name that several sources have at one
/// node: a node of the view, or the one node left once the same cell
/// reached twice is taken once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merged {
    /// A node of the view, by its index in [`Scan::views`].
    View(u32),
    /// A single node.
    One(ViewPart),
}

/// A file's view: its nodes that several sources have (see the module
/// documentation, "The view of a file").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// Where its nodes start in [`Scan::views`].
    pub start: u32,
    /// The number of its nodes with a leaf of the file under them, which
    /// come first.
    pub own: u32,
    /// Where its nodes end in [`Scan::views`].
    pub end: u32,
    /// What it makes of each name several sources have, by the node it is
    /// under ([`ROOT`] for a root, else its index in [`Scan::views`]) and
    /// the name.
    merged: HashMap<(u32, Name), Merged>,
}

/// The scanned workspace: what [`scan`] finds before anything is checked.
#[derive(Debug)]
pub struct Scan {
    /// The files, in path order; a file's index is its input number.
    pub files: Vec<File>,
    /// The directories, each after the directory above it.
    pub dirs: Vec<Dir>,
    /// The nodes of every directory's merged tree; a node's index is its
    /// slot.
    pub nodes: Vec<Node>,
    /// The top-level functions of every file.
    pub functions: Vec<Function>,
    /// The nodes of every file's view that several sources have, each
    /// file's together ([`File::view`]).
    pub views: Vec<ViewNode>,
    /// For each file, the files it needs, each once, in index order.
    pub needs: Vec<Vec<u32>>,
    /// The files in the order they are checked and run.
    pub order: Vec<u32>,
    /// The names of every file.
    pub names: Names,
    /// The nodes by directory, parent ([`ROOT`] for a root) and name.
    by_name: HashMap<(u32, u32, Name), u32>,
    /// The roots of each directory's tree, by directory, in order.
    roots: Vec<Vec<u32>>,
    /// The `def`s by directory and name, as indices in `functions`.
    defs: HashMap<(u32, Name), u32>,
    /// The position of each file in `order`.
    rank: Vec<usize>,
    /// For each file, the number of the own nodes of the files before it:
    /// where its [`Target::Part`]s start among the targets' numbers.
    part_base: Vec<u32>,
    /// The value files some file imports, each with the slot of its value's
    /// cell and its name (the file's stem), in path order. Their slots
    /// follow the merged nodes'.
    values: Vec<(u32, Name)>,
    /// The number of standard files the scan loaded, which are files
    /// `0..standard`, each in a directory of its own (see the module
    /// documentation, "The standard library").
    standard: u32,
    /// The directory of the `builtins` module, whose nodes are the first
    /// [`Builtin::ALL`]`.len()` of `nodes`.
    pub(crate) builtins_dir: u32,
    /// The name of each member of the `fileroot` module, in the order of
    /// [`Fileroot::ALL`], if some file spells it: looked up, never interned,
    /// once the files are parsed, since the names are lent to each file as
    /// it is checked, while its references are resolved.
    fileroot_names: [Option<Name>; 4],
    /// Whether the files given are one file outside any workspace (a lone
    /// file, a session's input), whose imports of anything but `builtins`
    /// and the standard files the checker reports.
    lone: bool,
    /// For a session's input ([`check_input`]): the number of cells of the
    /// standard library, which keep their slots, and the slot of each cell
    /// after them, past the slots of the session's earlier inputs (see
    /// [`Scan::slot`]).
    relocation: Option<(u32, Vec<Slot>)>,
    /// The run's table of directories, the root's first ([`Scan::locate`]),
    /// by which the code of each file names its directory: what
    /// `fileroot.caller` reads (see [`crate::eval`], "Callers and
    /// helpers"). Empty until the scan is located.
    dir_texts: Vec<Rc<str>>,
    /// Each file's directory, by its index in `dir_texts`, by input number.
    file_dirs: Vec<u32>,
    /// The `type` declarations of every file, in path order, each file's in
    /// source order: but one named like a built-in type, and a second of
    /// one name in one file, which the checker reports (see [Types](#types)).
    pub types: Vec<DeclaredType>,
    /// The types of each directory, by its index: its files' declarations,
    /// as indices in `types`, in order, the first of each name.
    dir_types: Vec<Vec<u32>>,
    /// The definitions of the types: each of `types` is the declaration
    /// numbered `type_base` more than its index. A session's input's scan
    /// has the session's, its earlier inputs' declarations first.
    pub type_store: TypeStore,
    /// The number in `type_store` of the first of `types`.
    pub(crate) type_base: u32,
    /// The types a session's input inherits from the earlier inputs, each
    /// with its declaration's number in `type_store`: the last source of its
    /// view, shadowed by anything (see [`crate::session`]).
    inherited_types: Vec<(Name, u32)>,
    /// The def-types a session's input inherits from the earlier inputs,
    /// each by its path, with its namespace's key: what a `DT` names that
    /// nothing of the input does (see [Def-types](#def-types)).
    inherited_def_types: Vec<(Vec<Name>, Name)>,
    /// For a session's input, its number among the session's inputs, which
    /// the keys of the def-types it declares are made with, in place of its
    /// index among the scan's files, which every input shares.
    input_number: Option<u32>,
    /// The lexical and syntax errors of every file, each naming its file,
    /// in path then source order (see [Syntax errors](#syntax-errors)): a
    /// file with some is scanned as its parser recovered it.
    syntax_errors: Vec<Diagnostic>,
    /// The errors of the scan itself, each naming its file, which it
    /// recovered from (see [Scan errors](#scan-errors)): but those of a file
    /// that may see a file with syntax errors.
    recovered: Vec<Diagnostic>,
    /// The declarations dropped as later definitions of a field defined
    /// first elsewhere, each by its file and its position among the file's
    /// items (see [Scan errors](#scan-errors)).
    conflict_items: Vec<(u32, usize)>,
    /// The CONFLICTED merged nodes, each with the declarations dropped as
    /// later definitions of it, by file and item, in path then source order:
    /// a node at which a dropped declaration's path meets the tree (see
    /// [Scan errors](#scan-errors)).
    conflicted: HashMap<u32, Vec<(u32, ItemId)>>,
    /// The conflicted nodes and every node above one, whose cells are read
    /// as `Error` (see [Scan errors](#scan-errors)).
    poisoned_nodes: HashSet<u32>,
    /// The `type` declarations, by their indices in `types`, that a
    /// declaration of their name in another file of their directory
    /// conflicts with: each is erroneous (see [Scan errors](#scan-errors)).
    poisoned_types: Vec<u32>,
    /// Whether each file is in a cycle of files that need each other, or
    /// needs itself: such a file is not checked (see [Scan
    /// errors](#scan-errors)).
    cyclic: Vec<bool>,
}

impl Scan {
    /// The lexical and syntax errors of every file, each naming its file by
    /// its input number, in path then source order (see [Syntax
    /// errors](#syntax-errors)).
    #[must_use]
    pub fn syntax_errors(&self) -> &[Diagnostic] {
        &self.syntax_errors
    }

    /// The errors of the scan itself, which it recovered from, each naming
    /// its file by its input number, in path then source order (see [Scan
    /// errors](#scan-errors)).
    #[must_use]
    pub fn scan_errors(&self) -> &[Diagnostic] {
        &self.recovered
    }

    /// The syntax errors and the errors of the scan, in path then source
    /// order: what a scan with either reports, and why nothing of it runs.
    fn errors(&self) -> Vec<Diagnostic> {
        merge_files(
            self.syntax_errors
                .iter()
                .chain(&self.recovered)
                .cloned()
                .collect(),
            Vec::new(),
        )
    }

    /// Whether the merged node `node` is conflicted, or above a node that
    /// is, so that it is read as `Error` (see [Scan errors](#scan-errors)).
    #[must_use]
    pub fn is_poisoned(&self, node: u32) -> bool {
        !self.poisoned_nodes.is_empty() && self.poisoned_nodes.contains(&node)
    }

    /// The declarations the scan dropped as later definitions of the merged
    /// node `node`, each by its file and item, if it is conflicted (see
    /// [Scan errors](#scan-errors)).
    #[must_use]
    pub fn conflicting(&self, node: u32) -> &[(u32, ItemId)] {
        self.conflicted.get(&node).map_or(&[], Vec::as_slice)
    }

    /// Whether file `index` is in a cycle of files that need each other, or
    /// needs itself: it is not checked (see [Scan errors](#scan-errors)).
    #[must_use]
    pub fn is_cyclic(&self, index: u32) -> bool {
        self.cyclic.get(index as usize) == Some(&true)
    }

    /// Whether file `index` has syntax errors, which the scan and the
    /// checker take the tree its parser recovered for.
    #[must_use]
    pub fn is_broken(&self, index: u32) -> bool {
        self.files
            .get(index as usize)
            .is_some_and(|file| file.ast.syntax() != Syntax::Clean)
    }

    /// Whether each file has syntax errors or is in a cycle, by input
    /// number: such a file always fails, and is not checked if in a cycle.
    fn broken_files(&self) -> Vec<bool> {
        (0..count(self.files.len()))
            .map(|index| self.is_broken(index) || self.is_cyclic(index))
            .collect()
    }

    /// The diagnostics `found` of the failed check of file `index`, as they
    /// are reported, each naming the file: the errors alone of a file with
    /// syntax errors, a warning of which may be of source that could not be
    /// parsed.
    fn failure(&self, index: u32, mut found: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.is_broken(index) {
            found.retain(|diagnostic| diagnostic.severity == Severity::Error);
        }
        in_file(found, index)
    }

    /// Whether file `index` is not checked: its tree is unbalanced, or it is
    /// in a cycle (see [Scan errors](#scan-errors)).
    fn unchecked(&self, index: u32) -> bool {
        self.is_unbalanced(index) || self.is_cyclic(index)
    }

    /// Whether file `index`'s tree is [`Syntax::Unbalanced`], its braces not
    /// matching, which is not checked.
    fn is_unbalanced(&self, index: u32) -> bool {
        self.files
            .get(index as usize)
            .is_some_and(|file| file.ast.syntax() == Syntax::Unbalanced)
    }

    /// Whether the view of file `index` sees a file with syntax errors, in
    /// which a declaration may be missing or misplaced (see [Syntax
    /// errors](#syntax-errors)): it has them, a file of its directory or of
    /// one above it has, or it imports a name, a type, a file or a
    /// directory of a file that has. A name it reads that names nothing is
    /// no error then; nor is it in a file that imports names the scan
    /// cannot know ([`File::blind`]).
    #[must_use]
    pub fn sees_broken(&self, index: u32) -> bool {
        if self.syntax_errors.is_empty() && self.recovered.is_empty() {
            return false;
        }
        let Some(file) = self.files.get(index as usize) else {
            return false;
        };
        file.blind || self.sees_syntax_errors(index)
    }

    /// Whether the view of file `index` sees a file with syntax errors (see
    /// [`Scan::sees_broken`]), or an import of it that failed names a module
    /// with one, whose member it lacks may be declared in source that could
    /// not be parsed: the errors of the scan of such a file are not reported
    /// (see [Syntax errors](#syntax-errors)).
    fn sees_syntax_errors(&self, index: u32) -> bool {
        if self.syntax_errors.is_empty() {
            return false;
        }
        let Some(file) = self.files.get(index as usize) else {
            return false;
        };
        let chain: Vec<u32> = self.chain(file.dir).collect();
        let in_chain = (0u32..)
            .zip(&self.files)
            .any(|(at, other)| chain.contains(&other.dir) && self.is_broken(at));
        // A type by its index in `types`; one of a session's earlier input
        // is none of them.
        let broken_type = |at: u32| {
            self.types
                .get(at as usize)
                .is_some_and(|declared| self.is_broken(declared.file))
        };
        let broken_target = |target: Target| {
            self.target_files(target)
                .into_iter()
                .any(|at| self.is_broken(at))
        };
        in_chain
            || file.aliases.iter().any(|alias| broken_target(alias.target))
            || file
                .failed_imports
                .iter()
                .any(|&(_, target)| target.is_some_and(broken_target))
            || file
                .type_imports
                .iter()
                .any(|import| broken_type(import.decl))
            || file
                .type_view
                .values()
                .any(|&decl| decl.checked_sub(self.type_base).is_some_and(broken_type))
    }

    /// Of `errors`, the errors the stages of the scan found, those it
    /// reports: those of files that see no file with syntax errors
    /// ([`Scan::sees_syntax_errors`]), since in any other a declaration may
    /// be missing or misplaced (see [Syntax errors](#syntax-errors)).
    fn stage_errors(&self, errors: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.syntax_errors.is_empty() {
            return errors;
        }
        errors
            .into_iter()
            .filter(|error| {
                error
                    .input
                    .is_some_and(|input| !self.sees_syntax_errors(input))
            })
            .collect()
    }
    /// The directories from `dir` up to the root, `dir` first.
    pub fn chain(&self, dir: u32) -> impl Iterator<Item = u32> + '_ {
        std::iter::successors(Some(dir), |&at| {
            self.dirs.get(at as usize).and_then(|d| d.parent)
        })
    }

    /// The files `file` needs, transitively, with `file` and the standard
    /// files the scan loaded, which every run runs (the prelude and those
    /// some scanned file imports, see the module documentation, "The
    /// standard library"), in the order: the files to run to run `file`.
    #[must_use]
    pub fn needed(&self, file: u32) -> Vec<u32> {
        let mut wanted = vec![false; self.files.len()];
        let mut stack: Vec<u32> = (0..self.standard).collect();
        stack.push(file);
        while let Some(at) = stack.pop() {
            if let Some(seen) = wanted.get_mut(at as usize)
                && !*seen
            {
                *seen = true;
                stack.extend(self.needs.get(at as usize).into_iter().flatten());
            }
        }
        self.order
            .iter()
            .copied()
            .filter(|&f| wanted.get(f as usize) == Some(&true))
            .collect()
    }

    /// The position of each file in [`Scan::order`].
    fn ranks(&self) -> Vec<usize> {
        let mut rank = vec![0; self.files.len()];
        for (at, &file) in self.order.iter().enumerate() {
            if let Some(entry) = rank.get_mut(file as usize) {
                *entry = at;
            }
        }
        rank
    }

    /// The file that computes the merged node `node`: its leaf's file, or
    /// the last of its contributors in the order.
    #[must_use]
    pub fn computed_by(&self, node: u32) -> Option<u32> {
        let entry = self.nodes.get(node as usize)?;
        if entry.leaf.is_some() {
            return entry.leaf;
        }
        entry
            .contributors
            .iter()
            .copied()
            .max_by_key(|&f| self.rank.get(f as usize).copied())
    }

    /// The number of `target` among every target of the workspace: the
    /// directories, then the files, then the files' own nodes, then the
    /// merged nodes, then the views' nodes, then the `fileroot` module and
    /// its members, in the order of [`Fileroot::ALL`].
    fn target_number(&self, target: Target) -> u32 {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        let parts = self.parts();
        let nodes = dirs.saturating_add(files).saturating_add(parts);
        match target {
            Target::Dir(dir) => dir,
            Target::File(file) => dirs.saturating_add(file),
            Target::Part { file, node } => {
                let base = self.part_base.get(file as usize).copied().unwrap_or(0);
                dirs.saturating_add(files)
                    .saturating_add(base)
                    .saturating_add(node)
            }
            Target::Node(node) => nodes.saturating_add(node),
            Target::View(view) => nodes
                .saturating_add(count(self.nodes.len()))
                .saturating_add(view),
            Target::Fileroot(member) => self
                .fileroot_number()
                .saturating_add(member.map_or(0, |member| member.index() + 1)),
        }
    }

    /// The number of the `fileroot` module among every target, after the
    /// views' nodes; its members' follow it (see [`Scan::target_number`]).
    fn fileroot_number(&self) -> u32 {
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        dirs.saturating_add(files)
            .saturating_add(self.parts())
            .saturating_add(count(self.nodes.len()))
            .saturating_add(count(self.views.len()))
    }

    /// The target of number `number` (see [`Scan::target_number`]).
    fn target_of(&self, number: u32) -> Option<Target> {
        if let Some(at) = number.checked_sub(self.fileroot_number()) {
            return match at.checked_sub(1) {
                None => Some(Target::Fileroot(None)),
                Some(member) => Fileroot::ALL
                    .get(member as usize)
                    .map(|&member| Target::Fileroot(Some(member))),
            };
        }
        let (dirs, files) = (count(self.dirs.len()), count(self.files.len()));
        let Some(rest) = number.checked_sub(dirs) else {
            return Some(Target::Dir(number));
        };
        let Some(rest) = rest.checked_sub(files) else {
            return Some(Target::File(rest));
        };
        let Some(node) = rest.checked_sub(self.parts()) else {
            // The last file whose parts start at or before `rest`.
            let after = self.part_base.partition_point(|&base| base <= rest);
            let file = count(after.checked_sub(1)?);
            let base = self.part_base.get(file as usize).copied()?;
            return Some(Target::Part {
                file,
                node: rest.checked_sub(base)?,
            });
        };
        let Some(view) = node.checked_sub(count(self.nodes.len())) else {
            return Some(Target::Node(node));
        };
        ((view as usize) < self.views.len()).then_some(Target::View(view))
    }

    /// The slot of the cell of the value of `file`, a value file some file
    /// imports: its owner stores its result there, and an importer reads
    /// it ([`Target::File`] of a value file). `None` for any other file.
    #[must_use]
    pub fn value_slot(&self, file: u32) -> Option<Slot> {
        self.value_cell(file).map(|cell| self.slot(cell))
    }

    /// The slot of the cell numbered `cell` ([`Scan::cells`]): its number;
    /// in a session's input (`check_input`), the cells after the standard
    /// library's take the slots after the session's instead, in order, but
    /// a `def`'s, which is never written, which takes none
    /// ([`Slot::NONE`]), as a `def` of a session's input never did.
    #[must_use]
    pub fn slot(&self, cell: u32) -> Slot {
        match &self.relocation {
            Some((from, slots)) if cell >= *from => slots
                .get((cell - from) as usize)
                .copied()
                .unwrap_or(Slot::NONE),
            _ => Slot::at(cell),
        }
    }

    /// Whether the merged node `node` is a `def`'s, whose cell is its
    /// function's slot and never written itself.
    fn is_def(&self, node: u32) -> bool {
        self.nodes.get(node as usize).is_some_and(|entry| {
            entry.parent.is_none()
                && entry.leaf.is_some_and(|file| {
                    self.defs
                        .get(&(entry.dir, entry.name))
                        .and_then(|&id| self.functions.get(id as usize))
                        .is_some_and(|function| function.file == file)
                })
        })
    }

    /// The built-in function whose cell is the merged node `node`, if it is
    /// one: the first nodes are the `builtins` module's.
    #[must_use]
    pub fn builtin(&self, node: u32) -> Option<Builtin> {
        Builtin::ALL.get(node as usize).copied()
    }

    /// The number of standard files the scan loaded: files `0..` this are
    /// the standard library's (see the module documentation, "The standard
    /// library").
    #[must_use]
    pub fn standard(&self) -> u32 {
        self.standard
    }

    /// Whether the scan is of one file outside any workspace, a lone file
    /// or a session's input, which imports nothing but the standard
    /// library's modules.
    pub(crate) fn is_lone(&self) -> bool {
        self.lone
    }

    /// The standard file whose stem is `stem`, if the scan loaded one.
    fn standard_file(&self, stem: &str) -> Option<u32> {
        (0..self.standard).find(|&file| {
            self.files.get(file as usize).is_some_and(|f| {
                f.path
                    .strip_prefix(STANDARD_DIR)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .and_then(|rest| rest.strip_suffix(".gcsl"))
                    == Some(stem)
            })
        })
    }

    /// Locates the files at `root`, the absolute path of the workspace's
    /// root directory, canonical and without a trailing `/` (but `/`
    /// itself), for the `fileroot` module (see the module documentation,
    /// "The standard library"): builds the run's table of directories,
    /// [`Scan::dir_texts`], and each file's index in it. The root is first,
    /// always, so that its index is 0 in every run, as a session's inputs,
    /// each in its root, need; then each other directory with a file, once,
    /// in the order of [`Scan::dirs`], each by its absolute path, its path
    /// relative to the root joined to `root` with a `/` (no other after a
    /// `root` of `/`); the standard files' `<stdlib>` once, as its own
    /// path, though each standard file is a directory of its own.
    fn locate(&mut self, root: &str) {
        let mut texts: Vec<Rc<str>> = vec![Rc::from(root)];
        let mut found: HashMap<Rc<str>, u32> = HashMap::new();
        found.insert(Rc::from(root), 0);
        // The index of each directory of the scan, once it has one; the
        // root, dir 0 (`place_dirs` interns it first), has the first.
        let mut indices: Vec<Option<u32>> = vec![None; self.dirs.len()];
        if let Some(entry) = indices.first_mut() {
            *entry = Some(0);
        }
        let mut file_dirs = Vec::with_capacity(self.files.len());
        for file in &self.files {
            if let Some(&Some(at)) = indices.get(file.dir as usize) {
                file_dirs.push(at);
                continue;
            }
            let path = self
                .dirs
                .get(file.dir as usize)
                .map_or("", |d| d.path.as_str());
            let text: Rc<str> = if path.is_empty() {
                Rc::from(root)
            } else if path == STANDARD_DIR {
                Rc::from(path)
            } else if root.ends_with('/') {
                Rc::from(format!("{root}{path}"))
            } else {
                Rc::from(format!("{root}/{path}"))
            };
            let at = *found.entry(Rc::clone(&text)).or_insert_with(|| {
                texts.push(text);
                count(texts.len() - 1)
            });
            if let Some(entry) = indices.get_mut(file.dir as usize) {
                *entry = Some(at);
            }
            file_dirs.push(at);
        }
        self.dir_texts = texts;
        self.file_dirs = file_dirs;
    }

    /// Where file `index` is, as [`Scan::locate`] found it: its directory's
    /// index in the run's table, which is its `fileroot.this`, and the
    /// root's, its `fileroot.workspace`. Directory 0 and empty texts for a
    /// scan not located, which no run has.
    fn place(&self, index: u32) -> Place {
        let dir = self.file_dirs.get(index as usize).copied().unwrap_or(0);
        let text = |at: u32| {
            self.dir_texts
                .get(at as usize)
                .map_or_else(|| Rc::from(""), Rc::clone)
        };
        Place {
            dir,
            this: text(dir),
            workspace: text(0),
        }
    }

    /// The number of the cell of the value of `file`, a value file some
    /// file imports, among the workspace's cells ([`Scan::cells`]): after
    /// the merged nodes. Like a merged node's, its slot is [`Slot::at`] its
    /// number. `None` for any other file.
    pub(crate) fn value_cell(&self, file: u32) -> Option<u32> {
        let at = self.values.iter().position(|&(f, _)| f == file)?;
        Some(count(self.nodes.len()).saturating_add(count(at)))
    }

    /// The number of the workspace's cells: the merged nodes, then the
    /// imported value files' values. Every other slot is a file's own.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.nodes.len() + self.values.len()
    }

    /// The names of the cells' slots, by slot.
    fn cell_names(&self) -> Vec<Name> {
        self.nodes
            .iter()
            .map(|node| node.name)
            .chain(self.values.iter().map(|&(_, name)| name))
            .collect()
    }

    /// The number of every file's own nodes.
    fn parts(&self) -> u32 {
        self.part_base
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_add(self.files.last().map_or(0, |file| count(file.global.len())))
    }

    /// The child of the target `target` named `name`, as a reference
    /// through an import means it.
    pub(crate) fn target_child(&self, target: Target, name: Name) -> Option<Target> {
        match target {
            Target::Dir(dir) => self
                .by_name
                .get(&(dir, ROOT, name))
                .map(|&n| Target::Node(n)),
            Target::File(file) => {
                let dir = self.files.get(file as usize)?.dir;
                let merged = *self.by_name.get(&(dir, ROOT, name))?;
                self.in_file(file, merged)
            }
            Target::Part { file, node } => {
                let entry = self.files.get(file as usize)?;
                let parent = *entry.global.get(node as usize)?;
                let merged = *self.by_name.get(&(entry.dir, parent, name))?;
                self.in_file(file, merged)
            }
            Target::Node(node) => {
                let dir = self.nodes.get(node as usize)?.dir;
                self.by_name
                    .get(&(dir, node, name))
                    .map(|&n| Target::Node(n))
            }
            Target::View(view) => match self.view_child(view, name)? {
                Merged::View(child) => Some(Target::View(child)),
                Merged::One(part) => Some(part.target),
            },
            Target::Fileroot(None) => Fileroot::ALL
                .into_iter()
                .zip(self.fileroot_names)
                .find(|&(_, named)| named == Some(name))
                .map(|(member, _)| Target::Fileroot(Some(member))),
            Target::Fileroot(Some(_)) => None,
        }
    }

    /// The child named `name` of the view node `view`: a view node of its
    /// own when several of its parts have one, else the one part's child.
    fn view_child(&self, view: u32, name: Name) -> Option<Merged> {
        let node = self.views.get(view as usize)?;
        let file = self.files.get(node.file as usize)?;
        if let Some(&merged) = file.view.merged.get(&(view, name)) {
            return Some(merged);
        }
        node.parts.iter().find_map(|part| {
            let target = self.target_child(part.target, name)?;
            Some(Merged::One(ViewPart { target, ..*part }))
        })
    }

    /// The names of the children of the view node `view`, in the view's
    /// order: by first definition, its parts root directory first, each in
    /// its own order.
    pub(crate) fn view_names(&self, view: u32) -> Vec<Name> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        let parts = self.views.get(view as usize).map_or(&[][..], |v| &v.parts);
        for part in parts {
            for name in self.child_names(part.target) {
                if seen.insert(name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// The names of the children of `target`, a merged node or a file's
    /// part of one, in its order.
    pub(crate) fn child_names(&self, target: Target) -> Vec<Name> {
        match target {
            Target::Node(node) => self.nodes.get(node as usize).map_or_else(Vec::new, |n| {
                n.children
                    .iter()
                    .filter_map(|&c| self.nodes.get(c as usize).map(|c| c.name))
                    .collect()
            }),
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| {
                    let tree = &f.mentions.tree;
                    let children = &tree.nodes.get(node as usize)?.children;
                    Some(
                        children
                            .iter()
                            .filter_map(|&c| tree.nodes.get(c as usize).map(|n| n.name))
                            .collect(),
                    )
                })
                .unwrap_or_default(),
            Target::View(view) => self.view_names(view),
            Target::Dir(_) | Target::File(_) | Target::Fileroot(_) => target_roots(self, target),
        }
    }

    /// The files with a leaf at or under the part `part` of a view node.
    fn part_owners(&self, part: ViewPart) -> Vec<u32> {
        self.target_files(part.target)
    }

    /// The merged node `merged` as a node of `file`'s own tree: the merged
    /// node when it is wholly `file`'s, else `file`'s part of it; `None`
    /// when `file` has nothing at it.
    fn in_file(&self, file: u32, merged: u32) -> Option<Target> {
        let entry = self.nodes.get(merged as usize)?;
        if entry.leaf == Some(file) || entry.contributors == [file] {
            return Some(Target::Node(merged));
        }
        let own = self
            .files
            .get(file as usize)?
            .global
            .iter()
            .position(|&g| g == merged)?;
        Some(Target::Part {
            file,
            node: count(own),
        })
    }

    /// The files a reference through an import to `target` needs: every
    /// file of a directory, the file of a file or a part, a merged node's
    /// leaf's file or its contributors, every owner of every part of a view
    /// node; none for the `fileroot` module, which no file computes.
    #[must_use]
    pub fn target_files(&self, target: Target) -> Vec<u32> {
        match target {
            Target::Fileroot(_) => Vec::new(),
            Target::Dir(dir) => (0u32..)
                .zip(&self.files)
                .filter(|(_, file)| file.dir == dir)
                .map(|(index, _)| index)
                .collect(),
            Target::File(file) | Target::Part { file, .. } => vec![file],
            Target::Node(node) => self.nodes.get(node as usize).map_or_else(Vec::new, |n| {
                n.leaf
                    .map_or_else(|| n.contributors.clone(), |leaf| vec![leaf])
            }),
            Target::View(view) => {
                let parts = self.views.get(view as usize).map_or(&[][..], |v| &v.parts);
                let mut files: Vec<u32> = parts
                    .iter()
                    .flat_map(|&part| self.part_owners(part))
                    .collect();
                files.sort_unstable();
                files.dedup();
                files
            }
        }
    }

    /// How `target` is shown in a diagnostic: a directory's or a file's
    /// module path, a node's dotted path, `fileroot` or `fileroot.caller`.
    fn show_target(&self, target: Target) -> String {
        let module = |path: &str| path.trim_end_matches(".gcsl").replace('/', ".");
        match target {
            Target::Fileroot(None) => Fileroot::MODULE.to_owned(),
            Target::Fileroot(Some(member)) => format!("{}.{}", Fileroot::MODULE, member.name()),
            Target::Dir(dir) => self
                .dirs
                .get(dir as usize)
                .map_or_else(String::new, |d| module(&d.path)),
            Target::File(file) => self
                .files
                .get(file as usize)
                .map_or_else(String::new, |f| module(&f.path)),
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.global.get(node as usize))
                .map_or_else(String::new, |&merged| self.show(merged)),
            Target::Node(node) => self.show(node),
            Target::View(view) => self.show_view(view),
        }
    }

    /// The name of the top-level function `function`, or its path if it is
    /// a `def` named by a dotted path.
    fn show_function(&self, function: Function) -> String {
        let Some(file) = self.files.get(function.file as usize) else {
            return String::new();
        };
        let Some(top) = file.mentions.functions.get(function.index as usize) else {
            return String::new();
        };
        match (top.name, file.ast.item(top.item).kind) {
            (Some(name), _) => self.names.get(name).to_owned(),
            (None, ItemKind::Field { path, .. }) => {
                let path: Vec<Name> = file
                    .ast
                    .path(path)
                    .iter()
                    .filter_map(PathSegment::name)
                    .collect();
                names::show_path(&self.names, &path)
            }
            // A definition made with a def-type, as it is written.
            (None, ItemKind::DefTypeDef { def, .. }) => {
                let def = file.ast.def_type(def);
                let path: Vec<Name> = def
                    .key
                    .into_iter()
                    .chain(file.ast.path(def.path).iter().filter_map(PathSegment::name))
                    .collect();
                names::show_path(&self.names, &path)
            }
            // A definition's meta, as `defmeta` and its def-type and path.
            (None, ItemKind::DefMeta { def, .. }) => {
                let path: Vec<Name> = file
                    .ast
                    .def_type(def)
                    .meta
                    .map(|(key, leaf)| vec![key, leaf])
                    .unwrap_or_default();
                names::show_path(&self.names, &path)
            }
            (None, _) => String::new(),
        }
    }

    /// The dotted path in its file's view of the view node `view`.
    fn show_view(&self, view: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(view);
        while let Some(index) = at
            && let Some(entry) = self.views.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.names, &path)
    }

    /// The dotted path of the merged node `node`, a definition's as it is
    /// written (see [`crate::names`], "Namespace keys").
    fn show(&self, node: u32) -> String {
        let mut path = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at
            && let Some(entry) = self.nodes.get(index as usize)
        {
            path.push(entry.name);
            at = entry.parent;
        }
        path.reverse();
        names::show_path(&self.names, &path)
    }
}

/// The scope a file's references are resolved in: its view, the merged
/// trees of its directory chain and what it imports (see the module
/// documentation, "The view of a file"), and the `def`s of its chain,
/// numbered as [`crate::mentions::Resolve`] asks. The file's own field
/// nodes come first, by their own numbers, then the nodes of its view with
/// a leaf of the file under them ([`Mentions::views`]), then every merged
/// node at its index in [`Scan::nodes`] after those, then every target an
/// import or the view reaches (a [`Target::View`] among them); its own
/// functions come first, then every other one at its index in
/// [`Scan::functions`] after them.
#[derive(Debug)]
pub struct FileScope<'a> {
    /// The workspace.
    scan: &'a Scan,
    /// The file's directory.
    dir: u32,
    /// The file's own functions.
    own: &'a [mentions::TopLevel],
    /// The file's own tree of top-level fields.
    tree: &'a FieldTree,
    /// For each of the file's own nodes, the merged node of its path.
    global: &'a [u32],
    /// The file's own node of each merged node it has one of.
    local: HashMap<u32, u32>,
    /// What each name the file imports names; of a name several sources
    /// have, the view's node says.
    aliases: HashMap<Name, Target>,
    /// The names an import binds that is shadowable, the implicit `import *
    /// from prelude`'s among them ([`FileScope::yields`]).
    shadowable: Vec<Name>,
    /// The names of the imports a session's input inherits from earlier
    /// inputs ([`FileScope::inherited`]).
    inherited: Vec<Name>,
    /// The file's view.
    view: &'a View,
}

impl<'a> FileScope<'a> {
    /// The scope of `file` of `scan`.
    #[must_use]
    pub fn new(scan: &'a Scan, file: &'a File) -> Self {
        let local = (0u32..)
            .zip(&file.global)
            .map(|(own, &global)| (global, own))
            .collect();
        Self {
            scan,
            dir: file.dir,
            own: &file.mentions.functions,
            tree: &file.mentions.tree,
            global: &file.global,
            local,
            aliases: file.aliases.iter().map(|a| (a.name, a.target)).collect(),
            shadowable: file
                .aliases
                .iter()
                .filter(|a| file.marked(a) || (!a.whole && scan.target_shadowable(a.target)))
                .map(|a| a.name)
                .collect(),
            inherited: file
                .aliases
                .iter()
                .filter(|a| a.inherited)
                .map(|a| a.name)
                .collect(),
            view: &file.view,
        }
    }

    /// Whether the root `name` is one an import a session's input inherits
    /// from an earlier input binds, which a top-level `let` of the input
    /// shadows from its point on, as it would an earlier input's variable,
    /// and clashes with nothing (see [`inherit_imports`]).
    #[must_use]
    pub fn inherited(&self, name: Name) -> bool {
        self.inherited.contains(&name)
    }

    /// Whether what the root `name` means in the file's view is shadowable
    /// (see the module documentation, "Shadowable declarations"): the
    /// prelude's names that nothing else has among them. A binding the file
    /// declares of its own of the name, a top-level `let` or `fn`, shadows
    /// it, as an earlier session input's variable does, and the clash family
    /// ignores it.
    #[must_use]
    pub fn yields(&self, name: Name) -> bool {
        if let Some(&merged) = self.view.merged.get(&(ROOT, name)) {
            return match merged {
                Merged::One(part) => self.scan.part_shadowable(part),
                Merged::View(view) => self.scan.target_shadowable(Target::View(view)),
            };
        }
        if let Some(&node) = self
            .scan
            .chain(self.dir)
            .find_map(|dir| self.scan.by_name.get(&(dir, ROOT, name)))
        {
            return self.scan.target_shadowable(Target::Node(node));
        }
        self.shadowable.contains(&name)
    }

    /// Where the numbers of the merged nodes start: after the file's own
    /// nodes and its view's own nodes.
    fn merged_start(&self) -> u32 {
        count(self.global.len()).saturating_add(self.view.own)
    }

    /// Where the numbers of targets reached through imports or the view
    /// start: after the merged nodes.
    fn targets_start(&self) -> u32 {
        self.merged_start()
            .saturating_add(count(self.scan.nodes.len()))
    }

    /// What the scope's number `node` is when it is reached through an
    /// import or a node of the view without a leaf of the file under it,
    /// `None` for the file's own nodes, its view's own nodes and the merged
    /// nodes of its directory chain ([`FileScope::merged`]). A reference
    /// through an import resolves to one of these, and so does a mention of
    /// it in [`Mentions::foreign`]; a [`Target::Node`] is a cell like any
    /// merged node, read by its slot.
    #[must_use]
    pub fn target(&self, node: u32) -> Option<Target> {
        self.scan.target_of(node.checked_sub(self.targets_start())?)
    }

    /// The import of the name `name`, if the file imports it.
    #[must_use]
    pub fn alias(&self, name: Name) -> Option<Target> {
        self.aliases.get(&name).copied()
    }

    /// Whether a reference to the root `name` reads through the file's
    /// view rather than a binding: when the root is imported, or several
    /// sources have it. A root of the file's own or of one directory of its
    /// chain alone is read through its binding, as before imports.
    #[must_use]
    pub fn reads_through(&self, name: Name) -> bool {
        // Only an imported name or a root of the view can: two lookups
        // settle most names, without the walk of the directory chain.
        if !self.aliases.contains_key(&name) && !self.view.merged.contains_key(&(ROOT, name)) {
            return false;
        }
        self.root(name)
            .is_some_and(|node| node >= count(self.global.len()) && self.merged(node).is_none())
    }

    /// The index in [`Scan::views`] of the scope's number `node`, if it is
    /// a node of the view with a leaf of the file under it.
    #[must_use]
    pub fn own_view(&self, node: u32) -> Option<u32> {
        let at = node.checked_sub(count(self.global.len()))?;
        (at < self.view.own).then(|| self.view.start.saturating_add(at))
    }

    /// The scope's number of `target`.
    fn target_number(&self, target: Target) -> u32 {
        if let Target::View(view) = target
            && let Some(at) = view.checked_sub(self.view.start)
            && at < self.view.own
        {
            return count(self.global.len()).saturating_add(at);
        }
        self.targets_start()
            .saturating_add(self.scan.target_number(target))
    }

    /// The scope's number of the merged node `node`.
    fn number(&self, node: u32) -> u32 {
        self.local
            .get(&node)
            .copied()
            .unwrap_or_else(|| self.merged_start().saturating_add(node))
    }

    /// The scope's number of what the view makes of a name.
    fn merged_number(&self, merged: Merged) -> u32 {
        match merged {
            Merged::View(view) => self.target_number(Target::View(view)),
            Merged::One(ViewPart {
                target: Target::Node(node),
                chain: true,
                ..
            }) => self.number(node),
            Merged::One(part) => self.target_number(part.target),
        }
    }

    /// The merged node of the scope's number `node`, if it is one of the
    /// file's own nodes or a merged node of its directory chain.
    #[must_use]
    pub fn merged(&self, node: u32) -> Option<u32> {
        if let Some(own) = self.global.get(node as usize) {
            return Some(*own);
        }
        let other = node.checked_sub(self.merged_start())?;
        (node < self.targets_start()).then_some(other)
    }

    /// The children of the view node `node` (a scope's number), each with
    /// its name and its scope's number, in the view's order: by first
    /// definition, its sources root directory first.
    #[must_use]
    pub fn view_children(&self, node: u32) -> Vec<(Name, u32)> {
        let view = match self.own_view(node) {
            Some(view) => view,
            None => match self.target(node) {
                Some(Target::View(view)) => view,
                _ => return Vec::new(),
            },
        };
        self.scan
            .view_names(view)
            .into_iter()
            .filter_map(|name| Some((name, self.child(node, name)?)))
            .collect()
    }

    /// The function of [`Scan::functions`] of the scope's number
    /// `function`, if it is another file's.
    #[must_use]
    pub fn foreign_function(&self, function: u32) -> Option<u32> {
        function.checked_sub(count(self.own.len()))
    }

    /// Whether the scope's number `node` is a leaf of the view: the file's
    /// own leaf, one of a directory of its chain, or one an import reaches.
    #[must_use]
    pub fn is_leaf(&self, node: u32) -> bool {
        if self.own_view(node).is_some() {
            return false;
        }
        let merged = self.merged(node).or(match self.target(node) {
            Some(Target::Node(merged)) => Some(merged),
            _ => None,
        });
        merged
            .and_then(|merged| self.scan.nodes.get(merged as usize))
            .is_some_and(|entry| entry.leaf.is_some())
    }

    /// The children of the scope's number `node` in the view, each with its
    /// name and its scope's number, in the view's order: none for a leaf.
    #[must_use]
    pub fn children(&self, node: u32) -> Vec<(Name, u32)> {
        if self.own_view(node).is_some() || matches!(self.target(node), Some(Target::View(_))) {
            return self.view_children(node);
        }
        let names = match (self.merged(node), self.target(node)) {
            (Some(merged), _) => self.scan.child_names(Target::Node(merged)),
            (None, Some(target)) => self.scan.child_names(target),
            (None, None) => Vec::new(),
        };
        names
            .into_iter()
            .filter_map(|name| Some((name, self.child(node, name)?)))
            .collect()
    }

    /// The file whose leaf the scope's number `node` is, with the file's own
    /// node of it, if it is a leaf of the view.
    fn leaf_owner(&self, node: u32) -> Option<(u32, u32)> {
        let merged = self.merged(node).or(match self.target(node) {
            Some(Target::Node(merged)) => Some(merged),
            _ => None,
        })?;
        let owner = self.scan.nodes.get(merged as usize)?.leaf?;
        let own = self
            .scan
            .files
            .get(owner as usize)?
            .global
            .iter()
            .position(|&g| g == merged)?;
        Some((owner, count(own)))
    }
}

impl Resolve for FileScope<'_> {
    fn root(&self, name: Name) -> Option<u32> {
        if let Some(&merged) = self.view.merged.get(&(ROOT, name)) {
            return Some(self.merged_number(merged));
        }
        self.scan
            .chain(self.dir)
            .find_map(|dir| self.scan.by_name.get(&(dir, ROOT, name)))
            .map(|&node| self.number(node))
            .or_else(|| Some(self.target_number(self.alias(name)?)))
    }

    fn child(&self, node: u32, name: Name) -> Option<u32> {
        let view = self.own_view(node).or(match self.target(node) {
            Some(Target::View(view)) => Some(view),
            _ => None,
        });
        if let Some(view) = view {
            return Some(self.merged_number(self.scan.view_child(view, name)?));
        }
        if let Some(target) = self.target(node) {
            let child = self.scan.target_child(target, name)?;
            return Some(self.target_number(child));
        }
        let merged = self.merged(node)?;
        let dir = self.scan.nodes.get(merged as usize)?.dir;
        let child = *self.scan.by_name.get(&(dir, merged, name))?;
        Some(self.number(child))
    }

    fn function(&self, name: Name) -> Option<u32> {
        if let Some(own) = self.own.iter().position(|f| f.name == Some(name)) {
            return Some(count(own));
        }
        let def = self
            .scan
            .chain(self.dir)
            .find_map(|dir| self.scan.defs.get(&(dir, name)))?;
        Some(count(self.own.len()).saturating_add(*def))
    }

    fn dotted_defs_under(&self, node: u32, each: &mut dyn FnMut(u32)) {
        // The file's own node, or the file's own node of its view's node:
        // another file's functions are no mentions of this file's.
        let own = match self.own_view(node) {
            Some(view) => self.scan.views.get(view as usize).and_then(|v| v.own),
            None => (node < count(self.global.len())).then_some(node),
        };
        if let Some(own) = own {
            self.tree.dotted_defs_under(own, each);
        }
    }

    fn own(&self) -> (u32, u32) {
        (self.merged_start(), count(self.own.len()))
    }
}

/// A count as a `u32`, saturating; a workspace's counts are bounded by its
/// sources' lengths.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The directory part of `path`, empty for a file of the root.
fn parent_path(path: &str) -> &str {
    path.rfind('/')
        .map_or("", |at| path.get(..at).unwrap_or(""))
}

/// Scans the workspace whose files are `sources`, named by their paths
/// relative to the root, beside the standard library: parses every file,
/// merges each directory's trees and each file's view, resolves every
/// file's references against its view, and finds the order and each file's
/// schedule (see the module documentation). The standard files it loads,
/// the prelude and each that a file imports, are files `0..`
/// [`Scan::standard`], and the workspace's follow them in path order.
///
/// # Errors
///
/// Every lexical and syntax error of every file, the files scanned as their
/// parser recovered them (see [Syntax errors](#syntax-errors)), and every
/// error of the scan itself, which it recovered from (see [Scan
/// errors](#scan-errors)), but those of files that may see a file with
/// syntax errors: every file or directory of the root named like a module of
/// the standard library, every conflict between files' declarations in a
/// directory, every import's error, every conflict of every file's view and
/// view of types, every cycle of types and every cycle of files that need
/// each other; in path then source order. Each names its file in
/// [`Diagnostic::input`], its index among the files.
pub fn scan(sources: &[SourceFile]) -> Result<Scan, Vec<Diagnostic>> {
    scan_with(sources, &[])
}

/// [`scan`], told the directories left out of the workspace as workspaces
/// of their own, each by its path relative to the root without a trailing
/// `/`: an import of one is "no module" with a note saying so.
///
/// # Errors
///
/// As [`scan`]; among the imports' errors, every import that names nothing
/// or clashes with a name of its file or directory chain, and with the
/// cycles, every reference through an import that reaches back into its
/// own file.
pub fn scan_with(sources: &[SourceFile], nested: &[String]) -> Result<Scan, Vec<Diagnostic>> {
    let scan = scan_into(
        (sources, Placing::Workspace(nested)),
        Library::NEEDED,
        &mut Names::new(),
        &mut Vec::new(),
    )?;
    if scan.syntax_errors.is_empty() && scan.recovered.is_empty() {
        Ok(scan)
    } else {
        Err(scan.errors())
    }
}

/// Where the files a scan is given are placed (see the module
/// documentation, "The standard library").
#[derive(Clone, Copy, Debug)]
enum Placing<'a> {
    /// The files of a workspace, by their paths relative to its root, with
    /// the directories left out as workspaces of their own.
    Workspace(&'a [String]),
    /// One file outside any workspace, a lone file or a session's input, in
    /// the root directory by the last part of its name, with the imports of
    /// the session's earlier inputs it inherits (see [`inherit_imports`]),
    /// none for a lone file, and whether it is a session's input, which
    /// cannot be marked `#![no_prelude]` ([`no_prelude_in_input`]).
    Lone(&'a [Alias], bool),
}

/// [`scan_with`] of `sources` placed as `placing` says, beside `library`,
/// interning into `names`, which the scan holds once it succeeds
/// ([`Scan::names`]) and which are given back to `names` if it fails. It
/// fails only for a session's input with a syntax error or an error of the
/// scan, which is not checked; any other scan recovers from its errors (see
/// [Scan errors](#scan-errors)). The
/// indices in the library's table of the standard files it loads, which
/// are files `0..` [`Scan::standard`], are left in `loaded` whether it
/// succeeds or fails, so that a caller can render a diagnostic of any file
/// against its source (see [`check_with_standard`]). Its types are its own
/// ([`scan_into_with`]).
fn scan_into(
    placed: (&[SourceFile], Placing),
    library: Library,
    names: &mut Names,
    loaded: &mut Vec<usize>,
) -> Result<Scan, Vec<Diagnostic>> {
    scan_into_with(
        placed,
        library,
        (names, loaded),
        (&mut TypeStore::new(), &[]),
        (&[], None),
    )
}

/// [`scan_into`], the files' types defined after those of `store`, which
/// the scan holds once it succeeds ([`Scan::type_store`]) and which is given
/// back to `store` if it fails, and with `inherited`, the types a session's
/// input inherits from the earlier inputs, each with its declaration's
/// number in `store` (see [Types](#types)); and for a session's input, with
/// `def_types`, the def-types it inherits from the earlier inputs, by path
/// with their namespaces' keys, and its number among the session's inputs
/// (see [Def-types](#def-types)).
fn scan_into_with(
    (sources, placing): (&[SourceFile], Placing),
    library: Library,
    (names, loaded): (&mut Names, &mut Vec<usize>),
    (store, inherited): (&mut TypeStore, &[(Name, u32)]),
    def_types: (&[(Vec<Name>, Name)], Option<u32>),
) -> Result<Scan, Vec<Diagnostic>> {
    let mut scan = Scan {
        files: Vec::with_capacity(sources.len() + library.table.len()),
        dirs: Vec::new(),
        nodes: Vec::new(),
        functions: Vec::new(),
        views: Vec::new(),
        needs: Vec::new(),
        order: Vec::new(),
        names: mem::take(names),
        by_name: HashMap::new(),
        roots: Vec::new(),
        defs: HashMap::new(),
        rank: Vec::new(),
        part_base: Vec::new(),
        values: Vec::new(),
        standard: 0,
        builtins_dir: 0,
        fileroot_names: [None; 4],
        lone: matches!(placing, Placing::Lone(..)),
        relocation: None,
        dir_texts: Vec::new(),
        file_dirs: Vec::new(),
        types: Vec::new(),
        dir_types: Vec::new(),
        type_store: mem::take(store),
        type_base: 0,
        inherited_types: inherited.to_vec(),
        inherited_def_types: def_types.0.to_vec(),
        input_number: def_types.1,
        syntax_errors: Vec::new(),
        recovered: Vec::new(),
        conflict_items: Vec::new(),
        conflicted: HashMap::new(),
        poisoned_nodes: HashSet::new(),
        poisoned_types: Vec::new(),
        cyclic: Vec::new(),
    };
    match scan_files(&mut scan, (sources, placing), library, loaded) {
        Ok(()) => Ok(scan),
        Err(errors) => {
            *names = mem::take(&mut scan.names);
            *store = mem::take(&mut scan.type_store);
            Err(errors)
        }
    }
}

/// Fills `scan`, empty but for its names, with the standard files of
/// `library` it loads, each in a directory of its own, and `sources`
/// placed as `placing` says (see [`scan_with`]), leaving in `loaded` the
/// indices in the library's table of the standard files it loads.
fn scan_files(
    scan: &mut Scan,
    (sources, placing): (&[SourceFile], Placing),
    library: Library,
    loaded: &mut Vec<usize>,
) -> Result<(), Vec<Diagnostic>> {
    let mut sorted: Vec<&SourceFile> = sources.iter().collect();
    sorted.sort_by(|a, b| a.name().as_bytes().cmp(b.name().as_bytes()));
    let (standard, parsed, syntax_errors) = parse_files(scan, &sorted, library, loaded);
    // A session's input with an error is not checked: nothing of it is
    // committed, whatever its parts hold.
    if !syntax_errors.is_empty() && matches!(placing, Placing::Lone(_, true)) {
        return Err(syntax_errors);
    }
    scan.syntax_errors = syntax_errors;
    scan.standard = count(standard.len());
    scan.fileroot_names = Fileroot::ALL.map(|member| scan.names.lookup(member.name()));
    let (dirs, user_dirs) = place_dirs(scan, &sorted, placing);
    // Each standard file is a directory of its own, with no directory
    // above: a scope of its own, apart from the workspace's tree and from
    // every other standard file.
    for (&at, (ast, spelled)) in loaded.iter().zip(standard) {
        let stem = library.table.get(at).map_or("", |&(stem, _)| stem);
        let dir = count(scan.dirs.len());
        scan.dirs.push(Dir {
            path: STANDARD_DIR.to_owned(),
            parent: None,
        });
        let path = format!("{STANDARD_DIR}/{stem}.gcsl");
        scan.files.push(File::new(&path, dir, ast, spelled));
    }
    scan.builtins_dir = count(scan.dirs.len());
    scan.dirs.push(Dir {
        path: BUILTINS.to_owned(),
        parent: None,
    });
    for ((source, &dir), (ast, spelled)) in sorted.iter().zip(&user_dirs).zip(parsed) {
        let path = match placing {
            Placing::Workspace(_) => source.name(),
            Placing::Lone(..) => source.name().rsplit('/').next().unwrap_or(""),
        };
        scan.files.push(File::new(path, dir, ast, spelled));
    }
    // Every stage of the scan reports its errors and recovers from them,
    // so that the next stages, and the check, see a workspace they can
    // work with (see [Scan errors](#scan-errors)).
    let mut errors = Vec::new();
    match placing {
        Placing::Workspace(_) => reserved(scan, library, &mut errors),
        Placing::Lone(_, true) => no_prelude_in_input(scan, &mut errors),
        Placing::Lone(_, false) => {}
    }
    // A definition made with a def-type is a leaf of its def-type's
    // namespace, which is known only once the files' views are: the trees
    // are made again with the definitions under their namespaces' keys, if
    // any definition has one (see [Def-types](#def-types)).
    let mark = scan.type_store.mark();
    for pass in 0..2 {
        let mut found = Vec::new();
        scan_trees(scan, placing, &dirs, &mut found);
        if !name_namespaces(scan) || pass > 0 {
            errors.append(&mut found);
            break;
        }
        // A meta declared twice in a directory, or with no definition there,
        // is its own error, before the trees with the metas in them would
        // report it as a conflict (see [Def-types](#def-types)); it is
        // dropped. The trees made again find again what these found.
        let (unpaired, dropped) = unpaired_metas(scan);
        errors.extend(unpaired);
        unscan(scan, mark, &dropped);
    }
    mark_unpaired(scan);
    type_views(scan, &mut errors);
    define_types(scan, &mut errors);
    value_cells(scan);
    resolve(scan);
    let mentioned = find_needs(scan, &mut errors);
    report_cycles(scan, &mentioned, &mut errors);
    scan.order = order(&scan.needs);
    scan.rank = scan.ranks();
    schedule(scan);
    let errors = scan.stage_errors(errors);
    scan.recovered = merge_files(errors, Vec::new());
    // A session's input with an error is not checked, as for a syntax
    // error.
    if !scan.recovered.is_empty() && matches!(placing, Placing::Lone(_, true)) {
        return Err(mem::take(&mut scan.recovered));
    }
    Ok(())
}

/// Merges the trees of `scan`'s files, placed as `placing` says, into their
/// directories' trees, resolves their imports, the directories being
/// `dirs`, by path, and makes their views: the part of [`scan_files`] that
/// a def-type's namespace needs done before it is known, and done again
/// once the definitions are under their namespaces' keys (see
/// [Def-types](#def-types)). Adds to `errors` the conflicts of the
/// directories' trees, the types declared twice, the imports' errors and
/// the conflicts of the views, recovering from each (see [Scan
/// errors](#scan-errors)).
fn scan_trees(
    scan: &mut Scan,
    placing: Placing,
    dirs: &HashMap<String, u32>,
    errors: &mut Vec<Diagnostic>,
) {
    shadow(scan);
    merge_trees(scan, errors);
    declare_types(scan, errors);
    shadowable_nodes(scan);
    let mut base = 0u32;
    scan.part_base = scan
        .files
        .iter()
        .map(|file| {
            let start = base;
            base = base.saturating_add(count(file.global.len()));
            start
        })
        .collect();
    let (nested, inherited) = match placing {
        Placing::Workspace(nested) => (nested, &[][..]),
        Placing::Lone(inherited, _) => (&[][..], inherited),
    };
    imports(scan, dirs, (nested, inherited), errors);
    views(scan, errors);
}

/// Merges the trees of `scan`'s files into their directories' trees,
/// adding to `errors` each conflict between two files of a directory, and
/// recovering from them (see [Scan errors](#scan-errors)): the later
/// declarations of each conflict are dropped from their files
/// ([`File::drop_items`]) and the trees merged again without them, and the
/// node each meets the tree at is conflicted ([`Scan::conflicted`]). With
/// no conflict, the trees are merged once.
fn merge_trees(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    loop {
        scan.nodes.clear();
        scan.by_name.clear();
        scan.functions.clear();
        scan.defs.clear();
        scan.roots = vec![Vec::new(); scan.dirs.len()];
        builtin_nodes(scan);
        let mut dropped: Vec<(u32, usize)> = Vec::new();
        for file in 0..count(scan.files.len()) {
            merge(scan, file, errors, &mut dropped);
        }
        dropped.sort_unstable();
        dropped.dedup();
        // Each merge again drops what is left of a conflict; one that drops
        // nothing new is the last.
        dropped.retain(|item| !scan.conflict_items.contains(item));
        if dropped.is_empty() {
            break;
        }
        for (index, file) in (0u32..).zip(&mut scan.files) {
            let mine: Vec<usize> = dropped
                .iter()
                .filter(|&&(at, _)| at == index)
                .map(|&(_, position)| position)
                .collect();
            if !mine.is_empty() {
                file.drop_items(&mine);
            }
        }
        scan.conflict_items.extend(dropped);
    }
    conflicted_nodes(scan);
}

/// Finds the conflicted nodes of `scan` ([`Scan::conflicted`]) from its
/// declarations dropped as later definitions of a field: the node of each
/// one's directory's tree its path meets last, the longest prefix of it the
/// tree has; and marks each, and every node above it, poisoned.
fn conflicted_nodes(scan: &mut Scan) {
    scan.conflicted.clear();
    scan.poisoned_nodes.clear();
    let mut items = scan.conflict_items.clone();
    items.sort_unstable();
    for (index, position) in items {
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        let Some(&item) = file.ast.items(file.ast.root()).get(position) else {
            continue;
        };
        let Some((_, names, _)) =
            mentions::declarations(&file.ast).find(|&(at, _, _)| at == position)
        else {
            continue;
        };
        let mut parent = ROOT;
        let mut met = None;
        for name in names {
            let Some(&node) = scan.by_name.get(&(file.dir, parent, name)) else {
                break;
            };
            met = Some(node);
            parent = node;
        }
        let Some(node) = met else {
            continue;
        };
        scan.conflicted.entry(node).or_default().push((index, item));
        let mut at = Some(node);
        while let Some(up) = at {
            scan.poisoned_nodes.insert(up);
            at = scan.nodes.get(up as usize).and_then(|n| n.parent);
        }
    }
}

/// Forgets what [`scan_trees`] made of `scan`'s files, and their files'
/// declarations, so that it can make them again from the files' trees,
/// which have changed: every file's declarations found again, the merged
/// nodes, functions, views and types gone, the type store put back at
/// `mark`, where it was before the types were declared. Of the declarations
/// the scan dropped, only `dropped` are, by file and position: the trees
/// made again find their conflicts again.
fn unscan(scan: &mut Scan, mark: TypeStoreMark, dropped: &[(u32, usize)]) {
    let files = mem::take(&mut scan.files);
    scan.files = (0u32..)
        .zip(files)
        .map(|(index, file)| {
            let mut file = File::new(&file.path, file.dir, file.ast, file.spelled);
            let mine: Vec<usize> = dropped
                .iter()
                .filter(|&&(at, _)| at == index)
                .map(|&(_, position)| position)
                .collect();
            if !mine.is_empty() {
                file.drop_items(&mine);
            }
            file
        })
        .collect();
    scan.nodes.clear();
    scan.by_name.clear();
    scan.functions.clear();
    scan.views.clear();
    scan.defs.clear();
    scan.part_base.clear();
    scan.types.clear();
    scan.dir_types.clear();
    scan.type_store.truncate(mark);
    scan.type_base = 0;
    scan.conflict_items.clear();
    scan.conflicted.clear();
    scan.poisoned_nodes.clear();
    scan.poisoned_types.clear();
}

/// A def-type definition, reference or meta whose def-type
/// [`name_namespaces`] has found: its file, its use, the text of its
/// def-type's key, and for a definition or a meta, the text of its meta key
/// and its path as one name.
type NamedUse = (u32, DefTypeId, String, Option<(String, String)>);

/// Names the namespace of each def-type definition and reference of every
/// file of `scan` whose namespace is not known yet: what its `DT` names in
/// the file's view, if that is a `def` marked `#[def_type]`, or, for a
/// session's input, a def-type of the earlier inputs by its path, gives it
/// the def-type's key ([`Ast::set_def_type_key`]), which the file is then
/// taken to spell (see [Def-types](#def-types)); and a definition or a meta
/// its leaf in the namespace of the metas of the def-type's definitions in
/// its file's directory, the meta key and its path as one name (see
/// [`crate::names`], "Namespace keys"), which the file is taken to spell
/// too. Returns whether a definition or a meta was given one, which changes
/// the trees.
fn name_namespaces(scan: &mut Scan) -> bool {
    let mut found: Vec<NamedUse> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut unknown = file
            .ast
            .def_types()
            .filter(|(_, def)| def.key.is_none())
            .peekable();
        if unknown.peek().is_none() {
            continue;
        }
        let scope = FileScope::new(scan, file);
        for (id, def) in unknown {
            let names = file.ast.def_type_names(id);
            let Some(key) = def_type_key(scan, index, &scope, &names) else {
                continue;
            };
            // A definition's and a meta's leaf in the namespace of the metas
            // of the def-type's definitions in its directory.
            let meta = match def.role {
                DefTypeRole::Reference => None,
                DefTypeRole::Definition | DefTypeRole::Meta => {
                    let path: Vec<&str> = file
                        .ast
                        .path(def.path)
                        .iter()
                        .filter_map(PathSegment::name)
                        .map(|name| scan.names.get(name))
                        .collect();
                    names::meta_key(&key, file.dir).map(|meta_key| (meta_key, path.join(".")))
                }
            };
            found.push((index, id, key, meta));
        }
    }
    let mut leaves = false;
    for (index, id, text, meta) in found {
        let key = scan.names.intern(&text);
        let meta =
            meta.map(|(meta_key, leaf)| (scan.names.intern(&meta_key), scan.names.intern(&leaf)));
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.set_def_type_key(id, key, meta);
            for name in std::iter::once(key).chain(meta.into_iter().flat_map(|(a, b)| [a, b])) {
                if let Err(at) = file.spelled.binary_search(&name) {
                    file.spelled.insert(at, name);
                }
            }
        }
        leaves |= meta.is_some();
    }
    leaves
}

/// A definition or a meta of a file of a scan by its leaf in the namespace
/// of the metas of its def-type's definitions in its directory (see
/// [Def-types](#def-types)): that leaf, its file, its use of its def-type
/// and its item's position among the file's items.
type MetaLeaf = ((Name, Name), u32, DefTypeId, usize);

/// The definitions and the metas of `scan`'s files whose def-types are
/// known, each by its leaf in the namespace of the metas of its def-type's
/// definitions in its directory, in path order of their files and then in
/// source order: so a definition and a meta of one path, def-type and
/// directory have one leaf.
fn meta_leaves(scan: &Scan) -> (Vec<MetaLeaf>, Vec<MetaLeaf>) {
    let (mut definitions, mut metas) = (Vec::new(), Vec::new());
    for (index, file) in (0u32..).zip(&scan.files) {
        let ast = &file.ast;
        for (position, &item) in ast.items(ast.root()).iter().enumerate() {
            let (ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. }) =
                ast.item(item).kind
            else {
                continue;
            };
            let Some(leaf) = ast.def_type(def).meta else {
                continue;
            };
            // A meta the scan dropped is none (see [Scan
            // errors](#scan-errors)).
            if file.dropped.binary_search(&position).is_ok() {
                continue;
            }
            match ast.item(item).kind {
                ItemKind::DefMeta { .. } => metas.push((leaf, index, def, position)),
                _ => definitions.push((leaf, index, def, position)),
            }
        }
    }
    (definitions, metas)
}

/// The errors of the metas of `scan`'s files that are not each their
/// directory's one meta of a definition of it (see [Def-types](#def-types)):
/// a meta whose directory declares another of its path and def-type before
/// it, "`defmeta package x` is declared twice in this directory", at it,
/// labelled "first declared here" at the first; and one whose directory
/// declares no definition of its path and def-type, "`defmeta package x`
/// has no definition `x` in its directory", at it. Each at the meta's
/// `defmeta`, def-type and path, in path order of their files, then in
/// source order. With them, each such meta, by its file and its position
/// among the file's items, which the scan drops (see [Scan
/// errors](#scan-errors)).
fn unpaired_metas(scan: &Scan) -> (Vec<Diagnostic>, Vec<(u32, usize)>) {
    let (definitions, metas) = meta_leaves(scan);
    let defined: HashSet<(Name, Name)> = definitions.iter().map(|&(leaf, ..)| leaf).collect();
    let mut first: HashMap<(Name, Name), (u32, Span)> = HashMap::new();
    let mut errors = Vec::new();
    let mut dropped = Vec::new();
    for &(leaf, index, def, position) in &metas {
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        let ast = &file.ast;
        let (def_type, path) = {
            let shown = ast.def_type_names(def);
            let shown: Vec<&str> = shown.iter().map(|&name| scan.names.get(name)).collect();
            let path: Vec<&str> = ast
                .path(ast.def_type(def).path)
                .iter()
                .filter_map(PathSegment::name)
                .map(|name| scan.names.get(name))
                .collect();
            (shown.join("."), path.join("."))
        };
        // From its `defmeta` through its path.
        let at = ast.def_type(def).span;
        if let Some(&(first_file, first_at)) = first.get(&leaf) {
            errors.push(
                Diagnostic::error(
                    format!("`defmeta {def_type} {path}` is declared twice in this directory"),
                    at,
                )
                .in_input(index)
                .with_context_in(first_file, first_at, "first declared here"),
            );
            dropped.push((index, position));
            continue;
        }
        first.insert(leaf, (index, at));
        if !defined.contains(&leaf) {
            errors.push(
                Diagnostic::error(
                    format!(
                        "`defmeta {def_type} {path}` has no definition `{path}` in its directory"
                    ),
                    at,
                )
                .in_input(index),
            );
            dropped.push((index, position));
        }
    }
    (errors, dropped)
}

/// Records in each file of `scan` its definitions whose directory declares
/// no meta of their path and def-type ([`File::unpaired`]), which the
/// checker reports if their def-type declares a meta (see the checker's
/// "Metas").
fn mark_unpaired(scan: &mut Scan) {
    let (definitions, metas) = meta_leaves(scan);
    let declared: HashSet<(Name, Name)> = metas.iter().map(|&(leaf, ..)| leaf).collect();
    for (leaf, index, def, _) in definitions {
        if !declared.contains(&leaf)
            && let Some(file) = scan.files.get_mut(index as usize)
        {
            file.unpaired.push(def);
        }
    }
}

/// The text of the key of the namespace of the def-type `names`, a path,
/// names in file `index`'s view, `scope`: the leaf of its path there, if it
/// is a `def` marked `#[def_type]`, keyed by its path, its file's number,
/// or for a session's input the input's, and its item (see
/// [`crate::names`], "Namespace keys"); or, for a session's input whose
/// view has nothing of its first name, the def-type of that path of the
/// earlier inputs.
fn def_type_key(scan: &Scan, index: u32, scope: &FileScope, names: &[Name]) -> Option<String> {
    let (&first, rest) = names.split_first()?;
    let Some(mut node) = scope.root(first) else {
        return scan
            .inherited_def_types
            .iter()
            .find(|(path, _)| path.as_slice() == names)
            .map(|&(_, key)| scan.names.get(key).to_owned());
    };
    for &name in rest {
        node = scope.child(node, name)?;
    }
    let (owner, own) = scope.leaf_owner(node)?;
    let file = scan.files.get(owner as usize)?;
    let leaf = file.mentions.tree.nodes.get(own as usize)?.leaf?;
    let ItemKind::Field {
        path,
        function: true,
        ..
    } = file.ast.item(leaf.item).kind
    else {
        return None;
    };
    if !file.ast.is_def_type(leaf.item) {
        return None;
    }
    let shown: Vec<&str> = file
        .ast
        .path(path)
        .iter()
        .filter_map(PathSegment::name)
        .map(|name| scan.names.get(name))
        .collect();
    let number = match scan.input_number {
        Some(input) if owner == index => input,
        _ => owner,
    };
    let item = u32::try_from(leaf.item.index()).unwrap_or(u32::MAX);
    Some(names::namespace_key(&shown.join("."), number, item))
}

/// A file parsed: its tree and the names it spells, with its lexical and
/// syntax errors, in source order (see [`parse`]).
type Parsed = (Ast, Vec<Name>, Vec<Diagnostic>);

/// Files parsed, each its tree and the names it spells.
type Trees = Vec<(Ast, Vec<Name>)>;

/// Parses the files `sorted`, in path order, and the standard files of
/// `library` that the scan loads, interning into the scan's names, and
/// leaves in `loaded` the indices in the library's table of those it loads,
/// in the table's order: every one if the library says so (a session's),
/// else the prelude and each standard file that a file, or a standard file
/// loaded, imports by its bare stem, in any form ([`standard_imports`]).
/// The prelude, and what it imports, is parsed first, as it always was (a
/// session's every standard file, in the table's order), then the files,
/// then the standard files they need. Returns the trees of the standard
/// files loaded, in the table's order, and the files', each with the names
/// it spells, and every lexical and syntax error, each named by its input
/// number, the standard files loaded first, then the files: a file with
/// errors is its recovered tree (see [Syntax errors](#syntax-errors)),
/// whose imports load the standard files they name as any file's do.
fn parse_files(
    scan: &mut Scan,
    sorted: &[&SourceFile],
    library: Library,
    loaded: &mut Vec<usize>,
) -> (Trees, Trees, Vec<Diagnostic>) {
    // Each standard file's parse, by its index in the table, once loaded.
    let mut standard: Vec<Option<Parsed>> = library.table.iter().map(|_| None).collect();
    let mut queue: Vec<usize> = if library.every {
        (0..library.table.len()).rev().collect()
    } else {
        library.find(PRELUDE).into_iter().collect()
    };
    load_standard(scan, library, &mut standard, &mut queue);
    let files: Vec<Parsed> = sorted
        .iter()
        .map(|source| parse(source, &mut scan.names))
        .collect();
    for (ast, _, _) in &files {
        queue.extend(standard_imports(ast, &scan.names, library));
    }
    load_standard(scan, library, &mut standard, &mut queue);
    loaded.clear();
    loaded.extend((0..standard.len()).filter(|&at| matches!(standard.get(at), Some(Some(_)))));
    let mut errors = Vec::new();
    let mut trees = Vec::with_capacity(loaded.len() + files.len());
    let numbered = standard.into_iter().flatten().chain(files);
    for (index, (ast, spelled, found)) in (0u32..).zip(numbered) {
        errors.extend(found.into_iter().map(|error| error.in_input(index)));
        trees.push((ast, spelled));
    }
    let files = trees.split_off(loaded.len());
    (trees, files, errors)
}

/// Parses each standard file of `library` that `queue` holds, and each it
/// imports in turn, into `standard`, by its index in the library's table,
/// unless it is there already, emptying `queue`.
fn load_standard(
    scan: &mut Scan,
    library: Library,
    standard: &mut [Option<Parsed>],
    queue: &mut Vec<usize>,
) {
    while let Some(at) = queue.pop() {
        let (Some(slot @ None), Some(&(stem, text))) =
            (standard.get_mut(at), library.table.get(at))
        else {
            continue;
        };
        let Some(source) = standard_source(stem, text) else {
            continue;
        };
        let parsed = parse(&source, &mut scan.names);
        queue.extend(standard_imports(&parsed.0, &scan.names, library));
        *slot = Some(parsed);
    }
}

/// The indices in `library`'s table of the standard files the file of tree
/// `ast` imports, its names being `names`: each `import` whose path is one
/// segment naming a standard file by its stem, in whatever form (`import
/// path`, `import path as p`, `import join from path`, `import * from
/// path`). A path of more than one segment never names one.
fn standard_imports<'a>(
    ast: &'a Ast,
    names: &'a Names,
    library: Library<'a>,
) -> impl Iterator<Item = usize> + 'a {
    ast.items(ast.root()).iter().filter_map(move |&item| {
        let ItemKind::Import(id) = ast.item(item).kind else {
            return None;
        };
        let [only] = ast.path(ast.import_decl(id).path) else {
            return None;
        };
        library.find(names.get(only.name()?))
    })
}

/// Adds the directories of the workspace to `scan`, in path order, the root
/// first, as its files are, `sorted`, placed as `placing` says (a lone
/// file's is the root). Returns them by path, and each file's.
fn place_dirs(
    scan: &mut Scan,
    sorted: &[&SourceFile],
    placing: Placing,
) -> (HashMap<String, u32>, Vec<u32>) {
    let mut dirs: HashMap<String, u32> = HashMap::new();
    intern_dir(&mut scan.dirs, &mut dirs, "");
    let user_dirs: Vec<u32> = sorted
        .iter()
        .map(|source| match placing {
            Placing::Workspace(_) => {
                intern_dir(&mut scan.dirs, &mut dirs, parent_path(source.name()))
            }
            Placing::Lone(..) => 0,
        })
        .collect();
    (dirs, user_dirs)
}

/// Adds the nodes of the `builtins` module's tree, a root leaf for each
/// built-in function in the order of [`Builtin::ALL`], before any file's:
/// they are nodes `0..`, so their cells are the first slots of the frame.
/// No file owns or contributes to them.
fn builtin_nodes(scan: &mut Scan) {
    let dir = scan.builtins_dir;
    for builtin in Builtin::ALL {
        let name = scan.names.intern(builtin.name());
        let node = count(scan.nodes.len());
        scan.nodes.push(Node {
            name,
            parent: None,
            children: Vec::new(),
            dir,
            leaf: None,
            contributors: Vec::new(),
            shadowable: false,
        });
        scan.by_name.insert((dir, ROOT, name), node);
        if let Some(roots) = scan.roots.get_mut(dir as usize) {
            roots.push(node);
        }
    }
}

/// Reports each file or directory of the workspace's root named like a
/// module of the standard library, the names it reserves: "`builtins` is
/// the name of a built-in module" (and so for `fileroot`), "`prelude` is
/// the name of a standard module", once per name, at the start of the first
/// file that has it (the file, or the first file under the directory), in
/// path order. Every
/// standard file of `library` reserves its stem, whether the scan loads it
/// or not.
fn reserved(scan: &Scan, library: Library, errors: &mut Vec<Diagnostic>) {
    let mut reported: Vec<&str> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files).skip(scan.standard as usize) {
        let first = file.path.split('/').next().unwrap_or("");
        let name = if file.path.contains('/') {
            first
        } else {
            first.strip_suffix(".gcsl").unwrap_or(first)
        };
        let message = if name == BUILTINS || name == Fileroot::MODULE {
            format!("`{name}` is the name of a built-in module")
        } else if library.find(name).is_some() {
            format!("`{name}` is the name of a standard module")
        } else {
            continue;
        };
        if reported.contains(&name) {
            continue;
        }
        reported.push(name);
        errors.push(Diagnostic::error(message, Span::new(0, 0)).in_input(index));
    }
}

/// Reports the `#![no_prelude]` of a session's input, the scan's one file
/// after the standard files: "`#![no_prelude]` applies only to a file", at
/// the attribute, since an input is no file and always has the prelude.
fn no_prelude_in_input(scan: &Scan, errors: &mut Vec<Diagnostic>) {
    for (index, file) in (0u32..).zip(&scan.files).skip(scan.standard as usize) {
        if let Some(span) = file.ast.no_prelude() {
            errors.push(
                Diagnostic::error("`#![no_prelude]` applies only to a file", span).in_input(index),
            );
        }
    }
}

/// Sweeps the declarations of the files of each directory together, in the
/// static order (path order, then each file's source order), and gives each
/// file whose declarations a later one shadows, in it or in a later file of
/// its directory, its mentions found again without them (see the module
/// documentation, "Shadowable declarations"): a declaration beaten within
/// its own directory's tree is in no tree, and is not a leaf of anything.
/// Each file's own sweep, [`mentions::scan`], found those a later one of its
/// own shadows already; only those a later file shadows are new.
fn shadow(scan: &mut Scan) {
    // Only a marked declaration is ever shadowed: a directory none of whose
    // files marks one is not swept.
    let marked: HashSet<u32> = scan
        .files
        .iter()
        .filter(|file| file.ast.marks_any())
        .map(|file| file.dir)
        .collect();
    if marked.is_empty() {
        return;
    }
    let mut sweeps: HashMap<u32, Sweep<(u32, usize)>> = HashMap::new();
    let mut shadowed: Vec<Option<Vec<bool>>> = vec![None; scan.files.len()];
    for (index, file) in (0u32..).zip(&scan.files) {
        if !marked.contains(&file.dir) {
            continue;
        }
        let sweep = sweeps.entry(file.dir).or_default();
        for (position, names, shadowable) in mentions::declarations(&file.ast) {
            // A declaration the scan dropped stands nowhere.
            if file.dropped.binary_search(&position).is_ok() {
                continue;
            }
            let killed = sweep.declare(&names, (index, position), shadowable);
            for (owner, at) in killed.unwrap_or_default() {
                let Some(known) = scan.files.get(owner as usize) else {
                    continue;
                };
                if known.mentions.shadowed.get(at) == Some(&true) {
                    continue;
                }
                let Some(entry) = shadowed.get_mut(owner as usize) else {
                    continue;
                };
                let entry = entry.get_or_insert_with(|| known.mentions.shadowed.clone());
                if let Some(flag) = entry.get_mut(at) {
                    *flag = true;
                }
            }
        }
    }
    for (file, shadowed) in scan.files.iter_mut().zip(shadowed) {
        if let Some(shadowed) = shadowed {
            file.mentions = mentions::scan_except(&file.ast, &shadowed);
        }
    }
}

/// Marks each interior node of every directory's merged tree shadowable if
/// all its leaves are (a leaf is marked as it is merged): a child comes
/// after its parent in [`Scan::nodes`], so its mark is known first.
fn shadowable_nodes(scan: &mut Scan) {
    for index in (0..scan.nodes.len()).rev() {
        let Some(node) = scan.nodes.get(index) else {
            continue;
        };
        if node.leaf.is_some() || node.children.is_empty() {
            continue;
        }
        let shadowable = node.children.iter().all(|&child| {
            scan.nodes
                .get(child as usize)
                .is_some_and(|child| child.shadowable)
        });
        if let Some(node) = scan.nodes.get_mut(index) {
            node.shadowable = shadowable;
        }
    }
}

/// Resolves every file's references against its view ([`FileScope`]).
fn resolve(scan: &mut Scan) {
    for index in 0..scan.files.len() {
        let mut mentions = scan
            .files
            .get_mut(index)
            .map(|file| mem::take(&mut file.mentions))
            .unwrap_or_default();
        if let Some(file) = scan.files.get(index) {
            // The functions stay: the groups are found among them.
            let (functions, tree) = (mentions.functions.clone(), mem::take(&mut mentions.tree));
            let mut scope = FileScope::new(scan, file);
            (scope.own, scope.tree) = (&functions, &tree);
            mentions.resolve(&file.ast, &scope);
            mentions.tree = tree;
        }
        if let Some(file) = scan.files.get_mut(index) {
            file.mentions = mentions;
        }
    }
}

/// Lexes and parses `source`, interning into `names`, going on past every
/// error ([`lexer::lex`], [`parser::parse_all`]); the tree's own table is
/// left empty. With the tree, recovered from the errors, the names its
/// checker can look up (see [`File::spelled`]), and the errors, in source
/// order and capped ([`parser::capped`]).
fn parse(source: &SourceFile, names: &mut Names) -> Parsed {
    let lexer::Lexed {
        tokens,
        mut strings,
        diagnostics: lexical,
    } = lexer::lex(source, names);
    let (builder, root, syntax) =
        parser::parse_parts(&tokens, source, (names, &mut strings), lexical.len());
    let errors = parser::capped(lexical, syntax);
    let mut spelled: Vec<Name> = tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Ident(name) => Some(name),
            // A quoted field name is a plain string literal's text.
            TokenKind::Str(text) => names.lookup(strings.get(text)),
            _ => None,
        })
        .collect();
    spelled.sort_unstable();
    spelled.dedup();
    let mut ast = builder.finish(root, mem::take(names), strings);
    *names = ast.replace_names(Names::new());
    // Every file but one marked `#![no_prelude]` has the implicit
    // `#[shadowable] import * from prelude` (see "Shadowable
    // declarations").
    if ast.no_prelude().is_none() {
        ast.add_implicit_import(names.intern(PRELUDE));
    }
    (ast, spelled, errors)
}

/// The directory `path`, added with the directories above it if new.
fn intern_dir(dirs: &mut Vec<Dir>, index: &mut HashMap<String, u32>, path: &str) -> u32 {
    if let Some(&dir) = index.get(path) {
        return dir;
    }
    let parent = (!path.is_empty()).then(|| intern_dir(dirs, index, parent_path(path)));
    let dir = count(dirs.len());
    dirs.push(Dir {
        path: path.to_owned(),
        parent,
    });
    index.insert(path.to_owned(), dir);
    dir
}

/// The span of the path of the field item of `file`'s leaf `own`, and of
/// the whole item.
fn leaf_spans(file: &File, own: u32) -> Option<(Span, Span)> {
    let node = file.mentions.tree.nodes.get(own as usize)?;
    let leaf = node.leaf?;
    let item = file.ast.item(leaf.item);
    let path = match item.kind {
        ItemKind::Field { path, .. } => {
            let segments = file.ast.path(path);
            let first = segments.first().map(|s: &PathSegment| s.span)?;
            let last = segments.last().map_or(first, |s| s.span);
            Span::new(first.start, last.end)
        }
        // A definition made with a def-type, or a definition's meta: its
        // def-type and its path.
        ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. } => {
            let def = file.ast.def_type(def);
            let segments = file.ast.path(def.path);
            Span::join(file.ast.get(def.dt).span, ast::path_span(segments))
        }
        _ => item.span,
    };
    Some((path, item.span))
}

/// The span of the first leaf of `file` at or under its own node `own`.
fn first_leaf(file: &File, own: u32) -> Option<(Span, Span)> {
    let mut at = own;
    loop {
        let node = file.mentions.tree.nodes.get(at as usize)?;
        if node.leaf.is_some() {
            return leaf_spans(file, at);
        }
        at = *node.children.first()?;
    }
}

/// The positions among `file`'s items of its leaves at or under its own
/// node `own`.
fn leaves_under(file: &File, own: u32) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut stack = vec![own];
    while let Some(at) = stack.pop() {
        let Some(node) = file.mentions.tree.nodes.get(at as usize) else {
            continue;
        };
        if let Some(leaf) = node.leaf {
            positions.push(leaf.position);
        }
        stack.extend(&node.children);
    }
    positions
}

/// Adds `file`'s own tree to its directory's merged tree, reporting the
/// conflicts with the files before it, and numbers its functions. The
/// declarations of each conflict of the file, its leaves at or under its
/// node that conflicts, are added to `dropped`, by file and position (see
/// [`merge_trees`]).
fn merge(
    scan: &mut Scan,
    file: u32,
    errors: &mut Vec<Diagnostic>,
    dropped: &mut Vec<(u32, usize)>,
) {
    let Some(entry) = scan.files.get(file as usize) else {
        return;
    };
    let dir = entry.dir;
    let mut global = Vec::with_capacity(entry.mentions.tree.nodes.len());
    for (own, node) in (0u32..).zip(&entry.mentions.tree.nodes) {
        let parent = match node.parent {
            Some(up) => global.get(up as usize).copied().unwrap_or(ROOT),
            None => ROOT,
        };
        let key = (dir, parent, node.name);
        let merged = if let Some(&existing) = scan.by_name.get(&key) {
            if let Some(clash) = conflict(scan, file, own, existing) {
                errors.push(clash);
                dropped.extend(leaves_under(entry, own).into_iter().map(|at| (file, at)));
            }
            existing
        } else {
            let merged = count(scan.nodes.len());
            scan.nodes.push(Node {
                name: node.name,
                parent: (parent != ROOT).then_some(parent),
                children: Vec::new(),
                dir,
                leaf: None,
                contributors: Vec::new(),
                shadowable: false,
            });
            scan.by_name.insert(key, merged);
            if let Some(up) = scan.nodes.get_mut(parent as usize) {
                up.children.push(merged);
            } else if let Some(roots) = scan.roots.get_mut(dir as usize) {
                roots.push(merged);
            }
            merged
        };
        if let Some(target) = scan.nodes.get_mut(merged as usize) {
            if node.leaf.is_some() && target.leaf.is_none() && target.children.is_empty() {
                target.leaf = Some(file);
                target.shadowable = node.shadowable;
            }
            if target.contributors.last() != Some(&file) {
                target.contributors.push(file);
            }
        }
        global.push(merged);
    }
    let mut functions = Vec::with_capacity(entry.mentions.functions.len());
    for (index, function) in (0u32..).zip(&entry.mentions.functions) {
        let id = count(scan.functions.len());
        scan.functions.push(Function { file, index });
        let is_def = entry
            .mentions
            .tree
            .item_leaf
            .get(function.position)
            .is_some_and(Option::is_some);
        // A `def` named by a dotted path is found by its path, through the
        // merged tree, never by a name.
        if is_def && let Some(name) = function.name {
            scan.defs.entry((dir, name)).or_insert(id);
        }
        functions.push(id);
    }
    if let Some(entry) = scan.files.get_mut(file as usize) {
        entry.global = global;
        entry.functions = functions;
    }
}

/// The conflict, if any, of `file`'s own node `own` with the merged node
/// `existing` that earlier files declared.
fn conflict(scan: &Scan, input: u32, own: u32, existing: u32) -> Option<Diagnostic> {
    let file = scan.files.get(input as usize)?;
    let node = file.mentions.tree.nodes.get(own as usize)?;
    let merged = scan.nodes.get(existing as usize)?;
    let shown = scan.show(existing);
    if let Some(other) = merged.leaf {
        let other_file = scan.files.get(other as usize)?;
        let other_own = other_file
            .global
            .iter()
            .position(|&g| g == existing)
            .map(count)?;
        let (other_path, other_item) = leaf_spans(other_file, other_own)?;
        let (path, _) = first_leaf(file, own)?;
        return Some(if node.leaf.is_some() {
            Diagnostic::error(format!("field `{shown}` is defined in two files"), path)
                .with_label("defined here")
                .with_context_in(other, other_path, "defined here")
                .in_input(input)
        } else {
            Diagnostic::error(format!("field `{shown}` is already defined"), path)
                .with_context_in(other, other_item, "defined here")
                .in_input(input)
        });
    }
    if node.leaf.is_some() {
        let first = *merged.contributors.first()?;
        let other_file = scan.files.get(first as usize)?;
        let other_own = other_file
            .global
            .iter()
            .position(|&g| g == existing)
            .map(count)?;
        let (_, other_item) = first_leaf(other_file, other_own)?;
        let (path, _) = leaf_spans(file, own)?;
        return Some(
            Diagnostic::error(format!("field `{shown}` is already defined"), path)
                .with_context_in(first, other_item, "defined here")
                .in_input(input),
        );
    }
    None
}

/// One source of a level of a file's view as it is merged: what the
/// source has at that level.
#[derive(Clone, Copy, Debug)]
enum Level {
    /// The roots of a directory of the file's chain, with the source's
    /// position ([`ViewPart::source`]).
    Dir(u32, u32),
    /// An imported member or root of a `*`: its bound name and its node.
    Alias(Name, ViewPart),
    /// The children of a part of a node of the view.
    Of(ViewPart),
}

/// A conflict found merging a file's view: the view node it is under
/// ([`ROOT`] for a root, else a view node by its index among the file's),
/// its name, and the parts, at least one a leaf.
type Conflict = (u32, Name, Vec<ViewPart>);

/// A file's view as [`file_view`] finds it: its nodes, numbered from 0,
/// what the merge made of each name several sources have, the conflicts,
/// and the imports gone from it, by their indices among the file's
/// aliases.
type FileView = (
    Vec<ViewNode>,
    HashMap<(u32, Name), Merged>,
    Vec<Conflict>,
    Vec<u32>,
);

/// What identifies one side of a conflict, so that a conflict seen from
/// several files' views is reported once: its node, and for an import the
/// importing file and where the import binds it.
type Side = (Target, Option<(u32, u32)>);

impl Scan {
    /// The number of names `level` has.
    fn level_size(&self, level: Level) -> usize {
        match level {
            Level::Dir(dir, _) => self.roots.get(dir as usize).map_or(0, Vec::len),
            Level::Alias(..) => 1,
            Level::Of(part) => match part.target {
                Target::Node(node) => self
                    .nodes
                    .get(node as usize)
                    .map_or(0, |n| n.children.len()),
                Target::Part { file, node } => self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                    .map_or(0, |n| n.children.len()),
                _ => 0,
            },
        }
    }

    /// Each name `level` has, with its node, in the source's order.
    fn level_entries(&self, level: Level) -> Vec<(Name, ViewPart)> {
        match level {
            Level::Dir(dir, source) => {
                let roots = self.roots.get(dir as usize).map_or(&[][..], Vec::as_slice);
                roots
                    .iter()
                    .filter_map(|&root| {
                        let name = self.nodes.get(root as usize)?.name;
                        Some((name, ViewPart::chain(source, root)))
                    })
                    .collect()
            }
            Level::Alias(name, part) => vec![(name, part)],
            Level::Of(part) => self
                .child_names(part.target)
                .into_iter()
                .filter_map(|name| Some((name, self.level_lookup(level, name)?)))
                .collect(),
        }
    }

    /// The node `level` has of the name `name`, if any.
    fn level_lookup(&self, level: Level, name: Name) -> Option<ViewPart> {
        match level {
            Level::Dir(dir, source) => self
                .by_name
                .get(&(dir, ROOT, name))
                .map(|&root| ViewPart::chain(source, root)),
            Level::Alias(bound, part) => (bound == name).then_some(part),
            Level::Of(part) => Some(ViewPart {
                target: self.target_child(part.target, name)?,
                ..part
            }),
        }
    }

    /// Each name that more than one of `levels`, the sources of one node of
    /// a view, have, with their nodes, sources root directory first. The
    /// largest source is never walked, only looked up in, so that a level
    /// costs the size of the others.
    fn overlaps(&self, levels: &[Level]) -> Vec<(Name, Vec<ViewPart>)> {
        if levels.len() < 2 {
            return Vec::new();
        }
        let largest = (0..levels.len())
            .max_by_key(|&at| levels.get(at).map_or(0, |&level| self.level_size(level)));
        let mut found: HashMap<Name, Vec<ViewPart>> = HashMap::new();
        let mut order = Vec::new();
        for (at, &level) in levels.iter().enumerate() {
            if Some(at) == largest {
                continue;
            }
            for (name, part) in self.level_entries(level) {
                found
                    .entry(name)
                    .or_insert_with(|| {
                        order.push(name);
                        Vec::new()
                    })
                    .push(part);
            }
        }
        let largest = largest.and_then(|at| levels.get(at)).copied();
        let mut overlapping = Vec::new();
        for name in order {
            let Some(mut parts) = found.remove(&name) else {
                continue;
            };
            if let Some(level) = largest
                && let Some(part) = self.level_lookup(level, name)
            {
                parts.push(part);
            }
            if parts.len() > 1 {
                parts.sort_by_key(|part| part.source);
                overlapping.push((name, parts));
            }
        }
        overlapping
    }

    /// `parts`, the nodes several sources have at one name, with the same
    /// cell reached twice taken once: a node met again, and a file's part
    /// of a merged node also met whole. Merging is idempotent. A cell
    /// reached both through a `#[shadowable]` import and plainly is the
    /// plain one, which stands (see the module documentation, "Shadowable
    /// declarations"): of two reachings of one node, the one kept is the
    /// first but for a marked one met again plainly, which the plain one
    /// replaces, its place in the order with it; and the node a file's part
    /// is met as whole is marked only if the part is too.
    fn same_cells(&self, parts: &[ViewPart]) -> Vec<ViewPart> {
        let whole = |part: &ViewPart| match part.target {
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.global.get(node as usize))
                .copied(),
            _ => None,
        };
        let mut kept: Vec<ViewPart> = Vec::with_capacity(parts.len());
        for part in parts {
            if let Some(same) = kept.iter_mut().find(|k| k.target == part.target) {
                if same.marked && !part.marked {
                    *same = *part;
                }
                continue;
            }
            if let Some(merged) = whole(part)
                && parts
                    .iter()
                    .any(|other| other.target == Target::Node(merged))
            {
                continue;
            }
            kept.push(*part);
        }
        // A part met whole too leaves its plainness to the whole node.
        for part in parts {
            if !part.marked
                && let Some(merged) = whole(part)
                && let Some(node) = kept.iter_mut().find(|k| k.target == Target::Node(merged))
            {
                node.marked = false;
            }
        }
        kept
    }

    /// Whether `target` is shadowable as its declarations say: a merged
    /// node or a file's part of one all of whose leaves are marked
    /// `#[shadowable]` (see the module documentation, "Shadowable
    /// declarations").
    fn target_shadowable(&self, target: Target) -> bool {
        match target {
            Target::Node(node) => self.nodes.get(node as usize).is_some_and(|n| n.shadowable),
            Target::Part { file, node } => self
                .files
                .get(file as usize)
                .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                .is_some_and(|n| n.shadowable),
            Target::View(view) => self
                .views
                .get(view as usize)
                .is_some_and(|v| v.parts.iter().all(|&part| self.part_shadowable(part))),
            Target::Dir(_) | Target::File(_) | Target::Fileroot(_) => false,
        }
    }

    /// Whether the part `part` of a node of a file's view is shadowable:
    /// reached through a `#[shadowable]` import, or shadowable as its
    /// declarations say.
    fn part_shadowable(&self, part: ViewPart) -> bool {
        part.marked || self.target_shadowable(part.target)
    }

    /// The position among its file's items of the item declaring the leaf
    /// `node`, a merged node whose leaf is file `owner`'s.
    fn leaf_position(&self, owner: u32, node: u32) -> u32 {
        self.files
            .get(owner as usize)
            .and_then(|file| {
                let own = file.global.iter().position(|&g| g == node)?;
                let leaf = file.mentions.tree.nodes.get(own)?.leaf?;
                Some(count(leaf.position))
            })
            .unwrap_or(0)
    }

    /// Whether `target` is a leaf: a merged node declared by an item, or a
    /// built-in function's.
    fn is_leaf(&self, target: Target) -> bool {
        matches!(target, Target::Node(node)
            if self.nodes.get(node as usize).is_some_and(|n| n.leaf.is_some())
                || self.builtin(node).is_some())
    }

    /// The file that declares the first leaf at or under `target`, a merged
    /// node or a file's part of one, and the spans of that leaf's path and
    /// whole item.
    fn part_spans(&self, target: Target) -> Option<(u32, Span, Span)> {
        let (file, own) = match target {
            Target::Node(node) => {
                let entry = self.nodes.get(node as usize)?;
                let file = entry.leaf.or_else(|| entry.contributors.first().copied())?;
                let own = self
                    .files
                    .get(file as usize)?
                    .global
                    .iter()
                    .position(|&g| g == node)?;
                (file, count(own))
            }
            Target::Part { file, node } => (file, node),
            _ => return None,
        };
        let (path, item) = first_leaf(self.files.get(file as usize)?, own)?;
        Some((file, path, item))
    }
}

/// Finds every file's view (see the module documentation, "The view of a
/// file"): the nodes several of its sources have, merged, into
/// [`Scan::views`] and [`File::view`], each leaf shadowing what it overlaps
/// or shadowed by it as the static order says (see the module
/// documentation, "Shadowable declarations"), and the imports something
/// shadows taken from [`File::aliases`]. Reports each conflict, two
/// sources with a leaf and anything else of one path neither of which
/// shadows the other, once per pair however many files' views it is in.
fn views(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    let mut reported: HashSet<(Side, Side)> = HashSet::new();
    let mut found = Vec::with_capacity(scan.files.len());
    // Each file's imports gone from its view, and the roots of its
    // conflicts, which it reads as `Error` (see [Scan errors](#scan-errors)).
    let mut gone_from: Vec<(Vec<u32>, Vec<Name>)> = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let (mut nodes, mut merged, conflicts, gone) = file_view(scan, index);
        let mut roots = Vec::new();
        for (parent, name, parts) in conflicts {
            report_conflicts(
                scan,
                index,
                &nodes,
                (parent, name, &parts),
                &mut reported,
                errors,
            );
            roots.push(conflict_root(&nodes, parent, name));
        }
        // The imports gone from the view are no aliases of the file: the
        // others are renumbered.
        if !gone.is_empty() {
            let depth = scan
                .files
                .get(index as usize)
                .map_or(0, |file| count(scan.chain(file.dir).count()));
            let renumber = |part: &mut ViewPart| {
                if !part.chain {
                    let at = part.source.saturating_sub(depth);
                    let before = count(gone.partition_point(|&g| g < at));
                    part.source = part.source.saturating_sub(before);
                }
            };
            for node in &mut nodes {
                node.parts.iter_mut().for_each(renumber);
            }
            for value in merged.values_mut() {
                if let Merged::One(part) = value {
                    renumber(part);
                }
            }
        }
        gone_from.push((gone, roots));
        // Its own nodes first, each after its parent as before: a node with
        // a leaf of the file under it has only such nodes above it.
        let start = count(scan.views.len());
        let mut renumbered = vec![0u32; nodes.len()];
        let mut at = start;
        for own in [true, false] {
            for (local, node) in nodes.iter().enumerate() {
                if node.own.is_some() == own
                    && let Some(entry) = renumbered.get_mut(local)
                {
                    *entry = at;
                    at = at.saturating_add(1);
                }
            }
        }
        let own = count(nodes.iter().filter(|node| node.own.is_some()).count());
        let global = |local: u32| renumbered.get(local as usize).copied().unwrap_or(ROOT);
        for node in &mut nodes {
            node.parent = node.parent.map(global);
        }
        let mut ordered: Vec<Option<ViewNode>> = vec![None; nodes.len()];
        for (local, node) in nodes.into_iter().enumerate() {
            let slot = global(count(local)).saturating_sub(start);
            if let Some(entry) = ordered.get_mut(slot as usize) {
                *entry = Some(node);
            }
        }
        scan.views.extend(ordered.into_iter().flatten());
        let merged = merged
            .into_iter()
            .map(|((parent, name), value)| {
                let parent = if parent == ROOT { ROOT } else { global(parent) };
                let value = match value {
                    Merged::View(local) => Merged::View(global(local)),
                    one @ Merged::One(_) => one,
                };
                ((parent, name), value)
            })
            .collect();
        found.push(View {
            start,
            own,
            end: count(scan.views.len()),
            merged,
        });
    }
    for ((file, view), (gone, roots)) in scan.files.iter_mut().zip(found).zip(gone_from) {
        for root in roots {
            file.poison(root);
        }
        file.view = view;
        if !gone.is_empty() {
            let aliases = mem::take(&mut file.aliases);
            file.aliases = (0u32..)
                .zip(aliases)
                .filter(|(at, _)| gone.binary_search(at).is_err())
                .map(|(_, alias)| alias)
                .collect();
        }
    }
}

/// The root of the path of a conflict of a file's view at the name `name`
/// under `parent` ([`ROOT`] for a root, else one of `nodes`, the view's
/// nodes numbered from 0): what the file reads as `Error` (see [Scan
/// errors](#scan-errors)).
fn conflict_root(nodes: &[ViewNode], parent: u32, name: Name) -> Name {
    let mut root = name;
    let mut at = (parent != ROOT).then_some(parent);
    while let Some(view) = at
        && let Some(node) = nodes.get(view as usize)
    {
        root = node.name;
        at = node.parent;
    }
    root
}

/// A place in the static order of the declarations and imports reaching a
/// file's view (see the module documentation, "Shadowable declarations"),
/// as a key: the implicit prelude's roots `(0, 0, 0)`; then a directory
/// above the file's own, `(1, its position in the chain from the root, 0)`;
/// then a declaration of the file's own directory, or an import of the
/// file, `(2, its file, its position among its file's items)`; then an
/// import a session's input inherits, `(3, its index, 0)`.
type Rank = (u8, u32, u32);

/// The static order of the parts of one file's view (see the module
/// documentation, "Shadowable declarations").
struct Order<'a> {
    /// The workspace.
    scan: &'a Scan,
    /// The file.
    file: &'a File,
    /// Its index.
    index: u32,
    /// The number of directories of its chain.
    depth: u32,
}

impl<'a> Order<'a> {
    /// The order of the view of `file`, file `index` of `scan`, whose chain
    /// has `depth` directories.
    const fn new(scan: &'a Scan, (file, index): (&'a File, u32), depth: u32) -> Self {
        Self {
            scan,
            file,
            index,
            depth,
        }
    }

    /// The position among the file's items of its item `item`; found only
    /// where a leaf meets an import, which is rare.
    fn position(&self, item: ItemId) -> u32 {
        let items = self.file.ast.items(self.file.ast.root());
        count(items.iter().position(|&at| at == item).unwrap_or(0))
    }

    /// The earliest and the latest place of the declarations of `part`: an
    /// import's place, a directory above's, or the places of the leaves at
    /// or under a node of the file's own directory, which may come before
    /// and after the file's imports.
    fn ranks(&self, part: ViewPart) -> (Rank, Rank) {
        if !part.chain {
            let at = part.source.saturating_sub(self.depth);
            let rank = match self.file.aliases.get(at as usize) {
                Some(alias) if alias.implicit => (0, 0, 0),
                Some(alias) if !alias.inherited => {
                    let position = alias.item.map_or(0, |item| self.position(item));
                    (2, self.index, position)
                }
                _ => (3, at, 0),
            };
            return (rank, rank);
        }
        if part.source.saturating_add(1) < self.depth {
            let rank = (1, part.source, 0);
            return (rank, rank);
        }
        let Target::Node(node) = part.target else {
            return ((2, 0, 0), (2, 0, 0));
        };
        let (mut earliest, mut latest): (Option<Rank>, Option<Rank>) = (None, None);
        let mut stack = vec![node];
        while let Some(at) = stack.pop() {
            let Some(entry) = self.scan.nodes.get(at as usize) else {
                continue;
            };
            if let Some(owner) = entry.leaf {
                let rank = (2, owner, self.scan.leaf_position(owner, at));
                earliest = Some(earliest.map_or(rank, |known| known.min(rank)));
                latest = Some(latest.map_or(rank, |known| known.max(rank)));
            }
            stack.extend(&entry.children);
        }
        (earliest.unwrap_or((2, 0, 0)), latest.unwrap_or((2, 0, 0)))
    }
}

/// Of `parts`, the nodes several sources have at one name of a file's
/// view, at least one of them a leaf: those that stand once each, in the
/// static order `order`, has shadowed every earlier one it overlaps (a
/// leaf overlaps every other part; two interior nodes do not, at this
/// name), which it may only if every one of those is shadowable (see the
/// module documentation, "Shadowable declarations"), in source order, with
/// those shadowed; or, when one overlaps one that is not, the parts that
/// conflict, as they always did, those shadowed before it left out. An
/// interior node of the file's own directory with leaves both before and
/// after a leaf of another source, an import of the file, would stand only
/// in part, which a view cannot hold: that is the conflict, conservatively.
fn shadow_parts(
    scan: &Scan,
    order: &Order,
    parts: &[ViewPart],
) -> Result<(Vec<ViewPart>, Vec<ViewPart>), Vec<ViewPart>> {
    let mut ranked: Vec<(Rank, Rank, ViewPart)> = parts
        .iter()
        .map(|&part| {
            let (earliest, latest) = order.ranks(part);
            (earliest, latest, part)
        })
        .collect();
    ranked.sort_by_key(|&(earliest, _, part)| (earliest, part.source));
    let interleaved = ranked.iter().any(|&(at, _, leaf)| {
        scan.is_leaf(leaf.target)
            && ranked.iter().any(|&(earliest, latest, other)| {
                !scan.is_leaf(other.target) && earliest < at && at < latest
            })
    });
    if interleaved {
        // The implicit prelude's, first of all, is shadowed whatever the
        // rest does.
        return Err(ranked
            .iter()
            .filter(|&&(earliest, _, _)| earliest.0 != 0)
            .map(|&(_, _, part)| part)
            .collect());
    }
    let mut live: Vec<ViewPart> = Vec::with_capacity(parts.len());
    let mut gone: Vec<ViewPart> = Vec::new();
    for (_, _, part) in ranked {
        let overlaps =
            scan.is_leaf(part.target) || live.iter().any(|stands| scan.is_leaf(stands.target));
        if live.is_empty() || !overlaps {
            live.push(part);
        } else if live.iter().all(|&stands| scan.part_shadowable(stands)) {
            gone.append(&mut live);
            live.push(part);
        } else {
            return Err(parts
                .iter()
                .filter(|part| !gone.contains(part))
                .copied()
                .collect());
        }
    }
    live.sort_by_key(|part| part.source);
    Ok((live, gone))
}

/// Lets each whole import of `file` (`import PATH [as NAME]`), whose chain
/// has `depth` directories, take its name in the view, `merged` (by
/// parent, [`ROOT`] for a root, and name), from whatever the view has of
/// it there, if that is shadowable (see the module documentation,
/// "Shadowable declarations"): a root of the directory chain, a member or
/// `*` import, or a node of the view, `nodes` being the view's so far. One
/// of a name that is not is the clash [`clashes`] reports. Each member or
/// `*` import it shadows is added to `gone`, by its index among the file's
/// aliases.
fn whole_imports_shadow(
    scan: &Scan,
    (file, depth): (&File, u32),
    (nodes, merged): (&[ViewNode], &mut HashMap<(u32, Name), Merged>),
    gone: &mut Vec<u32>,
) {
    for (at, alias) in (0u32..).zip(&file.aliases) {
        if !alias.whole {
            continue;
        }
        let shadowable = match merged.get(&(ROOT, alias.name)) {
            Some(&Merged::One(part)) => Some(scan.part_shadowable(part)),
            Some(&Merged::View(view)) => nodes
                .get(view as usize)
                .map(|node| node.parts.iter().all(|&part| scan.part_shadowable(part))),
            None => scan
                .chain(file.dir)
                .find_map(|dir| scan.by_name.get(&(dir, ROOT, alias.name)))
                .map(|&node| scan.target_shadowable(Target::Node(node)))
                .or_else(|| {
                    file.aliases
                        .iter()
                        .find(|other| !other.whole && other.name == alias.name)
                        .map(|other| file.marked(other) || scan.target_shadowable(other.target))
                }),
        };
        if shadowable == Some(true) {
            let part = ViewPart {
                source: depth.saturating_add(at),
                target: alias.target,
                chain: false,
                marked: false,
            };
            merged.insert((ROOT, alias.name), Merged::One(part));
            gone.extend(
                (0u32..)
                    .zip(&file.aliases)
                    .filter(|(_, other)| !other.whole && other.name == alias.name)
                    .map(|(other, _)| other),
            );
        }
    }
}

/// The view of file `index`: its view nodes, numbered from 0 in the order
/// found, each a parent before its children; what the merge made of each
/// name several sources have, by parent (a local number or [`ROOT`]); the
/// conflicts; and the member and `*` imports of the file that something
/// shadows, gone from the view, by their indices among its aliases.
fn file_view(scan: &Scan, index: u32) -> FileView {
    let (mut nodes, mut merged, mut conflicts) = (Vec::new(), HashMap::new(), Vec::new());
    let mut gone: Vec<u32> = Vec::new();
    let Some(file) = scan.files.get(index as usize) else {
        return (nodes, merged, conflicts, gone);
    };
    let mut chain: Vec<u32> = scan.chain(file.dir).collect();
    chain.reverse();
    let depth = count(chain.len());
    let order = Order::new(scan, (file, index), depth);
    let mut levels: Vec<Level> = (0u32..)
        .zip(&chain)
        .map(|(source, &dir)| Level::Dir(dir, source))
        .collect();
    for (at, alias) in (0u32..).zip(&file.aliases) {
        if !alias.whole {
            let part = ViewPart {
                source: depth.saturating_add(at),
                target: alias.target,
                chain: false,
                marked: file.marked(alias),
            };
            levels.push(Level::Alias(alias.name, part));
        }
    }
    let local: HashMap<u32, u32> = (0u32..)
        .zip(&file.global)
        .map(|(own, &global)| (global, own))
        .collect();
    let mut stack = vec![(ROOT, levels)];
    while let Some((parent, levels)) = stack.pop() {
        for (name, all) in scan.overlaps(&levels) {
            let parts = scan.same_cells(&all);
            // A leaf among them shadows, or is shadowed by, what it
            // overlaps, or conflicts with it.
            let (parts, shadowed) =
                if parts.len() > 1 && parts.iter().any(|part| scan.is_leaf(part.target)) {
                    match shadow_parts(scan, &order, &parts) {
                        Ok(stands) => stands,
                        Err(conflicting) => {
                            conflicts.push((parent, name, conflicting));
                            continue;
                        }
                    }
                } else {
                    (parts, Vec::new())
                };
            // The imports gone from the view at a root: those shadowed, and
            // a marked one whose cell a plain one reaches too.
            if parent == ROOT {
                let replaced = all.iter().filter(|part| {
                    part.marked
                        && parts
                            .iter()
                            .any(|kept| kept.target == part.target && kept.source != part.source)
                });
                gone.extend(
                    shadowed
                        .iter()
                        .chain(replaced)
                        .filter(|part| !part.chain)
                        .map(|part| part.source.saturating_sub(depth)),
                );
            }
            if let [one] = parts.as_slice() {
                merged.insert((parent, name), Merged::One(*one));
                continue;
            }
            let own = parts.iter().find_map(|part| match part.target {
                Target::Node(node) if part.chain => scan
                    .nodes
                    .get(node as usize)
                    .filter(|n| n.contributors.contains(&index))
                    .and_then(|_| local.get(&node).copied()),
                _ => None,
            });
            let view = count(nodes.len());
            nodes.push(ViewNode {
                name,
                parent: (parent != ROOT).then_some(parent),
                file: index,
                parts: parts.clone(),
                own,
            });
            merged.insert((parent, name), Merged::View(view));
            stack.push((view, parts.into_iter().map(Level::Of).collect()));
        }
    }
    whole_imports_shadow(scan, (file, depth), (&nodes, &mut merged), &mut gone);
    gone.sort_unstable();
    gone.dedup();
    (nodes, merged, conflicts, gone)
}

/// Reports the conflict `conflict` of file `index`'s view, whose view
/// nodes so far are `nodes`: each part, in the order the sources are
/// reported in (its own directory, then each directory upward, then its
/// imports in source order), against the first before it that it conflicts
/// with, once per pair (`reported`), at the later of the two.
fn report_conflicts(
    scan: &Scan,
    index: u32,
    nodes: &[ViewNode],
    (parent, name, parts): (u32, Name, &[ViewPart]),
    reported: &mut HashSet<(Side, Side)>,
    errors: &mut Vec<Diagnostic>,
) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    let depth = count(scan.chain(file.dir).count());
    // Its own directory first (the last of the chain), then upward, then
    // the imports.
    let rank = |part: &ViewPart| {
        if part.chain {
            depth.saturating_sub(1).saturating_sub(part.source)
        } else {
            part.source
        }
    };
    let mut parts = parts.to_vec();
    parts.sort_by_key(rank);
    let mut path = Vec::new();
    let mut at = (parent != ROOT).then_some(parent);
    while let Some(view) = at
        && let Some(node) = nodes.get(view as usize)
    {
        path.push(scan.names.get(node.name));
        at = node.parent;
    }
    path.reverse();
    path.push(scan.names.get(name));
    let shown = path.join(".");
    for (later_at, later) in parts.iter().enumerate() {
        let earlier = parts
            .get(..later_at)
            .into_iter()
            .flatten()
            .find(|earlier| scan.is_leaf(earlier.target) || scan.is_leaf(later.target));
        let Some(earlier) = earlier else { continue };
        let side = |part: &ViewPart| -> Side {
            let import = (!part.chain).then(|| {
                let alias = part.source.saturating_sub(depth);
                let start = file.aliases.get(alias as usize).map_or(0, |a| a.span.start);
                (index, start)
            });
            (part.target, import)
        };
        if !reported.insert((side(later), side(earlier))) {
            continue;
        }
        if let Some(error) = conflict_between(scan, index, &shown, (*later, *earlier), depth) {
            errors.push(error);
        }
    }
}

/// The conflict at the view path `shown` of file `index` (whose chain has
/// `depth` directories) between the later part and the earlier one: two
/// leaves, "defined in two files", or a leaf and an interior node, "already
/// defined", at the later, labelled as the module documentation says.
fn conflict_between(
    scan: &Scan,
    index: u32,
    shown: &str,
    (later, earlier): (ViewPart, ViewPart),
    depth: u32,
) -> Option<Diagnostic> {
    let file = scan.files.get(index as usize)?;
    let leaves = scan.is_leaf(later.target) && scan.is_leaf(earlier.target);
    let message = if leaves {
        format!("field `{shown}` is defined in two files")
    } else {
        format!("field `{shown}` is already defined")
    };
    let imported_at = |part: &ViewPart| {
        file.aliases
            .get(part.source.saturating_sub(depth) as usize)
            .map(|alias| alias.span)
    };
    // A built-in function's side, which only an import reaches, has no
    // definition to show: a note says what it is instead.
    let builtin = |part: &ViewPart| match part.target {
        Target::Node(node) => scan.builtin(node),
        _ => None,
    };
    let mut error = if later.chain {
        let (later_file, later_path, _) = scan.part_spans(later.target)?;
        let error = Diagnostic::error(message, later_path).in_input(later_file);
        if leaves {
            error.with_label("defined here")
        } else {
            error
        }
    } else {
        let error = Diagnostic::error(message, imported_at(&later)?)
            .with_label("imported here")
            .in_input(index);
        match scan.part_spans(later.target) {
            Some((later_file, later_path, _)) => {
                context_in(error, later_file, later_path, "defined here")
            }
            None => error,
        }
    };
    if !earlier.chain {
        error = context_in(error, index, imported_at(&earlier)?, "imported here");
    }
    if let Some((earlier_file, earlier_path, earlier_item)) = scan.part_spans(earlier.target) {
        let earlier_span = if leaves { earlier_path } else { earlier_item };
        error = context_in(error, earlier_file, earlier_span, "defined here");
    }
    for part in [later, earlier] {
        if let Some(builtin) = builtin(&part) {
            error = error.with_note(format!("`{}` is a built-in function", builtin.name()));
        }
    }
    // A directory above the file's own is one side.
    let own_dir = depth.saturating_sub(1);
    if [later, earlier]
        .iter()
        .any(|part| part.chain && part.source != own_dir)
    {
        error = error.with_note("one is in a directory above");
    }
    Some(error)
}

/// `error` with the label `text` at `span` of file `input`: a label of the
/// primary span's own file when it is that file, else an excerpt of its
/// own.
fn context_in(error: Diagnostic, input: u32, span: Span, text: &str) -> Diagnostic {
    if error.input == Some(input) {
        error.with_context(span, text)
    } else {
        error.with_context_in(input, span, text)
    }
}

/// What [`imports`] looks modules up in.
pub(crate) struct Modules<'a> {
    /// The directories by path.
    dirs: &'a HashMap<String, u32>,
    /// The files by path.
    files: HashMap<&'a str, u32>,
    /// The directories that are workspaces of their own.
    nested: &'a [String],
}

impl<'a> Modules<'a> {
    /// The modules of `scan`, whose directories by path are `dirs`, as
    /// [`imports`] resolves a path among them, but with no directory left
    /// out as a workspace of its own: for an index of the scan's imports
    /// ([`crate::index`]), whose paths the scan has resolved already.
    pub(crate) fn of(scan: &'a Scan, dirs: &'a HashMap<String, u32>) -> Self {
        Self {
            dirs,
            files: (0u32..)
                .zip(&scan.files)
                .map(|(index, file)| (file.path.as_str(), index))
                .collect(),
            nested: &[],
        }
    }
}

/// Resolves every file's imports into [`File::aliases`], reporting each
/// path that names nothing, each directory and file of one name (once per
/// pair, at the first import naming it), each member that is not a root,
/// and each imported name that clashes with a name of its file or a root of
/// its directory chain. `nested` are the directories that are workspaces of
/// their own, and `inherited` the imports of a session's earlier inputs,
/// which a session's input, the scan's one file of its own, inherits. A
/// file's implicit import of the prelude ([`Ast::implicit_import`]) is
/// resolved last, its aliases after every other, though it is first in the
/// static order; with no prelude loaded (a test's table), it binds nothing.
fn imports(
    scan: &mut Scan,
    dirs: &HashMap<String, u32>,
    (nested, inherited): (&[String], &[Alias]),
    errors: &mut Vec<Diagnostic>,
) {
    let modules = Modules {
        dirs,
        files: (0u32..)
            .zip(&scan.files)
            .map(|(index, file)| (file.path.as_str(), index))
            .collect(),
        nested,
    };
    let mut both: Vec<String> = Vec::new();
    let mut found = Vec::with_capacity(scan.files.len());
    // The prelude is loaded by every scan, but for a test's table without
    // one: then the implicit import binds nothing.
    let prelude = scan.standard_file(PRELUDE).is_some();
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut aliases = Vec::new();
        let mut types = Vec::new();
        let mut own_errors = Vec::new();
        // The imports that failed, and what they bind (see [Scan
        // errors](#scan-errors)).
        let mut failed_imports: Vec<(ItemId, Option<Target>)> = Vec::new();
        let mut poisoned: Vec<Name> = Vec::new();
        let mut blind = false;
        let mut import =
            |item: ItemId, (aliases, types): (&mut Vec<Alias>, &mut Vec<TypeImport>)| {
                let ItemKind::Import(id) = file.ast.item(item).kind else {
                    return;
                };
                let decl = file.ast.import_decl(id);
                let path = file.ast.path(decl.path);
                let before = own_errors.len();
                let resolved = module(scan, &modules, path, &mut both);
                let module_of = resolved.as_ref().ok().copied();
                let failed = match resolved {
                    Ok(target) => {
                        let bound = (&mut *aliases, &mut *types);
                        members(scan, (file, item, decl), target, bound, &mut own_errors);
                        own_errors.len() > before
                    }
                    Err(error) => {
                        // A lone file's import of anything but the standard
                        // library is the checker's to report.
                        let failed = error.is_some() || !scan.lone;
                        own_errors.extend(error);
                        failed
                    }
                };
                if own_errors.len() > before {
                    failed_imports.push((item, module_of));
                }
                if failed {
                    let (names, star) = import_binds(&file.ast, decl);
                    poisoned.extend(names.into_iter().filter(|&name| {
                        !aliases
                            .iter()
                            .any(|a| a.item == Some(item) && a.name == name)
                            && !types.iter().any(|t| t.name == name)
                    }));
                    blind |= star;
                }
            };
        for &item in file.ast.items(file.ast.root()) {
            import(item, (&mut aliases, &mut types));
        }
        if index >= scan.standard {
            inherit_imports(file, inherited, &mut aliases);
        }
        // The implicit `import * from prelude` is first of all in the
        // static order whatever its place among the aliases, which is last,
        // so that every other keeps its number.
        if let Some(item) = file.ast.implicit_import().filter(|_| prelude) {
            import(item, (&mut aliases, &mut types));
        }
        clashes(scan, index, &aliases, (&mut own_errors, &mut poisoned));
        errors.extend(own_errors.into_iter().map(|e| e.in_input(index)));
        found.push((aliases, types, (poisoned, blind, failed_imports)));
    }
    for (file, (aliases, types, (poisoned, blind, failed))) in scan.files.iter_mut().zip(found) {
        file.aliases = aliases;
        file.type_imports = types;
        file.failed_imports = failed;
        for name in poisoned {
            file.poison(name);
        }
        file.blind |= blind;
    }
}

/// The names the import `decl` of the tree `ast` binds, each member's (or
/// its alias), or a whole import's, and whether it has a `*`, whose names
/// are not known from it alone.
fn import_binds(ast: &Ast, decl: &ImportDecl) -> (Vec<Name>, bool) {
    match decl.form {
        ImportForm::Whole(alias) => {
            let bound = alias
                .map(|a| a.name)
                .or_else(|| ast.path(decl.path).last().and_then(PathSegment::name));
            (bound.into_iter().collect(), false)
        }
        ImportForm::Members(range) => {
            let mut names = Vec::new();
            let mut star = false;
            for member in ast.import_members(range) {
                match *member {
                    ImportMember::Named { name, alias } => names.push(alias.unwrap_or(name).name),
                    ImportMember::All(_) => star = true,
                }
            }
            (names, star)
        }
    }
}

/// Adds to `aliases`, the imports of `file`, a session's input, each of
/// `inherited`, the imports of the session's earlier inputs, that the input
/// neither imports again nor declares in the whole of its top level (by a
/// `fn`, field or `def` whose path starts with its name, or a `:=` of it,
/// which means the variable of its name): a name of an earlier input is
/// shadowed by a later input's, whether it was imported or declared (see
/// [`crate::session`]). A `let` shadows one only from its point on, as it
/// does an earlier input's variable: the checker reads an inherited import
/// through the view until a binding of its name is found
/// ([`FileScope::inherited`]). They follow the input's own imports, before
/// the implicit prelude's aliases but last in the static order (see the
/// module documentation, "Shadowable declarations"), so they shadow the
/// prelude's names, and they clash with nothing.
fn inherit_imports(file: &File, inherited: &[Alias], aliases: &mut Vec<Alias>) {
    if inherited.is_empty() {
        return;
    }
    let ast = &file.ast;
    let declared: HashSet<Name> = ast
        .items(ast.root())
        .iter()
        .filter_map(|&item| match ast.item(item).kind {
            ItemKind::Let {
                path,
                function: true,
                ..
            }
            | ItemKind::Rebind { path, .. }
            | ItemKind::Field { path, .. } => ast.path(path).first().and_then(PathSegment::name),
            _ => None,
        })
        .collect();
    let own = aliases.len();
    for alias in inherited {
        let imported = aliases.iter().take(own).any(|a| a.name == alias.name);
        if !imported && !declared.contains(&alias.name) {
            aliases.push(*alias);
        }
    }
}

/// What the module path `path` names, or the error that it names nothing
/// (`None` for a directory and file of one name already reported, and for
/// any path of a lone file but the standard library's, which the checker
/// reports). A single segment naming the `builtins` module, the `fileroot`
/// module or a standard file names it, before anything of the workspace.
pub(crate) fn module(
    scan: &Scan,
    modules: &Modules,
    path: &[PathSegment],
    both: &mut Vec<String>,
) -> Result<Target, Option<Diagnostic>> {
    let written = match (path.first(), path.last()) {
        (Some(first), Some(last)) => first.span.join(last.span),
        _ => return Err(None),
    };
    let named: Vec<(&str, Span)> = path
        .iter()
        .filter_map(|segment| Some((scan.names.get(segment.name()?), segment.span)))
        .collect();
    let shown: Vec<&str> = named.iter().map(|&(text, _)| text).collect();
    if let [only] = shown.as_slice() {
        if *only == BUILTINS {
            return Ok(Target::Dir(scan.builtins_dir));
        }
        if *only == Fileroot::MODULE {
            return Ok(Target::Fileroot(None));
        }
        if let Some(file) = scan.standard_file(only) {
            return Ok(Target::File(file));
        }
    }
    if scan.lone {
        return Err(None);
    }
    let mut at = String::new();
    for (position, name) in shown.iter().enumerate() {
        let candidate = if at.is_empty() {
            (*name).to_owned()
        } else {
            format!("{at}/{name}")
        };
        let dir = modules.dirs.get(&candidate).copied();
        let last = position + 1 == shown.len();
        let file = last
            .then(|| {
                modules
                    .files
                    .get(format!("{candidate}.gcsl").as_str())
                    .copied()
            })
            .flatten();
        match (dir, file) {
            (Some(_), Some(_)) => {
                if both.contains(&candidate) {
                    return Err(None);
                }
                both.push(candidate);
                return Err(Some(Diagnostic::error(
                    format!("`{name}` is both a directory and a file"),
                    written,
                )));
            }
            (None, Some(file)) => return Ok(Target::File(file)),
            (Some(dir), None) if last => return Ok(Target::Dir(dir)),
            (Some(_), None) => at = candidate,
            (None, None) => {
                let nested = modules.nested.contains(&candidate);
                let note = if nested {
                    format!("`{candidate}/` is a workspace of its own")
                } else {
                    let listed = contents(scan, modules.dirs.get(&at).copied());
                    if at.is_empty() {
                        format!("the workspace has {listed}")
                    } else {
                        format!("`{at}/` has {listed}")
                    }
                };
                let message = format!("no module `{}`", shown.join("/"));
                // A path written in the old form, its segments separated by
                // `.`, names a module once each `.` is a `/`.
                if let Some(slashed) = slashed_module(modules, &shown) {
                    let note = format!(
                        "module paths separate their segments with `/`: `{slashed}` names a module"
                    );
                    return Err(Some(
                        Diagnostic::error(message, written)
                            .with_note(note)
                            .with_suggestion(Some((written, slashed))),
                    ));
                }
                let suggestion = named
                    .get(position)
                    .filter(|_| !nested)
                    .and_then(|&(_, span)| {
                        let only = shown.len() == 1;
                        let best = module_suggestion(modules, &at, (last, only), name)?;
                        Some((span, best))
                    });
                return Err(Some(
                    Diagnostic::error(message, written)
                        .with_note(note)
                        .with_suggestion(suggestion),
                ));
            }
        }
    }
    Err(None)
}

/// The path `shown`, which names no module, written with each `.` of its
/// segments a `/` instead, if that names a directory or a file: the module
/// a path in the old form, `lib.x`, meant (see the parser's
/// [Imports](crate::parser#imports)).
fn slashed_module(modules: &Modules<'_>, shown: &[&str]) -> Option<String> {
    let joined = shown.join("/");
    if !joined.contains('.') {
        return None;
    }
    let slashed = joined.replace('.', "/");
    let names = modules.dirs.contains_key(&slashed)
        || modules
            .files
            .contains_key(format!("{slashed}.gcsl").as_str());
    names.then_some(slashed)
}

/// The suggestion for `name`, a segment of a module path that names no
/// module in the directory at path `at` (`""`, the root), the path's last
/// segment if `last` and its only one if `only` (see
/// [Suggestions](crate::check#suggestions)): the nearest of the directories
/// in `at` (none a workspace of its own), the stems of its files if `last`
/// (a file has no modules in it), and, for a path of one segment, the
/// standard modules, `builtins` and `fileroot`.
fn module_suggestion(
    modules: &Modules<'_>,
    at: &str,
    (last, only): (bool, bool),
    name: &str,
) -> Option<String> {
    let prefix = if at.is_empty() {
        String::new()
    } else {
        format!("{at}/")
    };
    let child = |path: &str| -> Option<String> {
        let rest = path.strip_prefix(prefix.as_str())?;
        (!rest.is_empty() && !rest.contains('/')).then(|| rest.to_owned())
    };
    let mut candidates: Vec<(String, u8)> = Vec::new();
    for path in modules.dirs.keys() {
        if !path.starts_with(STANDARD_DIR) && !modules.nested.contains(path) {
            candidates.extend(child(path).map(|dir| (dir, 0)));
        }
    }
    if last {
        for &path in modules.files.keys() {
            if path.starts_with(STANDARD_DIR) || path == WORKSPACE_FILE {
                continue;
            }
            let stem = child(path).and_then(|file| Some(file.strip_suffix(".gcsl")?.to_owned()));
            candidates.extend(stem.map(|stem| (stem, 0)));
        }
    }
    if only {
        let standard = crate::STANDARD.iter().map(|&(stem, _)| stem);
        let modules = standard.chain([BUILTINS, Fileroot::MODULE]);
        candidates.extend(modules.map(|module| (module.to_owned(), 1)));
    }
    let usable = candidates
        .iter()
        .filter(|(text, _)| lexer::is_module_segment(text))
        .map(|(text, rank)| (text.as_str(), *rank));
    crate::suggest::best(name, usable).map(str::to_owned)
}

/// The name of a workspace's root file, which no path imports.
const WORKSPACE_FILE: &str = "workspace.gcsl";

/// Gives a cell to the value of each value file (one whose outermost block
/// ends in a result) that some file imports whole ([`Scan::value_slot`]).
fn value_cells(scan: &mut Scan) {
    let mut imported: Vec<u32> = scan
        .files
        .iter()
        .flat_map(|file| &file.aliases)
        .filter_map(|alias| match alias.target {
            Target::File(file) => Some(file),
            _ => None,
        })
        .filter(|&file| scan.files.get(file as usize).is_some_and(is_value_file))
        .collect();
    imported.sort_unstable();
    imported.dedup();
    for file in imported {
        let stem = scan.files.get(file as usize).map_or("", |f| {
            let name = f.path.rsplit('/').next().unwrap_or("");
            name.strip_suffix(".gcsl").unwrap_or(name)
        });
        let name = scan.names.intern(stem);
        scan.values.push((file, name));
    }
}

/// Whether `file` is a value file: its outermost block ends in a result.
fn is_value_file(file: &File) -> bool {
    file.ast
        .items(file.ast.root())
        .last()
        .is_some_and(|&item| matches!(file.ast.item(item).kind, ItemKind::Result(_)))
}

/// What directory `dir` holds, as a "no module" note lists it: its
/// directories with `/`, then its files' stems, each in path order.
fn contents(scan: &Scan, dir: Option<u32>) -> String {
    let Some(dir) = dir else {
        return "nothing".to_owned();
    };
    let last = |path: &str| path.rsplit('/').next().unwrap_or("").to_owned();
    let mut subdirs: Vec<String> = (0u32..)
        .zip(&scan.dirs)
        .filter(|(_, d)| d.parent == Some(dir))
        .map(|(_, d)| format!("`{}/`", last(&d.path)))
        .collect();
    subdirs.sort_unstable();
    let stems = scan
        .files
        .iter()
        .filter(|file| file.dir == dir)
        .map(|file| format!("`{}`", last(&file.path).trim_end_matches(".gcsl")));
    subdirs.extend(stems);
    if subdirs.is_empty() {
        "nothing".to_owned()
    } else {
        subdirs.join(", ")
    }
}

/// Binds the names the import `item` of `file`, `decl`, makes of `target`,
/// what its path names, into `aliases`, each implicit if the item is the
/// implicit import of the prelude, and the types it imports, by member or
/// `*`, into `types` (see [Types](#types)): a member that is both a root
/// and a type binds both, and one that is neither is "`lib` has no `x`".
fn members(
    scan: &Scan,
    (file, item, decl): (&File, ItemId, &ImportDecl),
    target: Target,
    (aliases, types): (&mut Vec<Alias>, &mut Vec<TypeImport>),
    errors: &mut Vec<Diagnostic>,
) {
    let path = file.ast.path(decl.path);
    let implicit = file.ast.implicit_import() == Some(item);
    match decl.form {
        ImportForm::Whole(alias) => {
            let bound = alias.map(|a| (a.name, a.span)).or_else(|| {
                let last = path.last()?;
                Some((last.name()?, last.span))
            });
            if let Some((name, span)) = bound {
                aliases.push(Alias {
                    name,
                    span,
                    item: Some(item),
                    target,
                    whole: true,
                    implicit,
                    inherited: false,
                });
            }
        }
        ImportForm::Members(range) if target == Target::Fileroot(None) => {
            for member in file.ast.import_members(range) {
                errors.push(fileroot_member_import(scan, *member));
            }
        }
        ImportForm::Members(range) => {
            for member in file.ast.import_members(range) {
                match *member {
                    ImportMember::Named { name, alias } => {
                        let bound = alias.unwrap_or(name);
                        let child = scan.target_child(target, name.name);
                        let ty = scan.target_type(target, name.name);
                        if let Some(child) = child {
                            aliases.push(Alias {
                                name: bound.name,
                                span: bound.span,
                                item: Some(item),
                                target: child,
                                whole: false,
                                implicit,
                                inherited: false,
                            });
                        }
                        if let Some(decl) = ty {
                            types.push(TypeImport {
                                name: bound.name,
                                span: bound.span,
                                decl,
                            });
                        }
                        if child.is_none() && ty.is_none() {
                            let text = scan.names.get(name.name);
                            let suggestion = member_suggestion(scan, target, text)
                                .map(|best| (name.span, best.to_owned()));
                            errors.push(
                                Diagnostic::error(
                                    format!("`{}` has no `{text}`", scan.show_target(target)),
                                    name.span,
                                )
                                .with_suggestion(suggestion),
                            );
                        }
                    }
                    ImportMember::All(star) => {
                        if let Target::File(of) = target
                            && scan.files.get(of as usize).is_some_and(is_value_file)
                        {
                            errors.push(Diagnostic::error(
                                "a value file has no members to import",
                                star,
                            ));
                            continue;
                        }
                        for root in target_roots(scan, target) {
                            if let Some(child) = scan.target_child(target, root) {
                                aliases.push(Alias {
                                    name: root,
                                    span: star,
                                    item: Some(item),
                                    target: child,
                                    whole: false,
                                    implicit,
                                    inherited: false,
                                });
                            }
                        }
                        for decl in scan.target_types(target) {
                            if let Some(name) = scan.type_name(decl) {
                                types.push(TypeImport {
                                    name,
                                    span: star,
                                    decl,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The suggestion for `name`, imported by member from `target`, which has
/// no root or type of that name (see
/// [Suggestions](crate::check#suggestions)): the nearest of its roots and
/// its types.
fn member_suggestion<'s>(scan: &'s Scan, target: Target, name: &str) -> Option<&'s str> {
    let roots = target_roots(scan, target);
    let types = scan.target_types(target);
    let candidates = roots
        .into_iter()
        .chain(types.into_iter().filter_map(|decl| scan.type_name(decl)))
        .map(|candidate| scan.names.get(candidate))
        .filter(|text| lexer::is_identifier(text))
        .map(|text| (text, 0));
    crate::suggest::best(name, candidates)
}

/// The error of `member`, a member or the `*` of an import from the
/// `fileroot` module, whose members are read as `fileroot.NAME` only, after
/// a whole import of it (see the module documentation, "The standard
/// library"): "`fileroot`'s members are read as `fileroot.caller`", naming
/// the member imported, `caller` for a `*`; or, for a name that is no
/// member, "`fileroot` has no `nope`", as for any module.
fn fileroot_member_import(scan: &Scan, member: ImportMember) -> Diagnostic {
    let (read_as, at) = match member {
        ImportMember::Named { name, .. } => {
            let text = scan.names.get(name.name);
            // No suggestion: no member of `fileroot` can be imported.
            let Some(read_as) = Fileroot::named(text) else {
                return Diagnostic::error(
                    format!("`{}` has no `{text}`", Fileroot::MODULE),
                    name.span,
                )
                .with_note(crate::check::FILEROOT_MEMBERS);
            };
            (read_as, name.span)
        }
        ImportMember::All(star) => (Fileroot::Caller, star),
    };
    Diagnostic::error(
        format!(
            "`{module}`'s members are read as `{module}.{}`",
            read_as.name(),
            module = Fileroot::MODULE
        ),
        at,
    )
    .with_note("import the module whole, `import fileroot`, and read each member through it")
}

/// The names of the roots of `target`, a directory or a file, in order.
pub(crate) fn target_roots(scan: &Scan, target: Target) -> Vec<Name> {
    match target {
        Target::Dir(dir) => scan
            .nodes
            .iter()
            .filter(|node| node.dir == dir && node.parent.is_none())
            .map(|node| node.name)
            .collect(),
        Target::File(file) => scan.files.get(file as usize).map_or_else(Vec::new, |f| {
            f.mentions
                .tree
                .roots
                .iter()
                .filter_map(|&root| f.mentions.tree.nodes.get(root as usize).map(|n| n.name))
                .collect()
        }),
        Target::Part { .. } | Target::Node(_) | Target::View(_) | Target::Fileroot(_) => Vec::new(),
    }
}

impl Scan {
    /// The types of `target`, a directory or a file, as indices in
    /// [`Scan::types`], in order: a directory's, each the first of its name
    /// among its files', and a file's own. Any other target has none.
    pub(crate) fn target_types(&self, target: Target) -> Vec<u32> {
        match target {
            Target::Dir(dir) => self.dir_types.get(dir as usize).cloned(),
            Target::File(file) => self.files.get(file as usize).map(|f| f.types.clone()),
            Target::Part { .. } | Target::Node(_) | Target::View(_) | Target::Fileroot(_) => None,
        }
        .unwrap_or_default()
    }

    /// The type of `target` named `name`, as an index in [`Scan::types`],
    /// if it has one ([`Scan::target_types`]).
    fn target_type(&self, target: Target, name: Name) -> Option<u32> {
        self.target_types(target)
            .into_iter()
            .find(|&decl| self.type_name(decl) == Some(name))
    }

    /// The name the type `decl`, an index in [`Scan::types`], declares.
    fn type_name(&self, decl: u32) -> Option<Name> {
        self.types.get(decl as usize).map(|declared| declared.name)
    }

    /// The name of the type `decl`, an index in [`Scan::types`], as text.
    fn show_type(&self, decl: u32) -> &str {
        self.type_name(decl).map_or("", |name| self.names.get(name))
    }

    /// What each type expression of file `index` that reads a type by its
    /// name names in it, in order (see [Types](#types)): a name of a type of
    /// its view, that type's declaration, by its number in
    /// [`Scan::type_store`]; a qualified name, the type it names through a
    /// whole import, or the error that it names none
    /// ([`Scan::qualified_type`]); either with as many type arguments as the
    /// type has type parameters, none for one that is not generic, or else
    /// the error ([`Scan::applied_link`]); a name of a module the
    /// file imports whole that names no type, "`t` is a module, not a type".
    /// A name of no type, a built-in type's or an unknown one, has none:
    /// the checker reads it.
    fn type_links(&self, index: u32) -> Vec<(TypeExprId, TypeLink)> {
        let Some(file) = self.files.get(index as usize) else {
            return Vec::new();
        };
        let mut links = Vec::new();
        for (id, read) in mentions::type_reads(&file.ast) {
            let span = file.ast.type_expr(id).span;
            let link = match read {
                TypeRead::Named(name) => match file.type_view.get(&name) {
                    Some(&decl) => self.applied_link(decl, (self.names.get(name), 0), span),
                    None if file.aliases.iter().any(|a| a.whole && a.name == name) => {
                        let text = self.names.get(name);
                        TypeLink::Error(Diagnostic::error(
                            format!("`{text}` is a module, not a type"),
                            span,
                        ))
                    }
                    None => continue,
                },
                TypeRead::Applied(name, given) => match file.type_view.get(&name) {
                    Some(&decl) => self.applied_link(decl, (self.names.get(name), given), span),
                    None => continue,
                },
                TypeRead::Path(path) | TypeRead::PathApplied(path, _) => {
                    let given = match read {
                        TypeRead::PathApplied(_, given) => given,
                        _ => 0,
                    };
                    let path = file.ast.path(path);
                    // Through a name the file reads as `Error`, the
                    // checker's, quietly (see [Scan errors](#scan-errors)).
                    let first = path.first().and_then(PathSegment::name);
                    if first.is_some_and(|name| file.poisoned.binary_search(&name).is_ok()) {
                        continue;
                    }
                    match self.qualified_type(file, path, span) {
                        TypeLink::Decl(decl) => {
                            let shown: Vec<&str> = path
                                .iter()
                                .filter_map(PathSegment::name)
                                .map(|name| self.names.get(name))
                                .collect();
                            let shown = shown.join(".");
                            self.applied_link(decl, (&shown, given), span)
                        }
                        error @ TypeLink::Error(_) => error,
                    }
                }
            };
            links.push((id, link));
        }
        links
    }

    /// The link of a type expression, written at `span`, that names the
    /// declaration numbered `decl`, written `shown`, with `given` type
    /// arguments, none for a name written bare (see [Types](#types)): the declaration, if it has
    /// that many type parameters; otherwise the error, "type `Pair` needs
    /// its type arguments" for a generic one used bare, "type `Foo` takes no
    /// type argument" for another applied, and "type `Pair` takes 2 type
    /// arguments, given 1".
    fn applied_link(&self, decl: u32, (shown, given): (&str, u32), span: Span) -> TypeLink {
        let arity = self.type_store.arity(decl);
        if arity == given {
            return TypeLink::Decl(decl);
        }
        let message = match (arity, given) {
            (_, 0) => format!("type `{shown}` needs its type arguments"),
            (0, _) => format!("type `{shown}` takes no type argument"),
            (1, _) => format!("type `{shown}` takes 1 type argument, given {given}"),
            _ => format!("type `{shown}` takes {arity} type arguments, given {given}"),
        };
        let mut error = Diagnostic::error(message, span);
        if given == 0 {
            let plural = if arity == 1 { "" } else { "s" };
            error = error.with_note(format!("`{shown}` takes {arity} type argument{plural}"));
        }
        TypeLink::Error(error)
    }

    /// What the qualified type name `path`, written at `span` in `file`,
    /// names (see [Types](#types)): its first name must be a module the
    /// file imports whole (`import lib/types as t`), and its second a type
    /// of that module, and its last; a path of nothing else is resolved as a
    /// value's reference through an import is, the module's type named by
    /// the name after it. Otherwise the error: "unknown type `a.b`" for a
    /// first name that is no module the file imports, "`t` has no type
    /// `Foo`" with a note listing the module's types, and "`t.Foo` is a
    /// type, not a module" for a path that goes on after a type.
    pub(crate) fn qualified_type(&self, file: &File, path: &[PathSegment], span: Span) -> TypeLink {
        let names: Vec<Name> = path.iter().filter_map(PathSegment::name).collect();
        let shown = |count: usize| -> String {
            names
                .iter()
                .take(count)
                .map(|&name| self.names.get(name))
                .collect::<Vec<_>>()
                .join(".")
        };
        let error = |message: String| TypeLink::Error(Diagnostic::error(message, span));
        let (Some(&first), Some(&second)) = (names.first(), names.get(1)) else {
            return error(format!("unknown type `{}`", shown(names.len())));
        };
        let Some(alias) = file.aliases.iter().find(|a| a.whole && a.name == first) else {
            return error(format!("unknown type `{}`", shown(names.len())));
        };
        match self.target_type(alias.target, second) {
            Some(decl) if names.len() == 2 => TypeLink::Decl(self.type_base.saturating_add(decl)),
            Some(_) => error(format!("`{}` is a type, not a module", shown(2))),
            None => {
                let module = self.names.get(first);
                let listed: Vec<String> = self
                    .target_types(alias.target)
                    .iter()
                    .map(|&decl| format!("`{}`", self.show_type(decl)))
                    .collect();
                let note = if listed.is_empty() {
                    format!("`{module}` has no types")
                } else {
                    format!("`{module}` has the types {}", listed.join(", "))
                };
                TypeLink::Error(
                    Diagnostic::error(
                        format!("`{module}` has no type `{}`", self.names.get(second)),
                        span,
                    )
                    .with_note(note),
                )
            }
        }
    }
}

/// Finds every `type` declaration of the scan's files ([`Scan::types`],
/// [`File::types`]) and each directory's types, and gives each its number
/// in the scan's store, after the declarations it holds already (a
/// session's earlier inputs'). A name declared in two files of one
/// directory is "type `Foo` is declared in two files", at the later in path
/// order, labelled "declared here" there and at the earlier, as two leaves
/// of one path are. A declaration named like a built-in type, and a second
/// of one name in one file, are left out, for the checker to report
/// ([`crate::check::check_types`]).
fn declare_types(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    let mut types: Vec<DeclaredType> = Vec::new();
    let mut own: Vec<Vec<u32>> = Vec::with_capacity(scan.files.len());
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut mine: Vec<u32> = Vec::new();
        let mut seen: HashSet<Name> = HashSet::new();
        for decl in &file.mentions.types {
            let builtin = BUILTIN_TYPE_NAMES.contains(&scan.names.get(decl.name));
            if builtin || !seen.insert(decl.name) {
                continue;
            }
            mine.push(count(types.len()));
            types.push(DeclaredType {
                file: index,
                item: decl.item,
                name: decl.name,
                span: decl.span,
                ty: decl.ty,
            });
        }
        own.push(mine);
    }
    let mut dir_types: Vec<Vec<u32>> = vec![Vec::new(); scan.dirs.len()];
    let mut first: HashMap<(u32, Name), u32> = HashMap::new();
    let mut poisoned = Vec::new();
    for (at, decl) in (0u32..).zip(&types) {
        let Some(dir) = scan.files.get(decl.file as usize).map(|file| file.dir) else {
            continue;
        };
        let earlier_at = first.get(&(dir, decl.name)).copied();
        match earlier_at.and_then(|e| types.get(e as usize)) {
            None => {
                first.insert((dir, decl.name), at);
                if let Some(list) = dir_types.get_mut(dir as usize) {
                    list.push(at);
                }
            }
            Some(earlier) => {
                let shown = scan.names.get(decl.name);
                errors.push(
                    Diagnostic::error(
                        format!("type `{shown}` is declared in two files"),
                        decl.span,
                    )
                    .with_label("declared here")
                    .with_context_in(earlier.file, earlier.span, "declared here")
                    .in_input(decl.file),
                );
                // Both are erroneous, so nothing that uses either is
                // checked against the wrong one (see [Scan
                // errors](#scan-errors)).
                poisoned.extend(earlier_at);
                poisoned.push(at);
            }
        }
    }
    for (file, mine) in scan.files.iter_mut().zip(own) {
        file.types = mine;
    }
    scan.poisoned_types = poisoned;
    scan.type_base = scan.type_store.reserve(types.len());
    for (at, decl) in (0u32..).zip(&types) {
        if let Some(file) = scan.files.get(decl.file as usize) {
            let arity = file.ast.type_arity(decl.item);
            scan.type_store
                .set_arity(scan.type_base.saturating_add(at), arity);
        }
    }
    // Those marked `#[attribute]`, by their numbers, but one named like a
    // built-in attribute.
    for (at, decl) in (0u32..).zip(&types) {
        let Some(file) = scan.files.get(decl.file as usize) else {
            continue;
        };
        if !file.ast.is_attribute(decl.item) {
            continue;
        }
        let name = scan.names.get(decl.name);
        if file.ast.type_arity(decl.item) > 0 {
            errors.push(
                Diagnostic::error(
                    format!("attribute `{name}` takes no type parameters"),
                    decl.span,
                )
                .in_input(decl.file),
            );
            continue;
        }
        if BUILTIN_ATTRIBUTES.contains(&name) {
            errors.push(
                Diagnostic::error(format!("`{name}` is a built-in attribute"), decl.span)
                    .in_input(decl.file),
            );
            continue;
        }
        let module = file.path.strip_suffix(".gcsl").unwrap_or(&file.path);
        let id = AttributeId::Declared {
            module: Rc::from(module),
            name: Rc::from(name),
        };
        scan.type_store
            .mark_attribute(scan.type_base.saturating_add(at), id);
    }
    scan.types = types;
    scan.dir_types = dir_types;
}

/// Where one name of a file's view of types comes from, as [`type_views`]
/// meets it: its declaration, by its index in [`Scan::types`], and whether
/// it reaches the view through an import, written at a span of the file,
/// or from a directory of the file's chain, a directory above its own
/// (`above`) or its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TypeSource {
    /// The declaration.
    decl: u32,
    /// Where the file imports it, if it does.
    import: Option<Span>,
    /// Whether it is a type of a directory above the file's own.
    above: bool,
}

/// Gives each file its view of types ([`File::type_view`]; see
/// [Types](#types)), merged from its sources, in the order fields' views
/// report conflicts in: its own directory's types, then each directory's
/// upward, then the types it imports, in source order; and, for a
/// session's input, the types it inherits from the earlier inputs, last,
/// shadowed by anything. One declaration reached twice is one type. Two
/// declarations of one name are "type `Foo` is declared in two files", as
/// two leaves of one path are, once per pair however many views have it,
/// at the later source, labelled "declared here" at each declaration and
/// "imported here" at each import, with the note "one is in a directory
/// above" when one is. A type imported under a built-in type's name is
/// "`Int` is a built-in type", at the import.
fn type_views(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    type Side = (u32, Option<(u32, u32)>);
    let mut reported: HashSet<(Side, Side)> = HashSet::new();
    let mut found = Vec::with_capacity(scan.files.len());
    for (index, file) in (0u32..).zip(&scan.files) {
        let mut sources: Vec<(Name, TypeSource)> = Vec::new();
        for (position, dir) in scan.chain(file.dir).enumerate() {
            for &decl in scan.dir_types.get(dir as usize).into_iter().flatten() {
                if let Some(name) = scan.type_name(decl) {
                    let source = TypeSource {
                        decl,
                        import: None,
                        above: position > 0,
                    };
                    sources.push((name, source));
                }
            }
        }
        for import in &file.type_imports {
            let source = TypeSource {
                decl: import.decl,
                import: Some(import.span),
                above: false,
            };
            sources.push((import.name, source));
        }
        let mut view: HashMap<Name, TypeSource> = HashMap::new();
        // The names two declarations conflict on, which the file reads as
        // `Error` (see [Scan errors](#scan-errors)).
        let mut conflicting: Vec<Name> = Vec::new();
        for (name, source) in sources {
            if let Some(at) = source.import
                && BUILTIN_TYPE_NAMES.contains(&scan.names.get(name))
            {
                let text = scan.names.get(name);
                errors.push(
                    Diagnostic::error(format!("`{text}` is a built-in type"), at).in_input(index),
                );
                continue;
            }
            match view.get(&name) {
                None => {
                    view.insert(name, source);
                }
                Some(earlier) if earlier.decl == source.decl => {}
                Some(&earlier) => {
                    let side = |s: TypeSource| (s.decl, s.import.map(|span| (index, span.start)));
                    if reported.insert((side(source), side(earlier)))
                        && let Some(error) = type_conflict(scan, index, name, (source, earlier))
                    {
                        errors.push(error);
                    }
                    conflicting.push(name);
                }
            }
        }
        for name in &conflicting {
            view.remove(name);
        }
        let mut numbered: HashMap<Name, u32> = view
            .into_iter()
            .map(|(name, source)| (name, scan.type_base.saturating_add(source.decl)))
            .collect();
        if index >= scan.standard {
            for &(name, decl) in &scan.inherited_types {
                numbered.entry(name).or_insert(decl);
            }
        }
        found.push((numbered, conflicting));
    }
    for (file, (view, conflicting)) in scan.files.iter_mut().zip(found) {
        file.type_view = view;
        for name in conflicting {
            file.poison(name);
        }
    }
}

/// The conflict in file `index`'s view of types of the type `name` between
/// the later source and the earlier one (see [`type_views`]).
fn type_conflict(
    scan: &Scan,
    index: u32,
    name: Name,
    (later, earlier): (TypeSource, TypeSource),
) -> Option<Diagnostic> {
    let later_decl = scan.types.get(later.decl as usize)?;
    let earlier_decl = scan.types.get(earlier.decl as usize)?;
    let message = format!("type `{}` is declared in two files", scan.names.get(name));
    let mut error = match later.import {
        None => Diagnostic::error(message, later_decl.span)
            .with_label("declared here")
            .in_input(later_decl.file),
        Some(at) => context_in(
            Diagnostic::error(message, at)
                .with_label("imported here")
                .in_input(index),
            later_decl.file,
            later_decl.span,
            "declared here",
        ),
    };
    if let Some(at) = earlier.import {
        error = context_in(error, index, at, "imported here");
    }
    error = context_in(error, earlier_decl.file, earlier_decl.span, "declared here");
    if later.above || earlier.above {
        error = error.with_note("one is in a directory above");
    }
    Some(error)
}

/// Defines the scan's types and expands every file's (see [Types](#types)):
/// copies each declaration's definition into the store, each name in it of
/// another type, or qualified, as its file's view resolves it
/// ([`Scan::type_links`]); reports each cycle of types, "type `A` refers to
/// itself" ([`type_cycle`]); settles every declaration, those it refers to
/// first, those on a cycle or poisoned ([`Scan::poisoned_types`]) erroneous
/// ([`TypeStore::settle`]); and expands each file's tree with them
/// ([`Ast::expand_types`]), a use of an erroneous one erroneous, its check
/// failing quietly. A scan with no type to define and nothing that
/// reads one costs one look at each file's type expressions.
fn define_types(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
    let links: Vec<Vec<(TypeExprId, TypeLink)>> = (0..count(scan.files.len()))
        .map(|index| scan.type_links(index))
        .collect();
    let base = scan.type_base;
    for (at, decl) in (0u32..).zip(&scan.types) {
        let (Some(file), Some(links)) = (
            scan.files.get(decl.file as usize),
            links.get(decl.file as usize),
        ) else {
            continue;
        };
        scan.type_store
            .declare(base.saturating_add(at), (&file.ast, decl.ty), links);
    }
    // What each declaration refers to of the scan's own: the store's
    // earlier declarations, a session's earlier inputs', are settled.
    let edges: Vec<Vec<u32>> = (0..count(scan.types.len()))
        .map(|at| {
            scan.type_store
                .refs(base.saturating_add(at))
                .into_iter()
                .filter_map(|(to, _)| to.checked_sub(base))
                .filter(|&to| (to as usize) < scan.types.len())
                .collect()
        })
        .collect();
    let slices: Vec<&[u32]> = edges.iter().map(Vec::as_slice).collect();
    let (components, _) = mentions::components(&slices);
    for component in &components {
        let cyclic = component.len() > 1
            || component
                .first()
                .is_some_and(|&only| edges.get(only as usize).is_some_and(|e| e.contains(&only)));
        if cyclic && let Some(error) = type_cycle(scan, component, &edges) {
            errors.push(error);
        }
        for &member in component {
            let poisoned = scan.poisoned_types.contains(&member);
            scan.type_store
                .settle(base.saturating_add(member), cyclic || poisoned);
        }
    }
    // A declaration on a cycle, or poisoned, is erroneous, and so is each
    // use of it: the trees are expanded whatever was found (see [Scan
    // errors](#scan-errors)).
    for (file, links) in scan.files.iter_mut().zip(&links) {
        file.ast.expand_types(&scan.type_store, links);
    }
}

/// The error that the types of `component`, indices in [`Scan::types`]
/// that refer to each other, `edges` being what each refers to, refer to
/// themselves: "type `A` refers to itself", at the first of them in path
/// then source order, labelled at each step of a shortest cycle through it
/// ("`B` uses `A` here", at the name of `A` in `B`'s definition).
fn type_cycle(scan: &Scan, component: &[u32], edges: &[Vec<u32>]) -> Option<Diagnostic> {
    let first = component.iter().copied().min()?;
    // A shortest cycle through `first`, breadth first within the component.
    let mut previous: HashMap<u32, u32> = HashMap::new();
    let mut queue = std::collections::VecDeque::from([first]);
    let mut closing = None;
    while let Some(at) = queue.pop_front() {
        for &to in edges.get(at as usize).into_iter().flatten() {
            if to == first {
                closing = Some(at);
                break;
            }
            if component.contains(&to) && !previous.contains_key(&to) {
                previous.insert(to, at);
                queue.push_back(to);
            }
        }
        if closing.is_some() {
            break;
        }
    }
    let mut at = closing?;
    let mut steps = vec![(at, first)];
    while at != first {
        let from = *previous.get(&at)?;
        steps.push((from, at));
        at = from;
    }
    steps.reverse();
    let decl = scan.types.get(first as usize)?;
    let mut error = Diagnostic::error(
        format!("type `{}` refers to itself", scan.show_type(first)),
        decl.span,
    )
    .in_input(decl.file);
    for (from, to) in steps {
        let from_decl = scan.types.get(from as usize)?;
        let target = scan.type_base.saturating_add(to);
        let at = scan
            .type_store
            .refs(scan.type_base.saturating_add(from))
            .into_iter()
            .find(|&(refers, _)| refers == target)
            .map(|(_, at)| at)?;
        let (from_name, to_name) = (scan.show_type(from), scan.show_type(to));
        let label = format!("`{from_name}` uses `{to_name}` here");
        error = context_in(error, from_decl.file, at, &label);
    }
    Some(error)
}

/// Reports each name of `aliases`, the imports of file `index`, that
/// clashes: a whole import with another import, a top-level `fn`, `let`,
/// `let mut`, `:=` or field root of the file, or a root of its directory
/// chain; a member or a root of a `*`, which merges into the file's view
/// with the file's fields, its directory chain and the other members, with
/// a whole import or a top-level `fn`, `let`, `let mut` or `:=`. The clash
/// family ignores what is shadowable (see the module documentation,
/// "Shadowable declarations"): an alias that is (the implicit prelude's
/// among them), and those a session's input inherits, clash with nothing,
/// and a whole import named like a field root of the file or of its
/// directory chain all of whose leaves are shadowable shadows it silently
/// ([`whole_imports_shadow`]). Each name that clashes is added to
/// `poisoned`, read as `Error` in the file (see [Scan errors](#scan-errors)).
fn clashes(
    scan: &Scan,
    index: u32,
    aliases: &[Alias],
    (errors, poisoned): (&mut Vec<Diagnostic>, &mut Vec<Name>),
) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    // Whether an alias clashes with nothing: inherited, or shadowable, by
    // its import's mark or, for a member, by what it brings.
    let quiet = |alias: &Alias| {
        alias.inherited
            || file.marked(alias)
            || (!alias.whole && scan.target_shadowable(alias.target))
    };
    if aliases.iter().all(quiet) {
        return;
    }
    let ast = &file.ast;
    let tree = &file.mentions.tree;
    // The file's own top-level names, first declaration of each, and
    // whether it is a field's, but a field root all of whose leaves are
    // shadowable, and a declaration a later one shadows.
    let mut declared: Vec<(Name, Span, bool)> = Vec::new();
    let mut seen: HashSet<(Name, bool)> = HashSet::new();
    for (position, &item) in ast.items(ast.root()).iter().enumerate() {
        let (field, path) = match ast.item(item).kind {
            ItemKind::Let { path, .. } | ItemKind::Rebind { path, .. } => (false, path),
            ItemKind::Field { path, .. } => (true, path),
            _ => continue,
        };
        if file.mentions.shadowed.get(position) == Some(&true) {
            continue;
        }
        if let Some(first) = ast.path(path).first()
            && let Some(name) = first.name()
            && !(field
                && tree
                    .root(name)
                    .and_then(|root| tree.nodes.get(root as usize))
                    .is_some_and(|root| root.shadowable))
            && seen.insert((name, field))
        {
            declared.push((name, first.span, field));
        }
    }
    let star = |alias: &Alias| {
        let Some(ItemKind::Import(id)) = alias.item.map(|item| ast.item(item).kind) else {
            return false;
        };
        matches!(ast.import_decl(id).form, ImportForm::Members(range)
            if ast.import_members(range).iter().any(|m| m == &ImportMember::All(alias.span)))
    };
    // A shadowable import clashes with nothing: its names yield to every
    // declaration of theirs (see the module documentation, "Shadowable
    // declarations"). Nor does an inherited import, which the input's
    // declarations shadow ([`inherit_imports`]).
    for (position, alias) in aliases.iter().enumerate() {
        if quiet(alias) {
            continue;
        }
        let shown = scan.names.get(alias.name);
        let earlier = aliases
            .get(..position)
            .and_then(|before| {
                before
                    .iter()
                    .find(|a| !quiet(a) && a.name == alias.name && (alias.whole || a.whole))
            })
            .map(|a| (a.span, "imported here"));
        let other = earlier.or_else(|| {
            declared
                .iter()
                .filter(|&&(n, _, field)| n == alias.name && (alias.whole || !field))
                .min_by_key(|&&(_, span, _)| span.start)
                .map(|&(_, span, _)| (span, "declared here"))
        });
        if let Some((span, label)) = other {
            let message = format!("`{shown}` is imported in this file");
            errors.push(if star(alias) || label == "imported here" {
                Diagnostic::error(message, alias.span).with_context(span, label)
            } else {
                Diagnostic::error(message, span).with_context(alias.span, "imported here")
            });
            poisoned.push(alias.name);
            continue;
        }
        let above = scan
            .chain(file.dir)
            .find_map(|dir| scan.by_name.get(&(dir, ROOT, alias.name)));
        if alias.whole && above.is_some_and(|&node| !scan.target_shadowable(Target::Node(node))) {
            errors.push(Diagnostic::error(
                format!("`{shown}` is a field of this directory"),
                alias.span,
            ));
            poisoned.push(alias.name);
        }
    }
}

/// The error that file `index` reaches `target`, which it computes itself,
/// through an import at `at`: the one-file form of a cycle.
fn needs_itself(scan: &Scan, index: u32, target: Target, at: Span) -> Diagnostic {
    let label = format!(
        "`{}` is computed when this file runs",
        scan.show_target(target)
    );
    needs_itself_because(scan, index, label, at)
}

/// The error that file `index` needs itself through the reference at `at`,
/// with `label` saying why.
fn needs_itself_because(scan: &Scan, index: u32, label: String, at: Span) -> Diagnostic {
    let path = scan
        .files
        .get(index as usize)
        .map_or("", |f| f.path.as_str());
    Diagnostic::error(format!("`{path}` needs itself"), at)
        .with_label(label)
        .with_note(
            "a file runs as a whole before a file that needs it, so a file cannot need itself",
        )
        .in_input(index)
}

/// Reports each whole read of a directory through an import (`whole_dirs`:
/// the reading file, the directory's files, the target, the reference) one
/// of whose files needs the reading file back, in the needs `all` found so
/// far: the one-file form of a cycle too, "`app.gcsl` needs itself", at the
/// read, once per file (`itselves` saying which files have had theirs). The
/// needs that would close a cycle of two files with it are dropped, so that
/// the cycle is reported once.
fn whole_reads_needing_back(
    scan: &Scan,
    whole_dirs: Vec<(u32, Vec<u32>, Target, Span)>,
    all: &mut [HashMap<u32, (Span, String)>],
    itselves: &mut [bool],
    errors: &mut Vec<Diagnostic>,
) {
    for (index, owners, target, span) in whole_dirs {
        let back: Vec<u32> = owners
            .into_iter()
            .filter(|&o| all.get(o as usize).is_some_and(|f| f.contains_key(&index)))
            .collect();
        if back.is_empty() {
            continue;
        }
        if let Some(itself) = itselves.get_mut(index as usize)
            && !*itself
        {
            *itself = true;
            let file = back
                .first()
                .and_then(|&o| scan.files.get(o as usize))
                .map_or("", |f| f.path.as_str());
            let label = format!(
                "`{}` is read whole, and its file `{file}` needs this file",
                scan.show_target(target)
            );
            errors.push(needs_itself_because(scan, index, label, span));
        }
        if let Some(first) = all.get_mut(index as usize) {
            for owner in back {
                first.remove(&owner);
            }
        }
    }
}

/// The files a whole read by file `index` of its view node `view` needs:
/// every owner of every leaf under it, in whichever directory or import,
/// but `index` itself, whose own leaves are computed in its block; with the
/// node of an import that reaches into `index`, if one does, which is a
/// cycle of one file, as a reach through a whole import is.
fn view_owners(scan: &Scan, index: u32, view: u32) -> (Vec<u32>, Option<Target>) {
    let mut owners = Vec::new();
    let mut reached = None;
    let parts = scan.views.get(view as usize).map_or(&[][..], |v| &v.parts);
    for &part in parts {
        let files = scan.part_owners(part);
        if !part.chain && files.contains(&index) {
            reached.get_or_insert(part.target);
        } else {
            owners.extend(files.into_iter().filter(|&file| file != index));
        }
    }
    (owners, reached)
}

/// The field nodes each item of `file`'s outermost block mentions, with its
/// position, but a shadowed declaration's, which needs nothing and reads
/// nothing whole computed for it (see the module documentation,
/// "Shadowable declarations").
fn needing_items(file: &File) -> impl Iterator<Item = (usize, &Vec<u32>)> {
    let shadowed = &file.mentions.shadowed;
    file.mentions
        .fields
        .iter()
        .enumerate()
        .filter(move |&(position, _)| shadowed.get(position) != Some(&true))
}

/// Fills [`Scan::needs`], and returns for each file the first mention, in
/// source order, of a declaration of each file it needs, with what it
/// shows.
fn find_needs(scan: &mut Scan, errors: &mut Vec<Diagnostic>) -> Vec<HashMap<u32, (Span, String)>> {
    let mut all = Vec::with_capacity(scan.files.len());
    let mut itselves = Vec::with_capacity(scan.files.len());
    // Each whole read of a directory through an import: the reading file,
    // the directory's files, the target and the reference.
    let mut whole_dirs: Vec<(u32, Vec<u32>, Target, Span)> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files) {
        let scope = FileScope::new(scan, file);
        let mut itself = false;
        let mut first: HashMap<u32, (Span, String)> = HashMap::new();
        let mut record = |owner: u32, span: Span, shown: &dyn Fn() -> String| {
            if owner == index {
                return;
            }
            let entry = first.entry(owner).or_insert_with(|| (span, shown()));
            if span.start < entry.0.start {
                *entry = (span, shown());
            }
        };
        for (position, fields) in needing_items(file) {
            let spans = file.mentions.field_spans.get(position);
            let foreign = file.mentions.foreign.get(position).into_iter().flatten();
            let mut nodes: Vec<(u32, Span)> = Vec::new();
            // The nodes of the view it reads whole, each with the reference.
            let mut views: Vec<(u32, Span)> = Vec::new();
            for (&n, &span) in fields.iter().zip(spans.into_iter().flatten()) {
                if let Some(&global) = file.global.get(n as usize) {
                    nodes.push((global, span));
                } else if let Some(view) = scope.own_view(n) {
                    views.push((view, span));
                }
            }
            for &(need, span) in foreign {
                match need {
                    // Through an import: what it reaches needs its files,
                    // and reaching into this file is a cycle of one file.
                    Need::Field(n) => {
                        let target = match scope.target(n) {
                            None => {
                                nodes.extend(scope.merged(n).map(|merged| (merged, span)));
                                continue;
                            }
                            Some(Target::View(view)) => {
                                views.push((view, span));
                                continue;
                            }
                            Some(target) => target,
                        };
                        let owners = scan.target_files(target);
                        if owners.contains(&index) {
                            if !itself {
                                itself = true;
                                errors.push(needs_itself(scan, index, target, span));
                            }
                            continue;
                        }
                        for &owner in &owners {
                            record(owner, span, &|| scan.show_target(target));
                        }
                        if matches!(target, Target::Dir(_)) {
                            whole_dirs.push((index, owners, target, span));
                        }
                    }
                    Need::Function(f) => {
                        if let Some(function) = scope
                            .foreign_function(f)
                            .and_then(|id| scan.functions.get(id as usize))
                        {
                            record(function.file, span, &|| scan.show_function(*function));
                        }
                    }
                }
            }
            for (merged, span) in nodes {
                for owner in scan.target_files(Target::Node(merged)) {
                    record(owner, span, &|| scan.show(merged));
                }
            }
            for (view, span) in views {
                let (owners, reached) = view_owners(scan, index, view);
                if let Some(target) = reached
                    && !itself
                {
                    itself = true;
                    errors.push(needs_itself(scan, index, target, span));
                }
                for owner in owners {
                    record(owner, span, &|| scan.show_view(view));
                }
            }
        }
        all.push(first);
        itselves.push(itself);
    }
    whole_reads_needing_back(scan, whole_dirs, &mut all, &mut itselves, errors);
    // A file that needs itself is not checked (see [Scan
    // errors](#scan-errors)).
    scan.cyclic = itselves;
    scan.needs = all
        .iter()
        .map(|first| {
            let mut needs: Vec<u32> = first.keys().copied().collect();
            needs.sort_unstable();
            needs
        })
        .collect();
    all
}

/// Reports each cycle of files that need each other, once, and recovers
/// from it (see [Scan errors](#scan-errors)): each file of it is marked
/// cyclic ([`Scan::is_cyclic`]), not to be checked, and its needs of the
/// other files of it are forgotten, so that the order has every file.
fn report_cycles(
    scan: &mut Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) {
    let groups = cycle_groups(scan, mentioned, errors);
    for group in groups {
        for &file in &group {
            if let Some(entry) = scan.cyclic.get_mut(file as usize) {
                *entry = true;
            }
            if let Some(needs) = scan.needs.get_mut(file as usize) {
                needs.retain(|need| !group.contains(need));
            }
        }
    }
}

/// Reports each cycle of `scan`'s files that need each other, once, and
/// returns the files of each.
fn cycle_groups(
    scan: &Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) -> Vec<Vec<u32>> {
    let edges: Vec<&[u32]> = scan.needs.iter().map(Vec::as_slice).collect();
    let (mut groups, _) = mentions::components(&edges);
    for group in &mut groups {
        group.sort_unstable();
    }
    groups.sort();
    for group in groups.iter().filter(|group| group.len() > 1) {
        let path = |file: u32| {
            scan.files
                .get(file as usize)
                .map_or_else(String::new, |f| format!("`{}`", f.path))
        };
        let shown: Vec<String> = group.iter().map(|&f| path(f)).collect();
        let listed = match shown.split_last() {
            Some((last, rest)) if rest.len() > 1 => format!("{} and {last}", rest.join(", ")),
            Some((last, rest)) => format!("{} and {last}", rest.join("")),
            None => String::new(),
        };
        // Each file's first mention of another file of the cycle.
        let uses = group.iter().filter_map(|&file| {
            let first = mentioned.get(file as usize)?;
            let (at, what) = group
                .iter()
                .filter_map(|other| first.get(other))
                .min_by_key(|(at, _)| at.start)?;
            Some((file, *at, format!("{} uses `{what}` here", path(file))))
        });
        let mut diagnostic: Option<Diagnostic> = None;
        for (file, span, text) in uses {
            diagnostic = Some(match diagnostic {
                None => Diagnostic::error(format!("{listed} need each other"), span)
                    .with_label(text)
                    .in_input(file),
                Some(diagnostic) => diagnostic.with_context_in(file, span, text),
            });
        }
        if let Some(diagnostic) = diagnostic {
            errors.push(diagnostic.with_note(
                "a file runs as a whole before a file that needs it, so two files cannot need \
                 each other",
            ));
        }
    }
    groups.retain(|group| group.len() > 1);
    groups
}

/// The files in the order they run: repeatedly the path-smallest file all
/// of whose needs are placed. `needs` has no cycles.
fn order(needs: &[Vec<u32>]) -> Vec<u32> {
    let mut waiting: Vec<usize> = needs.iter().map(Vec::len).collect();
    let mut users: Vec<Vec<u32>> = vec![Vec::new(); needs.len()];
    for (file, list) in (0u32..).zip(needs) {
        for &need in list {
            if let Some(entry) = users.get_mut(need as usize) {
                entry.push(file);
            }
        }
    }
    let mut ready: BinaryHeap<Reverse<u32>> = (0u32..)
        .zip(&waiting)
        .filter(|&(_, &n)| n == 0)
        .map(|(file, _)| Reverse(file))
        .collect();
    let mut order = Vec::with_capacity(needs.len());
    while let Some(Reverse(file)) = ready.pop() {
        order.push(file);
        for &user in users.get(file as usize).into_iter().flatten() {
            if let Some(entry) = waiting.get_mut(user as usize) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    ready.push(Reverse(user));
                }
            }
        }
    }
    order
}

/// Finds each file's schedule, its shared interior nodes that a later
/// file in the order computes left out, and the nodes of its view with a
/// leaf of the file under them that it reads whole in: those mentioned and
/// the view nodes under them, each needing its own children (see
/// [`Mentions::views`]).
fn schedule(scan: &mut Scan) {
    let edges: Vec<(Vec<mentions::ViewNode>, Vec<bool>)> = scan
        .files
        .iter()
        .map(|file| view_edges(scan, file))
        .collect();
    let mut needs = Vec::with_capacity(edges.len());
    for (file, (views, needed)) in scan.files.iter_mut().zip(edges) {
        file.mentions.views = views;
        needs.push(needed);
    }
    let rank = &scan.rank;
    let mut schedules = Vec::with_capacity(scan.files.len());
    for ((index, file), needed) in (0u32..).zip(&scan.files).zip(needs) {
        let mut done: Vec<bool> = file
            .global
            .iter()
            .map(|&merged| {
                scan.nodes.get(merged as usize).is_some_and(|node| {
                    node.leaf.is_none()
                        && node
                            .contributors
                            .iter()
                            .max_by_key(|&&f| rank.get(f as usize).copied())
                            != Some(&index)
                })
            })
            .collect();
        done.extend(needed.iter().map(|&needed| !needed));
        schedules.push(file.mentions.schedule_except(&done));
    }
    for (file, schedule) in scan.files.iter_mut().zip(schedules) {
        file.schedule = schedule;
    }
}

/// The nodes of `file`'s view with a leaf of the file under them, as the
/// nodes of its graph of what needs what ([`Mentions::views`]), placed by
/// the file's own node of their path, with whether the file reads each
/// whole: those it mentions, and the view nodes under them. Only those
/// have their children, the file's own nodes or such view nodes, as edges:
/// finding every one's would cost every file of a directory the children of
/// its directory's nodes, which a whole read costs only the file that
/// reads them.
fn view_edges(scan: &Scan, file: &File) -> (Vec<mentions::ViewNode>, Vec<bool>) {
    let scope = FileScope::new(scan, file);
    let own = count(file.global.len());
    let start = file.view.start;
    let nodes = scan
        .views
        .get(start as usize..start.saturating_add(file.view.own) as usize)
        .unwrap_or_default();
    let mut views: Vec<mentions::ViewNode> = nodes
        .iter()
        .map(|node| {
            let tree = node
                .own
                .and_then(|own| file.mentions.tree.nodes.get(own as usize));
            mentions::ViewNode {
                name: node.name,
                parent: node
                    .parent
                    .map(|parent| own.saturating_add(parent.saturating_sub(start))),
                children: Vec::new(),
                first: tree.map_or(0, |tree| tree.first),
                last: tree.map_or(0, |tree| tree.last),
            }
        })
        .collect();
    let mut needed = vec![false; views.len()];
    let mut stack: Vec<u32> = needing_items(file)
        .flat_map(|(_, fields)| fields)
        .copied()
        .filter(|&n| scope.own_view(n).is_some())
        .collect();
    while let Some(number) = stack.pop() {
        let at = number.saturating_sub(own) as usize;
        if needed.get(at) != Some(&false) {
            continue;
        }
        let children: Vec<u32> = scope
            .view_children(number)
            .into_iter()
            .map(|(_, child)| child)
            .filter(|&child| child < scope.merged_start())
            .collect();
        stack.extend(children.iter().filter(|&&c| scope.own_view(c).is_some()));
        if let (Some(seen), Some(view)) = (needed.get_mut(at), views.get_mut(at)) {
            *seen = true;
            view.children = children;
        }
    }
    (views, needed)
}

/// What the checker knows of the workspace while it checks one of its
/// files (see [Directory scope](crate::check#directory-scope)).
#[derive(Debug)]
pub(crate) struct Context<'a> {
    /// The workspace.
    scan: &'a Scan,
    /// The file being checked.
    file: &'a File,
    /// The scope the file's references are resolved in.
    pub(crate) scope: FileScope<'a>,
    /// The file's index.
    index: u32,
    /// The type of each cell of the workspace, by its slot: `Error` until
    /// the file that computes it is checked.
    cell_types: &'a [TyId],
    /// The slot each cell of the workspace is read from, by its number: its
    /// own, but a `def`'s function's slot, known once its file is checked.
    cell_slots: &'a [Slot],
    /// For each cell that is a `def`'s, by its number, what a call of its
    /// function needs to know of its parameters, known once its file is
    /// checked ([`KnownParams`]): which are present because its body
    /// measures them, for the call-site form of the implicit-await error,
    /// and their defaults, which a call of it through an import or by its
    /// path gives the parameters it leaves unfilled.
    cell_known: &'a [Option<Rc<KnownParams>>],
    /// Whether the file is a session's input, whose scope is the session's:
    /// its variables are read as a session's input reads them, captured
    /// by a function, not read through the top level's frame as a
    /// directory's committed declarations are (see [`check_input`]).
    session: bool,
    /// Where the file is, as the driver resolved it: its directory's index
    /// in the run's table, and what `fileroot.this` and `fileroot.workspace`
    /// are in it (see the module documentation, "The standard library").
    place: Place,
}

/// What a [`Context`] reads cells by: the slot each cell is read from, and
/// for each `def`'s cell what a call of it needs to know of its parameters.
type CellReads<'a> = (&'a [Slot], &'a [Option<Rc<KnownParams>>]);

/// What a name read as an attribute's names in a file (see the parser's
/// [Attributes](crate::parser#attributes)).
#[derive(Clone, Copy, Debug)]
pub(crate) enum AttributeLookup<'a> {
    /// No type of the file's view.
    Nothing,
    /// A type of the file's view that is not marked `#[attribute]`.
    NotAttribute,
    /// A type marked `#[attribute]`, known to a host as this.
    Attribute(&'a AttributeId),
}

impl<'a> Context<'a> {
    /// What `name`, read as an attribute's, names in the file: a type of its
    /// view of types, as a type's name is read (see [Types](#types)), and
    /// whether it is marked `#[attribute]`.
    pub(crate) fn attribute(&self, name: Name) -> AttributeLookup<'a> {
        match self.file.type_view.get(&name) {
            None => AttributeLookup::Nothing,
            Some(&decl) => match self.scan.type_store.attribute(decl) {
                Some(id) => AttributeLookup::Attribute(id),
                None => AttributeLookup::NotAttribute,
            },
        }
    }

    /// The names of the attributes of the file's view of types, for
    /// suggestions.
    pub(crate) fn attribute_names(&self) -> Vec<Name> {
        let store = &self.scan.type_store;
        self.file
            .type_view
            .iter()
            .filter(|&(_, &decl)| store.attribute(decl).is_some())
            .map(|(&name, _)| name)
            .collect()
    }

    /// The context of file `index` of `scan`, where the scan located it
    /// ([`Scan::place`]).
    fn new(
        scan: &'a Scan,
        index: u32,
        cell_types: &'a [TyId],
        (cell_slots, cell_known): CellReads<'a>,
    ) -> Option<Self> {
        let file = scan.files.get(index as usize)?;
        Some(Self {
            scan,
            file,
            scope: FileScope::new(scan, file),
            index,
            cell_types,
            cell_slots,
            cell_known,
            session: false,
            place: scan.place(index),
        })
    }

    /// Whether the file is a session's input (see [`Context::session`]'s
    /// field).
    pub(crate) fn session(&self) -> bool {
        self.session
    }

    /// Where the file is (see [`Context::place`]'s field).
    pub(crate) fn place(&self) -> &Place {
        &self.place
    }

    /// Whether the file's view sees a file with syntax errors
    /// ([`Scan::sees_broken`]), in which a name it reads may be declared
    /// (see [Syntax errors](#syntax-errors)).
    pub(crate) fn sees_broken(&self) -> bool {
        self.scan.sees_broken(self.index)
    }

    /// The roots of the file's view, each with the rank of its scope for a
    /// suggestion (see [Suggestions](crate::check#suggestions)), the nearer
    /// the smaller: its own, 4; its directories', one rank each from 5 up,
    /// its own directory first; what it imports, the rank after the root
    /// directory's; the implicit prelude's, the one after that; but not the
    /// name of a whole import of `fileroot`, which is never a value. A name
    /// may come more than once.
    pub(crate) fn suggestion_roots(&self) -> Vec<(Name, u8)> {
        let tree = &self.file.mentions.tree;
        let mut found: Vec<(Name, u8)> = tree
            .roots
            .iter()
            .filter_map(|&root| tree.nodes.get(root as usize).map(|node| (node.name, 4)))
            .collect();
        let mut rank: u8 = 5;
        for dir in self.scan.chain(self.file.dir) {
            let roots = target_roots(self.scan, Target::Dir(dir));
            found.extend(roots.into_iter().map(|name| (name, rank)));
            rank = rank.saturating_add(1);
        }
        // The `fileroot` module is never a value: only its members are read.
        let values = self
            .file
            .aliases
            .iter()
            .filter(|alias| !matches!(alias.target, Target::Fileroot(_)));
        for alias in values {
            let at = u8::from(alias.implicit);
            found.push((alias.name, rank.saturating_add(at)));
        }
        found
    }

    /// The names of the types of the file's view ([`File::type_view`]).
    pub(crate) fn type_names(&self) -> impl Iterator<Item = Name> + '_ {
        self.file.type_view.keys().copied()
    }

    /// Whether the file reads any name as `Error` ([`File::poisoned`]).
    pub(crate) fn has_poison(&self) -> bool {
        !self.file.poisoned.is_empty()
    }

    /// Whether the file reads the name `name` as `Error`, silently, the
    /// scan having recovered from an error about it ([`File::poisoned`]; see
    /// [Scan errors](#scan-errors)).
    pub(crate) fn poisoned(&self, name: Name) -> bool {
        self.file.poisoned.binary_search(&name).is_ok()
    }

    /// Whether the scan of the workspace found errors, its own or syntax
    /// errors, which it recovered from: nothing runs, so a check that found
    /// an erroneous type with nothing to report, of a declaration the scan
    /// poisoned, fails quietly (see [Scan errors](#scan-errors)).
    pub(crate) fn scan_failed(&self) -> bool {
        !self.scan.recovered.is_empty() || !self.scan.syntax_errors.is_empty()
    }

    /// Whether the import item `item` of a lone file, or of a session's
    /// input, names nothing it can: a path of anything but `builtins` and
    /// the standard files, which the checker reports as needing a
    /// workspace (see the module documentation, "The standard library"); an
    /// import whose error the scan reported is not reported again.
    pub(crate) fn needs_workspace(&self, item: ItemId) -> bool {
        self.scan.lone
            && !self.file.aliases.iter().any(|a| a.item == Some(item))
            && !self.file.failed_imports.iter().any(|&(at, _)| at == item)
    }

    /// The slot of the cell numbered `cell` (see [`Scan::slot`]).
    pub(crate) fn slot(&self, cell: u32) -> Slot {
        self.scan.slot(cell)
    }

    /// What a call of the `def` whose merged node is `node` needs to know of
    /// its parameters, if `node` is a `def`'s cell of a file already checked.
    pub(crate) fn known(&self, node: u32) -> Option<&KnownParams> {
        self.cell_known.get(node as usize)?.as_deref()
    }

    /// The slot the cell of the merged node `node` is read from: its own,
    /// or for a `def` its function's (whose merged node's slot is never
    /// written).
    pub(crate) fn cell_slot(&self, node: u32) -> Slot {
        self.cell_slots
            .get(node as usize)
            .copied()
            .unwrap_or_else(|| self.scan.slot(node))
    }

    /// The cell the file's result is stored in, if it is a value file some
    /// file imports.
    pub(crate) fn value_slot(&self) -> Option<Slot> {
        self.scan.value_slot(self.index)
    }

    /// The number of the cell of the value of `target`, among the
    /// workspace's cells, if it is a value file's: its slot is [`Slot::at`]
    /// that number, and [`Context::cell_type`] of it is its type.
    pub(crate) fn value_of(&self, target: Target) -> Option<u32> {
        match target {
            Target::File(file) => self.scan.value_cell(file),
            _ => None,
        }
    }

    /// The fields of the record of `target`, a directory, a file, a part or
    /// a view node read whole: each child's name and target, in the order of
    /// the directory's or the file's tree, or the view's.
    pub(crate) fn target_fields(&self, target: Target) -> Vec<(Name, Target)> {
        let names = match target {
            Target::Part { .. } | Target::View(_) => self.scan.child_names(target),
            other => target_roots(self.scan, other),
        };
        names
            .into_iter()
            .filter_map(|name| Some((name, self.scan.target_child(target, name)?)))
            .collect()
    }

    /// Whether the file's definition whose use of its def-type is `def` is
    /// one whose directory declares no meta of its path and def-type
    /// ([`File::unpaired`]).
    pub(crate) fn unpaired(&self, def: DefTypeId) -> bool {
        self.file.unpaired.contains(&def)
    }

    /// The meta that the def-type whose namespace's key is `key` declares
    /// for its definitions, as its declaration writes it, `META->
    /// PackageMeta`, shown with `names`, the table the file being checked
    /// has: if the def-type is a `def` of a file of the scan whose definition
    /// parameter is written as a function type with a meta (see
    /// [Def-types](#def-types)). `None` otherwise, as for one a session's
    /// earlier input declares, which the scan has no tree of.
    pub(crate) fn meta_written(&self, key: Name, names: &Names) -> Option<String> {
        let (number, item) = names::key_numbers(names.get(key))?;
        // In a session's input, only the input's own def-types are its
        // file's; any other number is an earlier input's.
        let index = match self.scan.input_number {
            Some(input) if input == number => self.scan.standard,
            Some(_) => return None,
            None => number,
        };
        let ast = &self.scan.files.get(index as usize)?.ast;
        let item = ast.item_at(item)?;
        if !ast.is_def_type(item) {
            return None;
        }
        let ItemKind::Field { value, .. } = ast.item(item).kind else {
            return None;
        };
        let ExprKind::Fn(function) = ast.get(value).kind else {
            return None;
        };
        let param = ast.params(ast.fn_decl(function).params).get(1)?;
        let TypeExprKind::Fn {
            meta: Some(meta), ..
        } = ast.written_type(param.ty)
        else {
            return None;
        };
        Some(format!(
            "{} {}",
            meta.effect.meta_marker(),
            ast.show_type_with(meta.ty, names)
        ))
    }

    /// A name for the slot of the record of `target`: the name the file
    /// imports it as, or a part's or a view node's last name.
    pub(crate) fn target_name(&self, target: Target) -> Option<Name> {
        if let Some(alias) = self.file.aliases.iter().find(|a| a.target == target) {
            return Some(alias.name);
        }
        match target {
            Target::Part { file, node } => self
                .scan
                .files
                .get(file as usize)
                .and_then(|f| f.mentions.tree.nodes.get(node as usize))
                .map(|n| n.name),
            Target::View(view) => self.scan.views.get(view as usize).map(|v| v.name),
            _ => self.file.aliases.first().map(|a| a.name),
        }
    }

    /// Whether a field access in the file may read through its view rather
    /// than a binding: it imports something that has children, or several
    /// of its sources share a node (see [`FileScope::reads_through`]). The
    /// implicit `import * from prelude` of leaves alone does not count: a
    /// field access on a leaf reads the leaf's value.
    pub(crate) fn has_view(&self) -> bool {
        self.file
            .aliases
            .iter()
            .any(|a| !a.implicit || !self.scan.is_leaf(a.target))
            || self.file.view.end > self.file.view.start
    }

    /// The file being checked.
    pub(crate) fn file(&self) -> &'a File {
        self.file
    }

    /// Whether the merged node `node` has leaves in more than one file.
    pub(crate) fn shared(&self, node: u32) -> bool {
        self.scan
            .nodes
            .get(node as usize)
            .is_some_and(|entry| entry.contributors.len() > 1)
    }

    /// The type of the cell numbered `node` (a merged node, or an imported
    /// value file's value, [`Context::value_of`]), as its file's check
    /// found it.
    pub(crate) fn cell_type(&self, node: u32) -> TyId {
        self.cell_types
            .get(node as usize)
            .copied()
            .unwrap_or(TyId::ERROR)
    }

    /// The children of the file's own node `node` that other files
    /// declare: each one's name, cell and type.
    pub(crate) fn foreign_children(&self, node: u32) -> Vec<(Name, Slot, TyId)> {
        self.merged_children(node)
            .into_iter()
            .filter(|&(_, own, _, _)| own.is_none())
            .map(|(name, _, slot, ty)| (name, slot, ty))
            .collect()
    }

    /// Every child of the file's own node `node` if it is shared with
    /// other files, in the merged node's order: each one's name, the
    /// file's own node of it if it has one, its cell and, for another
    /// file's, its type. Empty for a node of the file alone.
    pub(crate) fn merged_children(&self, node: u32) -> Vec<(Name, Option<u32>, Slot, TyId)> {
        let Some(&global) = self.file().global.get(node as usize) else {
            return Vec::new();
        };
        if !self.shared(global) {
            return Vec::new();
        }
        let children = self
            .scan
            .nodes
            .get(global as usize)
            .map_or(&[][..], |n| &n.children);
        children
            .iter()
            .filter_map(|&child| {
                let entry = self.scan.nodes.get(child as usize)?;
                let own = self.scope.local.get(&child).copied();
                Some((
                    entry.name,
                    own,
                    self.scan.slot(child),
                    self.cell_type(child),
                ))
            })
            .collect()
    }
}

/// A file runs at most once. A failed attempt must remain a failure for every
/// later request that needs it, without repeating any of its earlier effects.
/// Boxing keeps this state table compact after the large IR is released.
#[derive(Debug)]
enum FileRun {
    Pending(Box<Ir>),
    Succeeded,
    Failed(Box<Diagnostic>),
}

/// A workspace checked and lowered, ready to run any of its files: driven
/// like a [`crate::session::Session`], one frame and one memo table for
/// every file, but with each file's declarations committed once it is
/// checked, and only the files a run needs evaluated. See the module
/// documentation.
#[derive(Debug)]
pub struct Workspace {
    /// The scanned workspace.
    scan: Scan,
    /// Each file's pending IR or completed outcome, by input number.
    runs: Vec<FileRun>,
    /// The type of every expression of each file, by input number, for
    /// [`Workspace::dump_types`].
    checked: Vec<Option<Types>>,
    /// The value of each file that has run, `{}` for the others.
    values: Vec<Value>,
    /// The warnings of every file, in path then source order.
    warnings: Vec<Diagnostic>,
    /// The top level's frame of every file.
    frame: Vec<Value>,
    /// The memo tables of every memoised closure.
    memo: MemoTables,
    /// The layout of the record `exec` returns, built once for the run as
    /// the frame is laid out ([`exec_layout`]).
    exec_result: Rc<Layout>,
    /// The temporary directories `fs.temp_dir` makes for its runs, none
    /// until the first is asked for, removed with everything in them as the
    /// workspace is dropped, in the temp root if one was set
    /// ([`Workspace::set_temp_root`]; see the module documentation, "The
    /// standard library").
    temp_dirs: TempDirs,
    /// The table the files' types are interned into.
    types: TypeTable,
    /// The name of every slot of the frame, the last variable's in it, for
    /// [`Workspace::dump_ir`]: one table for every file, each file's IR
    /// keeping none (a copy of the whole frame's in each would make the
    /// workspace's memory quadratic in its files).
    slot_names: Vec<Name>,
    /// For each file, by input number, the number of slots it committed,
    /// and the names of its slots past them, its temporaries', which a later
    /// file may take and name otherwise in `slot_names`: what
    /// [`Workspace::dump_ir`] names them by.
    slot_tails: Vec<(usize, Vec<Name>)>,
    /// The type of each cell, by its number, as its file's check found it:
    /// what a session's inputs read the standard library's cells as.
    cell_types: Vec<TyId>,
    /// The slot each cell is read from, by merged node: its own, or for a
    /// `def` its function's (see [`Context::cell_slot`]). Kept for a
    /// session's inputs, and for the tests' check that every cell an import
    /// reaches is stored by its owner.
    cell_slots: Vec<Slot>,
    /// For each cell that is a `def`'s, what a call of it needs to know of
    /// its parameters (see [`Context::known`]), kept for a session's inputs
    /// and for the call that invokes a definition by name.
    cell_known: Vec<Option<Rc<KnownParams>>>,
    /// Each file's records of the nodes of its view with a leaf of it under
    /// them, by input number: kept for the tests' check that each reads
    /// only cells stored before it.
    #[cfg(test)]
    view_records: Vec<Vec<ViewRecord>>,
}

/// Scans, checks and lowers the workspace of `sources`, each with its
/// path relative to the root, in the order of [`Scan::order`].
///
/// # Errors
///
/// The diagnostics of the scan, or those of every file whose check or
/// lowering fails, with every file's syntax errors and the warnings found,
/// in path then source order; each names its file in [`Diagnostic::input`].
/// A file that needs a file that failed is not checked: its errors would
/// follow from the first. A file with syntax errors always fails, after
/// its check of what was parsed (see [Syntax errors](#syntax-errors)).
pub fn check(sources: &[SourceFile]) -> Result<Workspace, Vec<Diagnostic>> {
    check_with(sources, &[])
}

/// [`check`], told the directories left out of the workspace as
/// workspaces of their own (see [`scan_with`]). Its root, for the
/// `fileroot` module, is the process's working directory, as a session's
/// is ([`working_directory`]); [`check_with_standard`] is told it.
///
/// # Errors
///
/// As [`check`].
pub fn check_with(sources: &[SourceFile], nested: &[String]) -> Result<Workspace, Vec<Diagnostic>> {
    check_with_standard(sources, nested, &working_directory()).1
}

/// [`check_with`], with the standard files it loaded as sources, each named
/// `<stdlib>/STEM.gcsl`, which are its first files, whether the check
/// succeeds or fails: the sources the diagnostics of the standard files
/// name by input number, the workspace's files' numbers coming after them.
/// Only the prelude and the standard files some file imports are loaded,
/// so the number of the first file of the workspace is their number. `root`
/// is the absolute path of the workspace's root directory, canonical and
/// without a trailing `/` (but `/` itself), which every directory of its
/// files is found from, for the `fileroot` module (see the module
/// documentation, "The standard library").
pub fn check_with_standard(
    sources: &[SourceFile],
    nested: &[String],
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    check_in((sources, Placing::Workspace(nested)), Library::NEEDED, root)
}

/// The process's working directory, canonical (symbolic links resolved),
/// as an absolute path without a trailing `/` (but `/` itself): the root a
/// session's inputs are in ([`crate::Session::new`]), and that of a
/// workspace or a lone file checked by an entry point that is given none
/// ([`check`], [`check_with`], [`check_lone`], [`crate::run`]). `/` if it
/// cannot be found, as when it has been removed; a path that is not UTF-8
/// is taken lossily, each invalid sequence replaced by U+FFFD.
#[must_use]
pub fn working_directory() -> String {
    std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_or_else(|_| "/".to_owned(), |dir| dir.to_string_lossy().into_owned())
}

/// Scans, checks and lowers `source`, a file outside any workspace (`gcsl
/// run` of a lone file), as the workspace of one directory holding just
/// it, beside the standard library (see the module documentation, "The
/// standard library"): it is file [`Scan::standard`], after the standard
/// files it loads.
///
/// # Errors
///
/// As [`check`]; an import of anything but `builtins` and the standard
/// files is the checker's error, "`lib` is not a built-in or standard
/// module, and there is no `workspace.gcsl` above this file".
pub fn check_lone(source: &SourceFile) -> Result<Workspace, Vec<Diagnostic>> {
    check_lone_with_standard(source, &working_directory()).1
}

/// [`check_lone`], with the standard files it loaded as sources, as
/// [`check_with_standard`] gives them: the file's input number is their
/// number. `dir` is the absolute path of the file's directory, canonical
/// and without a trailing `/` (but `/` itself), which is the root of the
/// workspace of one directory it is checked as, for the `fileroot` module;
/// [`check_lone`] takes the working directory.
pub fn check_lone_with_standard(
    source: &SourceFile,
    dir: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    let (loaded, checked) = check_lone_loading(source, dir);
    (Library::NEEDED.sources(&loaded), checked)
}

/// [`check_lone_with_standard`], with the indices in [`crate::STANDARD`] of
/// the standard files it loaded, which are its first files: what
/// [`crate::run`] numbers the file by, with no source made of them.
pub(crate) fn check_lone_loading(
    source: &SourceFile,
    dir: &str,
) -> (Vec<usize>, Result<Workspace, Vec<Diagnostic>>) {
    let sources = std::slice::from_ref(source);
    check_loading((sources, Placing::Lone(&[], false)), Library::NEEDED, dir)
}

/// Scans `sources` placed as `placing` says, beside `library`, and checks
/// and lowers every file, as [`check_with`] does, the root being `root`;
/// with the standard files the scan loaded, as [`check_with_standard`]
/// gives them.
fn check_in(
    placed: (&[SourceFile], Placing),
    library: Library,
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>) {
    let (loaded, checked) = check_loading(placed, library, root);
    (library.sources(&loaded), checked)
}

/// [`check_in`], with the indices in the library's table of the standard
/// files the scan loaded in place of their sources.
fn check_loading(
    placed: (&[SourceFile], Placing),
    library: Library,
    root: &str,
) -> (Vec<usize>, Result<Workspace, Vec<Diagnostic>>) {
    let mut loaded = Vec::new();
    let checked =
        scan_into(placed, library, &mut Names::new(), &mut loaded).and_then(|mut scan| {
            scan.locate(root);
            check_scan(scan)
        });
    (loaded, checked)
}

/// The tables of the cells of `scan` before any file is checked: each
/// one's type, `Error` but for the built-in functions', the first
/// ([`builtin_nodes`]), each built-in's (see [`Builtin::ty`]), interned
/// into `types` with `names`; each one's slot; and nothing known of any
/// `def`'s parameters.
fn cell_tables(scan: &Scan, types: &mut TypeTable, names: &mut Names) -> CellTables {
    let mut cell_types = vec![TyId::ERROR; scan.cells()];
    for (cell, builtin) in cell_types.iter_mut().zip(Builtin::ALL) {
        *cell = builtin.ty(types, names);
    }
    let cell_slots = (0..count(scan.cells()))
        .map(|cell| scan.slot(cell))
        .collect();
    (cell_types, cell_slots, vec![None; scan.cells()])
}

/// Takes the names of the slots of `ir`, a file's: its slots' names are the
/// workspace's, `slot_names` (see [`Workspace::dump_ir`]), but for those of
/// its temporaries past the slots it committed, which are kept, with how
/// many it committed, and added to `slot_names`, so that the files checked
/// after it take slots past them. Files do not run in the order they are
/// checked in (a second invocation runs the files it needs after files
/// checked later have run), so a later file's slots, which hold its cells
/// for as long as the workspace lives, must not be a temporary of one that
/// may run after it.
fn own_slot_names(ir: &mut Ir, slot_names: &mut Vec<Name>) -> (usize, Vec<Name>) {
    let committed = slot_names.len();
    let own = ir.replace_slot_names(Vec::new());
    let tail = own.get(committed..).unwrap_or_default().to_vec();
    slot_names.extend_from_slice(&tail);
    (committed, tail)
}

/// The tables of a workspace's cells, by number: each one's type, the slot
/// it is read from, and for a `def`'s what a call of it needs to know of its
/// parameters.
type CellTables = (Vec<TyId>, Vec<Slot>, Vec<Option<Rc<KnownParams>>>);

/// Checks and lowers every file of `scan` in the order (see [`check`]), each
/// where the scan located it ([`Scan::locate`]).
fn check_scan(scan: Scan) -> Result<Workspace, Vec<Diagnostic>> {
    check_scan_partial(scan).map_err(|failed| failed.diagnostics)
}

/// A check of a workspace that failed ([`check_partial`]): its diagnostics,
/// as [`check_with_standard`] gives them, and what it found of the
/// workspace, if its scan succeeded.
#[derive(Debug)]
pub struct Failed {
    /// Every diagnostic, in path then source order.
    pub diagnostics: Vec<Diagnostic>,
    /// What the check found, once the scan succeeded; `None` if it failed.
    pub partial: Option<Box<Partial>>,
}

impl Failed {
    /// The check of `scan`, which interned its types into `types` and found
    /// `checked` of its files, failed with `errors`, and each file's
    /// `warnings`, by input number.
    fn of(
        scan: Scan,
        (types, checked): (TypeTable, Vec<Option<Types>>),
        (errors, warnings): (Vec<Diagnostic>, Vec<Vec<Diagnostic>>),
    ) -> Self {
        let warnings = warnings.into_iter().flatten().collect();
        Self {
            diagnostics: merge_files(reported(errors), warnings),
            partial: Some(Box::new(Partial {
                scan,
                types,
                checked,
            })),
        }
    }
}

/// What a check of a workspace that failed leaves of it once its scan
/// succeeded ([`check_partial`]), for a tool that shows what it can of it,
/// as a language server would: the scan, every file's tree as its parser
/// recovered it from its syntax errors ([`crate::ast::Ast::syntax`]), the
/// table its types are interned into, and the types of each file whose
/// check succeeded, by input number. A file whose check failed, one with a
/// syntax error among them, has none, nor does one not checked.
#[derive(Debug)]
pub struct Partial {
    /// The scanned workspace, its names in [`Scan::names`].
    pub scan: Scan,
    /// The table the files' types are interned into.
    pub types: TypeTable,
    /// The type of every expression of each file whose check succeeded, by
    /// input number.
    pub checked: Vec<Option<Types>>,
}

/// [`check_with_standard`], with what the check found of the workspace if
/// it failed after its scan, as [`Partial`] gives it.
pub fn check_partial(
    sources: &[SourceFile],
    nested: &[String],
    root: &str,
) -> (Vec<SourceFile>, Result<Workspace, Failed>) {
    let placed = (sources, Placing::Workspace(nested));
    let mut loaded = Vec::new();
    let checked = match scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded) {
        Ok(mut scan) => {
            scan.locate(root);
            check_scan_partial(scan)
        }
        Err(diagnostics) => Err(Failed {
            diagnostics,
            partial: None,
        }),
    };
    (Library::NEEDED.sources(&loaded), checked)
}

/// [`check_scan`], with what the check found if it fails ([`Partial`]).
fn check_scan_partial(scan: Scan) -> Result<Workspace, Failed> {
    check_scan_recording(scan, false).0
}

/// What a check of a workspace that recorded found for its index, besides
/// the workspace or what it found of it (see [`crate::index`]).
#[derive(Debug, Default)]
pub(crate) struct Recorded {
    /// What each file's check recorded, by input number; `None` for a file
    /// not checked, whose tree is unbalanced. (A file that needs one whose
    /// check failed is checked for this alone: `record_blocked`.)
    pub(crate) files: Vec<Option<Box<Record>>>,
    /// The type of each cell, by its number, as the check of the file that
    /// computes it found it, whether that check succeeded or failed: for
    /// what is shown, never for what is checked.
    pub(crate) cells: Vec<TyId>,
}

impl Recorded {
    /// What a check of a workspace of `files` files records, if it
    /// `records`, before any file is checked, the cells' types being
    /// `cells`: nothing of any file yet.
    fn new(records: bool, files: usize, cells: &[TyId]) -> Self {
        if !records {
            return Self::default();
        }
        Self {
            files: (0..files).map(|_| None).collect(),
            cells: cells.to_vec(),
        }
    }

    /// Keeps `found`, what the check of file `index` of `scan` recorded, if
    /// it recorded, and the types of the cells the file computes, as its
    /// check found them, failed or not.
    fn keep(&mut self, scan: &Scan, index: u32, found: Option<Box<Record>>) {
        let Some(found) = found else {
            return;
        };
        let global = scan
            .files
            .get(index as usize)
            .map_or(&[][..], |f| &f.global);
        for (&global, &ty) in global.iter().zip(&found.cells) {
            if ty != TyId::ERROR
                && scan.computed_by(global) == Some(index)
                && !scan.is_poisoned(global)
            {
                set(&mut self.cells, global, ty);
            }
        }
        if let (Some(cell), Some(ty)) = (scan.value_cell(index), found.value) {
            set(&mut self.cells, cell, ty);
        }
        set(&mut self.files, index, Some(found));
    }
}

/// The check of file `index` of `scan` in `context`, from `scope`,
/// interning into `types`, and its lowering if it succeeds: what it records
/// of its names kept in `recorded`, if the check of the workspace records
/// (see [`crate::check`], "Recording").
fn check_file<'a>(
    (scan, index): (&'a Scan, u32),
    context: &'a Context<'a>,
    (scope, types): (Scope, &'a mut TypeTable),
    recorded: &mut Recorded,
) -> Result<(Ir, CheckOutput), Vec<Diagnostic>> {
    let ast = &context.file().ast;
    let checking = Checker::in_workspace(ast, scope, types, context);
    let records = !recorded.files.is_empty();
    let checking = if records {
        checking.recording()
    } else {
        checking
    };
    let (output, found) = checking.check_recorded();
    recorded.keep(scan, index, found);
    output.and_then(|output| session::lower_checked(ast, output, types, index))
}

/// If the check of `scan` records (see [`crate::check`], "Recording"),
/// once every file that could be is checked, checks each file that needs
/// a file whose check failed, and so was not, for its index alone (see
/// [`crate::index`]), in the order: from the declarations `scopes` hold,
/// with the cells' slots and parameters as the checks left them, and their
/// types as `recorded` has them (those the failed checks found among
/// them), lent the workspace's `names`, interning into `types`. It keeps
/// what each records in `recorded`, throws away what each finds, and
/// commits nothing, a name that names nothing taken as no error, since it
/// may be declared in the file that failed. A file whose tree is
/// unbalanced is still not checked.
fn record_blocked(
    scan: &mut Scan,
    (scopes, reads): (&[Committed], CellReads<'_>),
    (names, types, recorded): (&mut Names, &mut TypeTable, &mut Recorded),
) {
    if recorded.files.is_empty() {
        return;
    }
    let cell_types = recorded.cells.clone();
    for index in scan.order.clone() {
        let checked = recorded
            .files
            .get(index as usize)
            .is_some_and(Option::is_some);
        if checked || scan.is_unbalanced(index) {
            continue;
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.replace_names(mem::take(names));
        }
        if let Some(file) = scan.files.get(index as usize)
            && let Some(context) = Context::new(scan, index, &cell_types, reads)
        {
            let scope = Committed::scope(scan, scopes, file, scan.cell_names());
            let checking = Checker::in_workspace(&file.ast, scope, types, &context);
            let (_, found) = checking.recording().blind().check_recorded();
            recorded.keep(scan, index, found);
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            *names = file.ast.replace_names(Names::new());
        }
    }
}

/// [`check_scan_partial`], each file's check recording what its names refer
/// to if `record` (see [`crate::check`], "Recording"), with what it
/// recorded.
fn check_scan_recording(mut scan: Scan, record: bool) -> (Result<Workspace, Failed>, Recorded) {
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let (mut cell_types, mut cell_slots, mut cell_known) =
        cell_tables(&scan, &mut types, &mut names);
    let mut slot_names: Vec<Name> = scan.cell_names();
    let mut scopes = vec![Committed::default(); scan.dirs.len()];
    let mut runs: Vec<FileRun> = (0..scan.files.len()).map(|_| FileRun::Succeeded).collect();
    let mut tails: Vec<(usize, Vec<Name>)> = vec![(0, Vec::new()); scan.files.len()];
    let mut checked: Vec<Option<Types>> = (0..scan.files.len()).map(|_| None).collect();
    let mut warnings = vec![Vec::new(); scan.files.len()];
    let mut errors = check_type_items(&mut scan, &mut names, &mut types);
    // The files whose check failed, or that need one that did: those are
    // not checked, since their errors would follow from the first; and
    // those with syntax errors (see [Syntax errors](#syntax-errors)).
    let mut failed = scan.broken_files();
    let mut recorded = Recorded::new(record, scan.files.len(), &cell_types);
    #[cfg(test)]
    let mut view_records: Vec<Vec<ViewRecord>> = vec![Vec::new(); scan.files.len()];
    for index in scan.order.clone() {
        if blocked(&scan, &mut failed, index) || scan.unchecked(index) {
            continue;
        }
        if let Some(file) = scan.files.get_mut(index as usize) {
            file.ast.replace_names(mem::take(&mut names));
        }
        // Every file of the order is a file of the workspace.
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        // The slots' names are moved, not copied: the check gives them back
        // with the file's own slots added.
        let scope = Committed::scope(&scan, &scopes, file, mem::take(&mut slot_names));
        let start = scope.len();
        let reads = (&cell_slots[..], &cell_known[..]);
        let compiled = Context::new(&scan, index, &cell_types, reads).map(|context| {
            check_file((&scan, index), &context, (scope, &mut types), &mut recorded)
        });
        // What the file declares, committed for the files after it.
        let cells = match &compiled {
            Some(Ok((_, output))) => computed_cells(&scan, index, output),
            _ => Vec::new(),
        };
        let keep = |name: Name| commits(&scan, index, name);
        let poison = |name: Name| poisoned_root(&scan, index, name);
        #[cfg(test)]
        if let Some(Ok((_, output))) = &compiled {
            let records = output.checked.resolution.view_records();
            set(&mut view_records, index, records.to_vec());
        }
        let result = match compiled {
            Some(Ok((ir, output))) => {
                if let Some(dir) = scopes.get_mut(file.dir as usize) {
                    dir.commit(&output.scope, start, (keep, poison));
                }
                slot_names = output.scope.into_slot_names();
                Ok((ir, output.checked.types, output.warnings))
            }
            failed_check => {
                // The slots' names went to the failed check. Nothing runs
                // once a file has failed, so the files checked after it
                // need only the merged nodes' slots kept apart from theirs.
                slot_names = scan.cell_names();
                // Every file of the order is a file of the workspace.
                Err(failed_check.and_then(Result::err).unwrap_or_default())
            }
        };
        if let Some(file) = scan.files.get_mut(index as usize) {
            names = file.ast.replace_names(Names::new());
        }
        set_cells(&mut cell_types, &mut cell_slots, &mut cell_known, cells);
        match result {
            Ok((mut ir, file_types, found)) => {
                set(&mut tails, index, own_slot_names(&mut ir, &mut slot_names));
                set(&mut runs, index, FileRun::Pending(Box::new(ir)));
                set(&mut checked, index, Some(file_types));
                set(&mut warnings, index, in_file(found, index));
            }
            Err(found) => {
                errors.extend(scan.failure(index, found));
                set(&mut failed, index, true);
            }
        }
    }
    let tables = (&scopes[..], (&cell_slots[..], &cell_known[..]));
    let lent = (&mut names, &mut types, &mut recorded);
    record_blocked(&mut scan, tables, lent);
    scan.names = names;
    if !errors.is_empty() || failed.contains(&true) {
        let failed = Failed::of(scan, (types, checked), (errors, warnings));
        return (Err(failed), recorded);
    }
    let workspace = Workspace {
        values: vec![Value::Empty; scan.files.len()],
        // Built before `scan` moves; its names are interned already.
        exec_result: exec_layout(&mut scan.names),
        temp_dirs: TempDirs::new(),
        scan,
        runs,
        checked,
        warnings: warnings.into_iter().flatten().collect(),
        // The driver writes the built-in functions' cells, the first slots,
        // as it lays the frame out, before any file runs.
        frame: builtin_cells(),
        slot_names,
        slot_tails: tails,
        memo: MemoTables::new(),
        types,
        cell_types,
        cell_slots,
        cell_known,
        #[cfg(test)]
        view_records,
    };
    (Ok(workspace), recorded)
}

/// Scans `sources` placed as `placing` says, beside the standard library,
/// and checks every file as [`check_with_standard`] does, the root being
/// `root`, each file's check recording what its names refer to (see
/// [`crate::check`], "Recording"), and indexes what it found, whether it
/// succeeded or failed: what [`index_with_standard`] and
/// [`index_lone_with_standard`] do.
fn index_in(placed: (&[SourceFile], Placing), root: &str) -> Indexed {
    let given = placed.0;
    let mut loaded = Vec::new();
    let scanned = scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded);
    let standard = Library::NEEDED.sources(&loaded);
    let mut scan = match scanned {
        Ok(scan) => scan,
        Err(diagnostics) => {
            return Indexed {
                standard,
                diagnostics,
                succeeded: false,
                index: None,
            };
        }
    };
    scan.locate(root);
    let (checked, recorded) = check_scan_recording(scan, true);
    let (diagnostics, succeeded, parts) = match checked {
        Ok(workspace) => {
            let Workspace {
                scan,
                types,
                warnings,
                ..
            } = workspace;
            (warnings, true, Some((scan, types)))
        }
        Err(Failed {
            diagnostics,
            partial,
        }) => (diagnostics, false, partial.map(|p| (p.scan, p.types))),
    };
    let index = parts.map(|(scan, types)| {
        let mut sources = standard.clone();
        sources.extend(given.iter().cloned());
        Index::new(scan, types, recorded, (sources, &diagnostics))
    });
    Indexed {
        standard,
        diagnostics,
        succeeded,
        index,
    }
}

/// Scans, checks and indexes the workspace of `sources`, as
/// [`check_with_standard`] checks it, with the same diagnostics, running
/// nothing, for a language server (see [`crate::index`]): each file's check
/// records what its names
/// refer to (see [`crate::check`], "Recording"), and the index answers what
/// a name at a place is and where it is declared, whether the check
/// succeeded or failed, in every file it checked. `root` is as for
/// [`check_with_standard`].
#[must_use]
pub fn index_with_standard(sources: &[SourceFile], nested: &[String], root: &str) -> Indexed {
    index_in((sources, Placing::Workspace(nested)), root)
}

/// [`index_with_standard`] of `source`, a file outside any workspace,
/// checked as [`check_lone_with_standard`] checks it, in the directory
/// `dir`.
#[must_use]
pub fn index_lone_with_standard(source: &SourceFile, dir: &str) -> Indexed {
    let sources = std::slice::from_ref(source);
    index_in((sources, Placing::Lone(&[], false)), dir)
}

/// Checks the `type` items of every file of `scan` that has any, before any
/// file is checked, since a file may use a type of a file checked after it
/// (see [Types](#types)): what is wrong in each declaration is reported
/// here, once, and every file that uses it fails quietly. Lends each file
/// `names`, and interns into `types`. Returns the errors, each naming its
/// file, after the files' syntax errors and the errors of the scan: the
/// errors found before any file is checked. A file whose tree is
/// unbalanced is not checked; in one whose
/// view sees a file with syntax errors, an unknown type is no error (see
/// [Syntax errors](#syntax-errors)).
fn check_type_items(scan: &mut Scan, names: &mut Names, types: &mut TypeTable) -> Vec<Diagnostic> {
    let mut errors = scan.syntax_errors.clone();
    errors.extend(scan.recovered.iter().cloned());
    let broken: Vec<bool> = (0..count(scan.files.len()))
        .map(|index| scan.sees_broken(index))
        .collect();
    for ((index, file), sees_broken) in (0u32..).zip(&mut scan.files).zip(broken) {
        if file.mentions.types.is_empty() || file.ast.syntax() == Syntax::Unbalanced {
            continue;
        }
        file.ast.replace_names(mem::take(names));
        let found = check_types(&file.ast, types, sees_broken);
        *names = file.ast.replace_names(Names::new());
        errors.extend(in_file(found, index));
    }
    errors
}

/// `errors`, the errors of a workspace some file of which failed its check,
/// or, if there are none, the internal error that one failed with nothing
/// reported: a check fails so quietly only for an erroneous type, which
/// [`check_type_items`] reports where it is declared.
fn reported(errors: Vec<Diagnostic>) -> Vec<Diagnostic> {
    if !errors.is_empty() {
        return errors;
    }
    vec![
        InternalError {
            message: "a file's check failed with nothing reported",
            span: Span::new(0, 0),
        }
        .into(),
    ]
}

/// Whether file `index` commits its declaration `name` for the files
/// checked after it: a root it computes, being its last contributor, or a
/// `def` of that name. (A `def` named by a dotted path is a leaf under its
/// root, committed with the root.)
fn commits(scan: &Scan, index: u32, name: Name) -> bool {
    let Some(file) = scan.files.get(index as usize) else {
        return false;
    };
    file.mentions.tree.root(name).is_some_and(|root| {
        let global = file.global.get(root as usize).copied();
        global.and_then(|global| scan.computed_by(global)) == Some(index)
    }) || file.mentions.functions.iter().any(|top| {
        top.name == Some(name) && matches!(file.ast.item(top.item).kind, ItemKind::Field { .. })
    })
}

/// Whether file `index`'s root `name` is poisoned, a conflicted node at or
/// under it (see [Scan errors](#scan-errors)): it is committed as `Error`.
fn poisoned_root(scan: &Scan, index: u32, name: Name) -> bool {
    if scan.poisoned_nodes.is_empty() {
        return false;
    }
    let Some(file) = scan.files.get(index as usize) else {
        return false;
    };
    file.mentions
        .tree
        .root(name)
        .and_then(|root| file.global.get(root as usize))
        .is_some_and(|&global| scan.is_poisoned(global))
}

/// The declarations the files of one directory have committed, for the
/// files checked after them: a scope, with its variables indexed by name,
/// so that a file is checked from the declarations of the names it spells
/// alone, never a copy of every declaration of its directory chain (which
/// would make checking a workspace quadratic in its files).
#[derive(Clone, Debug, Default)]
struct Committed {
    /// The variables, in the order they were committed.
    scope: Scope,
    /// The indices in `scope` of the variables of each name, in order.
    by_name: HashMap<Name, Vec<usize>>,
}

impl Committed {
    /// The scope `file` is checked from, given the committed declarations
    /// of each directory of `scan` and the names of the frame's slots so
    /// far: those of its directory chain, from the root down, of the names
    /// it spells.
    fn scope(scan: &Scan, committed: &[Self], file: &File, slot_names: Vec<Name>) -> Scope {
        let chain: Vec<u32> = scan.chain(file.dir).collect();
        Scope::joined(
            chain.iter().rev().filter_map(|&dir| {
                let committed = committed.get(dir as usize)?;
                Some((&committed.scope, committed.picked(&file.spelled)))
            }),
            slot_names,
        )
    }

    /// The indices in `scope` of the variables named in `spelled`, in
    /// commit order, so that of two variables of one name the later is
    /// still the later.
    fn picked(&self, spelled: &[Name]) -> Vec<usize> {
        let mut picked: Vec<usize> = spelled
            .iter()
            .filter_map(|name| self.by_name.get(name))
            .flatten()
            .copied()
            .collect();
        picked.sort_unstable();
        picked
    }

    /// Commits the variables of `from` after its first `start` whose names
    /// `keep` accepts, as [`Scope::commit`] does, and indexes them; each
    /// whose name `poisoned` accepts as `Error` (see [Scan
    /// errors](#scan-errors)).
    fn commit(
        &mut self,
        from: &Scope,
        start: usize,
        (keep, poisoned): (impl Fn(Name) -> bool, impl Fn(Name) -> bool),
    ) {
        let before = self.scope.len();
        self.scope.commit(from, start, keep);
        self.scope.poison_from(before, poisoned);
        for (at, name) in self.scope.names().enumerate().skip(before) {
            self.by_name.entry(name).or_default().push(at);
        }
    }
}

/// The cells file `index` computes, now that its check gave `output`: each
/// one's number, the slot it is read from (a `def`'s is its function's) and
/// its type, with the cell of its value if it is an imported value file.
fn computed_cells(scan: &Scan, index: u32, output: &CheckOutput) -> Vec<ComputedCell> {
    let file = scan.files.get(index as usize);
    let global = file.map_or(&[][..], |file| &file.global);
    let slots = output.checked.resolution.field_cells();
    let known_by_slot = output.scope.known_by_slot();
    let nodes = file.map_or(&[][..], |file| &file.mentions.tree.nodes);
    let mut cells = Vec::new();
    for (own, (&global, &ty)) in global.iter().zip(&output.cell_types).enumerate() {
        if scan.computed_by(global) == Some(index) {
            // A `def`'s cell is read from its function's slot.
            let slot = slots.get(own).copied().unwrap_or_else(|| scan.slot(global));
            let mut known = known_by_slot.get(&slot).copied().flatten().cloned();
            // A `def` named by a dotted path has no binding: what a call of
            // it needs to know is its function's, which the cell's type is.
            if known.is_none()
                && ty != TyId::ERROR
                && let Some(function) = nodes
                    .get(own)
                    .and_then(|node| node.leaf)
                    .and_then(|leaf| leaf.function)
                    .and_then(|function| file?.mentions.functions.get(function as usize))
                    .filter(|function| function.name.is_none())
            {
                let resolution = &output.checked.resolution;
                let attributes = resolution.attributes(function.function);
                known = Some(KnownParams::of(
                    resolution.function(function.function),
                    attributes,
                ));
            }
            // A poisoned cell is read as `Error` (see [Scan
            // errors](#scan-errors)).
            if scan.is_poisoned(global) {
                cells.push((global, slot, TyId::ERROR, None));
            } else {
                cells.push((global, slot, ty, known));
            }
        }
    }
    // An imported value file's value is a cell of the workspace.
    if let Some(cell) = scan.value_cell(index) {
        cells.push((cell, scan.slot(cell), output.ty, None));
    }
    cells
}

/// A cell a file computes, as [`computed_cells`] gives it: its number, the
/// slot it is read from, its type, and for a `def` what a call of it needs to
/// know of its parameters.
type ComputedCell = (u32, Slot, TyId, Option<Rc<KnownParams>>);

/// Sets the type, the slot and the parameters of each of `cells` in the
/// cells' tables, `types`, `slots` and `known`.
fn set_cells(
    types: &mut [TyId],
    slots: &mut [Slot],
    known: &mut [Option<Rc<KnownParams>>],
    cells: Vec<ComputedCell>,
) {
    for (global, slot, ty, params) in cells {
        set(types, global, ty);
        set(slots, global, slot);
        set(known, global, params);
    }
}

/// Sets the entry `index` of `entries` to `value`, if it has one.
fn set<T>(entries: &mut [T], index: u32, value: T) {
    if let Some(entry) = entries.get_mut(index as usize) {
        *entry = value;
    }
}

/// Whether file `index` needs a file in `failed` (whose check failed, or
/// that was not checked for this reason), marking it failed if it does.
fn blocked(scan: &Scan, failed: &mut [bool], index: u32) -> bool {
    let blocked = scan.needs.get(index as usize).is_some_and(|needs| {
        needs
            .iter()
            .any(|&need| failed.get(need as usize).copied().unwrap_or(false))
    });
    if blocked && let Some(entry) = failed.get_mut(index as usize) {
        *entry = true;
    }
    blocked
}

/// Merges `errors` and `warnings` of several files into one vector, as
/// [`crate::diagnostic::merge`] does for one: in path order of their files (their
/// [`Diagnostic::input`]), then in source order, errors first among those
/// that start at one place.
fn merge_files(errors: Vec<Diagnostic>, warnings: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut all = errors;
    all.extend(warnings);
    all.sort_by_key(|diagnostic| (diagnostic.input, diagnostic.span.start));
    all
}

/// `diagnostics`, each named as of file `index` unless it names one.
fn in_file(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        diagnostic.input.get_or_insert(index);
    }
    diagnostics
}

impl Workspace {
    /// The scanned workspace: its files, their order and needs.
    #[must_use]
    pub fn scan(&self) -> &Scan {
        &self.scan
    }

    /// The input number of the file at `path`, relative to the root.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<u32> {
        self.scan
            .files
            .iter()
            .position(|file| file.path == path)
            .and_then(|at| u32::try_from(at).ok())
    }

    /// The table the files' types are interned into.
    #[must_use]
    pub fn types(&self) -> &TypeTable {
        &self.types
    }

    /// The warnings of every file, in path then source order.
    #[must_use]
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Makes `root` the workspace's temp root: the directory in which the
    /// parent of the temporary directories `fs.temp_dir` makes in its runs
    /// is made, instead of the system's temporary directory (see the module
    /// documentation, "The standard library"). Everything else is as
    /// without it: the parent, `gcsl-PID-N`, is made by the first call and
    /// never before, and removed with everything in it as the workspace is
    /// dropped. `root` is made then, with its missing parents, if it is
    /// missing, and is never removed. A `root` that is not absolute fails
    /// every call: "`fs.temp_dir` could not create a temporary directory:
    /// the temporary directory root `ROOT` is not an absolute path".
    ///
    /// The root is read only as the parent is made, so a host calls this
    /// before running anything: once a temporary directory has been made,
    /// it changes nothing, and the parent made stays in use.
    pub fn set_temp_root(&mut self, root: PathBuf) {
        self.temp_dirs.set_root(root);
    }

    /// The syntax tree of file `index` annotated with the types it was
    /// checked with in the workspace, as `--dump-types` prints it.
    pub fn dump_types(&mut self, index: u32) -> Option<String> {
        let types = self.checked.get(index as usize)?.as_ref()?;
        let file = self.scan.files.get_mut(index as usize)?;
        file.ast.replace_names(mem::take(&mut self.scan.names));
        let annotation = types.annotation(&self.types, file.ast.names());
        let text = file.ast.pretty_with(&annotation).to_string();
        self.scan.names = file.ast.replace_names(Names::new());
        Some(text)
    }

    /// The IR file `index` was lowered to in the workspace, as `--dump-ir`
    /// prints it, until the file runs.
    #[must_use]
    pub fn dump_ir(&mut self, index: u32) -> Option<String> {
        let FileRun::Pending(ir) = self.runs.get_mut(index as usize)? else {
            return None;
        };
        // The names of the slots it committed, of which its frame is a
        // prefix, then its own temporaries'.
        let (committed, tail) = self.slot_tails.get(index as usize)?;
        let mut names = self
            .slot_names
            .get(..*committed)
            .unwrap_or_default()
            .to_vec();
        names.extend_from_slice(tail);
        ir.replace_slot_names(names);
        let dump = ir.pretty(&self.scan.names).to_string();
        ir.replace_slot_names(Vec::new());
        Some(dump)
    }

    /// Runs file `index` and, before it, the standard files the scan loaded
    /// and every file it needs that has not run, in the order
    /// ([`Scan::needed`]), on the frame whose first cells the driver wrote
    /// with the built-in functions, and returns its own value: its result,
    /// or the record of its own fields, with the warnings of the whole
    /// workspace.
    ///
    /// # Errors
    ///
    /// The runtime error of the first file that fails, naming its file,
    /// with the workspace's warnings. A failed file retains its diagnostic:
    /// later requests needing it return that error without running it or its
    /// dependents again. Requests for unrelated files can still run.
    pub fn run(&mut self, index: u32) -> Result<Outcome, Vec<Diagnostic>> {
        self.run_needed(index)?;
        Ok(Outcome {
            value: self
                .values
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Empty),
            warnings: self.warnings.clone(),
        })
    }

    /// Runs file `index` and every file it needs, as [`Workspace::run`]
    /// does, but those that have run already, keeping each one's value.
    ///
    /// # Errors
    ///
    /// As [`Workspace::run`].
    fn run_needed(&mut self, index: u32) -> Result<(), Vec<Diagnostic>> {
        for at in self.scan.needed(index) {
            let Some(state) = self.runs.get_mut(at as usize) else {
                continue;
            };
            let mut ir = match mem::replace(state, FileRun::Succeeded) {
                FileRun::Pending(ir) => ir,
                FileRun::Succeeded => continue,
                FileRun::Failed(error) => {
                    let diagnostic = (*error).clone();
                    *state = FileRun::Failed(error);
                    return Err(merge_files(vec![diagnostic], self.warnings.clone()));
                }
            };
            // The slots of the files run before it, which may end after its
            // own, are kept (see [`Ir::reserve_frame`]).
            ir.reserve_frame(count(self.frame.len()));
            let context = (
                &mut self.memo,
                Some(&self.exec_result),
                &self.scan.dir_texts[..],
                &self.temp_dirs,
            );
            match session::run_input(*ir, &mut self.frame, context) {
                Ok(value) => {
                    if let Some(entry) = self.values.get_mut(at as usize) {
                        *entry = value;
                    }
                }
                Err(mut error) => {
                    error.input.get_or_insert(at);
                    *state = FileRun::Failed(Box::new(error.clone()));
                    return Err(merge_files(vec![error], self.warnings.clone()));
                }
            }
        }
        Ok(())
    }
}

/// A def-type of a workspace, as [`Workspace::def_types`] lists it: a `def`
/// marked `#[def_type]` of one of the workspace's files, with every
/// definition made with it (see the module documentation, "Def-types by
/// name").
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
/// ... }`, as [`Workspace::def_types`] lists it (see the module
/// documentation, "Def-types by name").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Its path, as the definition writes it: `dev.peios.testpkg`. It is
    /// invoked by it, alone if no other definition of its def-type has its
    /// path, else qualified by its directory ([`Definition::qualified`]).
    pub path: String,
    /// The path of its file's directory relative to the root, as
    /// [`Dir::path`] gives it, `pkgs/net.busybox.busybox`; empty for the
    /// root.
    pub dir: String,
    /// The input number of its file.
    pub file: u32,
    /// Where its path is written in its file.
    pub span: Span,
}

impl Definition {
    /// Its path qualified by its directory, as [`Workspace::invoke`] takes
    /// it to name this definition whatever others have its path:
    /// `pkgs/net.busybox.busybox/busybox`, and for one of the root,
    /// `/busybox` (see the module documentation, "Def-types by name").
    #[must_use]
    pub fn qualified(&self) -> String {
        qualified(&self.dir, &self.path)
    }
}

/// The path `path` of a definition of the directory `dir` qualified by it:
/// `dir/path`, and `/path` for the root (see [`Definition::qualified`]).
fn qualified(dir: &str, path: &str) -> String {
    format!("{dir}/{path}")
}

/// The definition a def-type reference names, as
/// [`Workspace::definition_of`] finds it: what [`Workspace::invoke`] and
/// [`Workspace::meta_field`] take to invoke it, or read its meta, by name.
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
    /// [`Definition::qualified`] gives it: what names this very definition
    /// to [`Workspace::invoke`] and [`Workspace::meta_field`], whatever
    /// others have its path.
    #[must_use]
    pub fn qualified(&self) -> String {
        qualified(&self.dir, &self.path)
    }
}

/// Why [`Workspace::invoke`] could not invoke a definition, or another
/// call by name could not be made ([`Workspace::call_composed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvokeError {
    /// What the workspace's files say: the duplicates that
    /// [`Workspace::def_types`] fails with, or the runtime error of a file
    /// that ran or of the call, with the workspace's warnings, as
    /// [`Workspace::run`] returns them. Each names its file, a standard
    /// file or one of the workspace's.
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
    /// file ([`Workspace::call_composed`]): "`pekit.gcsl` has no top-level
    /// function `teardown`".
    NoFunction {
        /// The file's path, relative to the root.
        file: String,
        /// The name asked for.
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
            RequestError::CannotBeGiven { .. }
            | RequestError::DuplicateArgument { .. }
            | RequestError::MissingArguments { .. }
            | RequestError::NoFunction { .. } => Vec::new(),
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
fn quoted_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `items`, each in backquotes, the last two joined by ` and `, the others
/// by `, `: "`a`", "`a` and `b`", "`a`, `b` and `c`".
fn and_list(items: &[String]) -> String {
    match items {
        [rest @ .., last] if !rest.is_empty() => {
            format!("{} and `{last}`", quoted_list(rest))
        }
        _ => quoted_list(items),
    }
}

/// A value given for an invocation parameter of a def-type, to invoke one of
/// its definitions with ([`Workspace::invoke`]): of one of the six types a
/// parameter given an argument can be, `String`, `Int`, `Float`, `Bool`,
/// `Duration` or `Size`,
/// which the call that invokes the definition writes as a literal of its
/// kind (see the module documentation, "Def-types by name").
/// [`Workspace::arguments`] reads one from text, by the parameter's type.
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
enum Scalar {
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
    const ALL: [Scalar; 6] = [
        Scalar::String,
        Scalar::Int,
        Scalar::Float,
        Scalar::Bool,
        Scalar::Quantity(Quantity::Duration),
        Scalar::Quantity(Quantity::Size),
    ];

    /// Its type.
    fn ty(self) -> TyId {
        match self {
            Scalar::String => TyId::STRING,
            Scalar::Int => TyId::INT,
            Scalar::Float => TyId::FLOAT,
            Scalar::Bool => TyId::BOOL,
            Scalar::Quantity(quantity) => TyId::of_quantity(quantity),
        }
    }

    /// Its name, as a type is written.
    fn name(self) -> &'static str {
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
    fn read(self, text: &str) -> Option<Given> {
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
    fn fits(self, given: &Given) -> bool {
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

/// An argument for an invocation parameter, as [`Workspace::given`] reads
/// it: the text of a value, read by the parameter's type, or none, its value
/// not known yet; or a value.
#[derive(Clone, Copy)]
enum Argument<'a> {
    /// The text of the value, or none.
    Text(Option<&'a str>),
    /// The value.
    Given(&'a Given),
}

/// An invocation parameter of a def-type, as [`Workspace::given`] reads
/// arguments for it.
struct Invocable<'a> {
    /// Its name.
    name: &'a str,
    /// Its type, as it is written.
    written: &'a str,
    /// Whether it has a default.
    optional: bool,
    /// Which of the four types an argument can be given for it is, if one.
    scalar: Option<Scalar>,
}

/// A def-type as [`Workspace::def_types`] finds it, with what invoking one
/// of its definitions needs besides: its item, the type of each of its
/// invocation parameters as it is written, and the item of each of its
/// definitions, in the order of `listed`'s.
#[derive(Debug)]
struct FoundDefType {
    /// The def-type as it is listed.
    listed: DefType,
    /// Its `def`'s item.
    item: ItemId,
    /// The type of each of its invocation parameters, as it is written, in
    /// the order of `listed`'s parameters.
    written: Vec<String>,
    /// The item of each of its definitions.
    items: Vec<ItemId>,
}

/// A definition resolved for a host request, with both typed runtime cells.
/// Named fields keep the definition's index distinct from its storage slot.
struct ResolvedDefinition {
    def_type: FoundDefType,
    definition_index: usize,
    def_type_cell: (Slot, TyId),
    definition_cell: (Slot, TyId),
}

/// The name, no source can write it, that the call invoking a definition by
/// name reads the def-type by (see [Def-types by name](#def-types-by-name)).
const INVOKED_DEF_TYPE: &str = "<def-type>";

/// The name, no source can write it, that the call invoking a definition by
/// name reads the definition by.
const INVOKED_DEFINITION: &str = "<definition>";

impl Workspace {
    /// Every def-type of the workspace's own files, the standard files'
    /// aside, with every definition made with it, a def-type with none
    /// included, in path order of their files, then in source order (see
    /// the module documentation, "Def-types by name"). A definition is
    /// found in whichever file it is, whichever files see it, and listed
    /// under the def-type its `DT` names, through whatever import or alias,
    /// by its path as it writes it; a `#[shadowable]` one that a later one
    /// shadows is found too.
    ///
    /// # Errors
    ///
    /// One error for each chain of definitions of one path of a def-type in
    /// directories that see each other, one through another, "`package
    /// dev.peios.x` is defined 2 times in directories that see each other",
    /// and each path of which two def-types or
    /// more are declared, "def-type `package` is declared 2 times in the
    /// workspace", at the first in path then source order, labelled "also
    /// defined here", or "also declared here", at each other; and one error
    /// for each definition whose path lies under another's of its def-type
    /// in a directory that sees its own, "`package app.core` lies under another
    /// `package` definition", at it, labelled "`app` is defined here" at
    /// each definition above it; all in path order of their files, then in
    /// source order, with the workspace's warnings, as [`Workspace::run`]
    /// returns its errors. None is an error of the language: the workspace
    /// checks and runs regardless. One path in directories that see nothing
    /// of each other is no error: each is named with its directory.
    pub fn def_types(&self) -> Result<Vec<DefType>, Vec<Diagnostic>> {
        let found = self.found_def_types();
        let duplicates = duplicates(&found);
        if !duplicates.is_empty() {
            return Err(merge_files(duplicates, self.warnings.clone()));
        }
        Ok(found.into_iter().map(|found| found.listed).collect())
    }

    /// Invokes the definition of path `path` made with the def-type of path
    /// `def_type`, both as written (`"package"`, `"dev.peios.testpkg"`,
    /// `"org.gnu.gmp-c++"`, `"org.golang.go-stage0-1.24"`), compared with
    /// the listed paths as text, the definition's alone if it is the only
    /// one of its path, else qualified by its directory, `"pkgs/a/main"`, or
    /// `"/main"` for the root's ([`Definition::qualified`]),
    /// with the arguments `args`, each the name of one of its invocation
    /// parameters and the value given for it, as if `DT PATH(NAME = VALUE,
    /// ...)` were called from a file of the definition's directory, each
    /// value a literal of its kind (see the module documentation, "Def-types
    /// by name"): the definition's file and the def-type's run first, each as a
    /// whole, with the files they need, unless they have run, and then the
    /// def-type is called with the definition's path, the definition and
    /// the arguments, each filling the parameter it names, each one given
    /// none given its default, its caller the directory of
    /// the definition's file. Returns the value
    /// the def-type returns, with the warnings of the whole workspace.
    /// Invoked again, the call runs again.
    ///
    /// # Errors
    ///
    /// In this order: the duplicates [`Workspace::def_types`] fails with,
    /// definitions under others among them, with the warnings; no def-type
    /// of path `def_type`, no definition of it of path `path`, `path` above
    /// some, or `path` alone defined in several directories
    /// ([`RequestError::Ambiguous`]), and the errors of the arguments that
    /// [`Workspace::arguments`] finds, a value of another type than its
    /// parameter's among them, each a [`RequestError`]; and the runtime
    /// error of a file that runs or of the call, as
    /// [`Workspace::run`] returns it, a part of it in the call itself,
    /// which has no source, located at the definition's path, labelled
    /// "invoked here".
    pub fn invoke(
        &mut self,
        def_type: &str,
        path: &str,
        args: &[(String, Given)],
    ) -> Result<Outcome, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        } = self.invoked(def_type, path)?;
        let given = args
            .iter()
            .map(|(name, value)| (name.as_str(), Argument::Given(value)));
        let args = self
            .given(&found, (def_type_cell.1, path), given)
            .map_err(InvokeError::Request)?;
        let Some(definition) = found.listed.definitions.get(at) else {
            return Err(InvokeError::Request(missing_definition(
                &found.listed,
                path,
            )));
        };
        // The files a reference to the definition needs, each run whole.
        self.run_needed(definition.file)
            .map_err(InvokeError::Diagnostics)?;
        self.run_needed(found.listed.file)
            .map_err(InvokeError::Diagnostics)?;
        let known = self.def_type_known(&found);
        self.call_definition(
            (definition, Invocation::Call(&args)),
            (def_type_cell, definition_cell),
            known,
        )
    }

    /// Reads the arguments `args` for the invocation parameters of the
    /// def-type of path `def_type`, to invoke its definition of path `path`
    /// with ([`Workspace::invoke`]), both as written: each the name of a
    /// parameter and the text of its value, read by the parameter's declared
    /// type (an impure def-type's either shape, `String | Future<String>`
    /// of a bare `String`, taken off), which must be one of `String`,
    /// `Int`, `Float` and `Bool` (see [`Given`]): a `String` the text as it
    /// is, an `Int` a decimal integer, with `-` before it if negative, in
    /// range, a `Float` a decimal number, finite, a `Bool` `true` or
    /// `false`. A name with no text is checked as one given, its value not
    /// known yet, and left out of what is returned: so a host can check a
    /// request before it knows every value. Runs nothing: what it finds
    /// wrong, invoking would. Returns each value read, with its parameter's
    /// name, in the order of `args`.
    ///
    /// # Errors
    ///
    /// In this order: the duplicates [`Workspace::def_types`] fails with, with
    /// the warnings; no def-type of path `def_type`, no definition of it of
    /// path `path`, `path` above some, or `path` alone defined in several
    /// directories; an invocation parameter that has
    /// no default and is not one of the four types, the first; then, for
    /// each argument in order, a name that is no invocation parameter's, a
    /// parameter named twice, a parameter given an argument that is not one
    /// of the four types, or a text that is not a value of its type; and
    /// every parameter that has no default given none, each a
    /// [`RequestError`].
    pub fn arguments(
        &self,
        def_type: &str,
        path: &str,
        args: &[(&str, Option<&str>)],
    ) -> Result<Vec<(String, Given)>, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            def_type_cell,
            ..
        } = self.invoked(def_type, path)?;
        let texts = args
            .iter()
            .map(|&(name, text)| (name, Argument::Text(text)));
        self.given(&found, (def_type_cell.1, path), texts)
            .map_err(InvokeError::Request)
    }

    /// Reads the field `field` of the meta of the definition of path `path`
    /// made with the def-type of path `def_type` (see [Metas](#metas)), as
    /// `(DT PATH).FIELD` would be read from a file of the definition's
    /// directory: the definition's file runs first, with the files it
    /// needs, unless they have run, and then the meta is read, running its
    /// `defmeta` as its marker says. Returns `None` if the def-type declares
    /// no meta or its meta has no field `field`. What `pekit` reads a
    /// package's versions with, to list them when none is given.
    ///
    /// # Errors
    ///
    /// As [`Workspace::invoke`], but for its arguments' errors: the
    /// duplicates, no such def-type or definition, and the runtime error of
    /// a file that runs or of the read.
    pub fn meta_field(
        &mut self,
        def_type: &str,
        path: &str,
        field: &str,
    ) -> Result<Option<Outcome>, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        } = self.invoked(def_type, path)?;
        let Some(definition) = found.listed.definitions.get(at) else {
            return Err(InvokeError::Request(missing_definition(
                &found.listed,
                path,
            )));
        };
        // The definition's parameter's type, its meta's.
        let func = self
            .types
            .fn_sig(self.types.strip(def_type_cell.1))
            .and_then(|sig| sig.params.get(1))
            .map(|&(_, ty)| self.types.strip(ty));
        let Some(func) = func else {
            return Ok(None);
        };
        let meta = self.types.fn_sig(func).and_then(|sig| sig.meta);
        let has_field = meta.is_some_and(|meta| {
            let record = self.types.strip(meta.ty);
            let name = self.scan.names.lookup(field);
            name.is_some_and(|name| self.types.field(record, name).is_some())
        });
        if !has_field {
            return Ok(None);
        }
        self.run_needed(definition.file)
            .map_err(InvokeError::Diagnostics)?;
        self.call_definition(
            (definition, Invocation::MetaField(field)),
            (def_type_cell, (definition_cell.0, func)),
            None,
        )
        .map(Some)
    }

    /// Which definition `value` names, if it is the closure of a def-type
    /// reference, `DT PATH` evaluated (see the module documentation,
    /// "Def-types by name"): its def-type's path as declared, whatever
    /// import or alias the reference names it by, and the definition's path
    /// as it writes it, found by the definition the closure captures, which
    /// is the closure its definition's cell holds. What a host that gets a
    /// reference back, from an invocation or a meta (a package's `build =
    /// build busybox`), reads to invoke it or read its meta by name. `None`
    /// for any other value, a closure that calls a definition included, and
    /// for a reference to no definition of the workspace's def-types. Runs
    /// nothing and changes nothing.
    #[must_use]
    pub fn definition_of(&self, value: &Value) -> Option<Referenced> {
        let Value::Fn(closure) = value else {
            return None;
        };
        if !closure.code.is_reference(closure.function) {
            return None;
        }
        let Some(Value::Fn(definition)) = closure
            .captures
            .get(crate::lower::REFERENCE_DEFINITION as usize)
        else {
            return None;
        };
        let holds = |file: u32, item: ItemId| {
            let cell = self.leaf_cell(file, item);
            let held = cell.and_then(|(slot, _)| self.frame.get(slot.index()));
            matches!(held, Some(Value::Fn(held)) if Rc::ptr_eq(held, definition))
        };
        self.found_def_types().into_iter().find_map(|found| {
            let (_, listed) = found
                .items
                .iter()
                .zip(&found.listed.definitions)
                .find(|&(&item, listed)| holds(listed.file, item))?;
            Some(Referenced {
                def_type: found.listed.path.clone(),
                path: listed.path.clone(),
                dir: listed.dir.clone(),
            })
        })
    }

    /// The top-level field or `def` of file `file` named `name`, a field of
    /// the file's own record, which another file reads as a member of its
    /// module (`pekit.environment` of `pekit.gcsl` at the root): its type
    /// and, if it is a function, what it may do and how many parameters it
    /// takes (see the module documentation, "Top-level functions by name").
    /// `None` if the file declares none such: no item of the name, or only
    /// a private one (`let`, `fn`), a dotted path under the name, or one
    /// shadowed in its own file.
    #[must_use]
    pub fn top_level(&self, file: u32, name: &str) -> Option<TopLevel> {
        let (item, ((_, ty), _)) = self.top_level_cell(file, name)?;
        let function = self.types.fn_sig(self.types.strip(ty));
        Some(TopLevel {
            ty: self.shown_top_level(file, item, ty),
            function: function.map(|sig| (sig.effect, sig.params.len())),
        })
    }

    /// Whether the top-level function of file `file` named `name`, as
    /// [`Workspace::top_level`] finds it, has returned for no arguments: as
    /// a call of it with none written in a file of `file`'s directory would
    /// find, its closure, which the file stored as it ran, holding a value
    /// for them in the workspace's memo tables (see the evaluator's
    /// "Memoised functions"). So only a memoised (`->>`) function's call
    /// that completed counts: one that failed has left nothing, and one
    /// still running nothing yet. The tables are the workspace's, which
    /// every run of a file, every invocation and every call it makes share,
    /// so a call made in any of them counts, once, for the workspace's life.
    /// False if the file has not run, or has no such function. Looks the
    /// tables up as such a call would, its caller the file's directory: a
    /// function whose call read `fileroot.caller` (see the evaluator's
    /// "Ambient inputs") counts only if it was called from that directory.
    /// Runs nothing and changes nothing.
    #[must_use]
    pub fn memoised(&self, file: u32, name: &str) -> bool {
        let Some((_, ((slot, _), _))) = self.top_level_cell(file, name) else {
            return false;
        };
        let Some(Value::Fn(closure)) = self.frame.get(slot.index()) else {
            return false;
        };
        let dir = self.scan.place(file).dir;
        self.memo.completed(closure, &[], dir)
    }

    /// Checks the call [`Workspace::call_composed`] makes of `functions`,
    /// top-level functions of file `file`, running nothing: whether each
    /// takes what the next returns, as the checker checks a call written in
    /// a file of `file`'s directory, `f(g())` for `["f", "g"]`. An empty
    /// `functions` calls nothing, which is well.
    ///
    /// # Errors
    ///
    /// As [`Workspace::call_composed`], but for the runtime errors: a name
    /// that is no top-level function of the file, or the diagnostics of the
    /// call's check.
    pub fn check_composed(&mut self, file: u32, functions: &[&str]) -> Result<(), InvokeError> {
        let Some((cells, at)) = self.composed(file, functions)? else {
            return Ok(());
        };
        self.lower_composed(&cells, at).map(|_| ())
    }

    /// Calls the composition of `functions`, top-level functions of file
    /// `file` as [`Workspace::top_level`] finds them, each given what the
    /// next returns and the last none, `f(g(h()))` for `["f", "g", "h"]`, as
    /// if written in a file of `file`'s directory, its caller: the file runs
    /// first, with the files it needs, unless they have run, as for
    /// [`Workspace::run`], and then the call, a call built rather than parsed
    /// that reads the functions' cells by names no source can write, as the
    /// call [`Workspace::invoke`] makes does (see the module documentation,
    /// "Top-level functions by name"), on the workspace's frame and memo
    /// tables, so a memoised function called before returns what it
    /// returned. Returns the value the first returns, with the warnings of
    /// the whole workspace; for an empty `functions`, `{}`, running nothing.
    /// Called again, the call runs again.
    ///
    /// # Errors
    ///
    /// A name that is no top-level function of the file,
    /// [`RequestError::NoFunction`], the first such, before anything runs;
    /// the runtime error of a file that runs, as [`Workspace::run`] returns
    /// it; and the diagnostics of the call's check, a function that does
    /// not take what the next returns, or its runtime error, each part of
    /// them in the call itself, which has no source, located at the first
    /// function's name, labelled "invoked here".
    pub fn call_composed(&mut self, file: u32, functions: &[&str]) -> Result<Outcome, InvokeError> {
        let Some((cells, at)) = self.composed(file, functions)? else {
            return Ok(Outcome {
                value: Value::Empty,
                warnings: self.warnings.clone(),
            });
        };
        self.run_needed(file).map_err(InvokeError::Diagnostics)?;
        let lowered = self.lower_composed(&cells, at)?;
        self.run_built(lowered, at)
    }

    /// The cells of `functions`, top-level functions of file `file`, as
    /// [`Workspace::top_level`] finds them, each with what a call of it
    /// needs to know of its parameters, and where the first's name is
    /// written; `None` for an empty `functions`.
    ///
    /// # Errors
    ///
    /// The first name that is no top-level function of the file.
    fn composed(&self, file: u32, functions: &[&str]) -> Result<Option<Composed>, InvokeError> {
        let mut cells = Vec::new();
        let mut at = None;
        for &name in functions {
            let found = self
                .top_level_cell(file, name)
                .filter(|&(_, ((_, ty), _))| self.types.fn_sig(self.types.strip(ty)).is_some());
            let Some((item, (cell, known))) = found else {
                let path = self.scan.files.get(file as usize);
                return Err(InvokeError::Request(RequestError::NoFunction {
                    file: path.map(|path| path.path.clone()).unwrap_or_default(),
                    name: name.to_owned(),
                }));
            };
            at = at.or_else(|| Some((file, self.top_level_span(file, item))));
            cells.push((cell, known));
        }
        Ok(at.map(|at| (cells, at)))
    }

    /// Checks and lowers the call of `cells`, each given what the next
    /// returns, the first's name written at `at` ([`Workspace::lower_built`]).
    ///
    /// # Errors
    ///
    /// The diagnostics of its check or lowering.
    fn lower_composed(
        &mut self,
        cells: &[CellOf],
        at: (u32, Span),
    ) -> Result<(Ir, Vec<Diagnostic>), InvokeError> {
        let bound: Vec<BoundCell> = (0..cells.len())
            .zip(cells)
            .map(|(number, (cell, known))| {
                let name = self.scan.names.intern(&format!("<function{number}>"));
                (name, *cell, known.clone())
            })
            .collect();
        let names: Vec<Name> = bound.iter().map(|&(name, _, _)| name).collect();
        self.lower_built(&bound, |all| composition(all, &names, at.1), at)
    }

    /// The top-level field or `def` of file `file` named `name`, as
    /// [`Workspace::top_level`] finds it: its item, its cell and what a call
    /// of it needs to know of its parameters, if it is a `def`'s.
    fn top_level_cell(&self, file: u32, name: &str) -> Option<(ItemId, CellOf)> {
        let ast = &self.scan.files.get(file as usize)?.ast;
        ast.items(ast.root()).iter().find_map(|&item| {
            let ItemKind::Field { path, .. } = ast.item(item).kind else {
                return None;
            };
            let [segment] = ast.path(path) else {
                return None;
            };
            let named = segment.name()?;
            if self.scan.names.get(named) != name {
                return None;
            }
            // A declaration a later one shadows in its own file is a leaf of
            // no tree.
            let cell = self.leaf_cell(file, item)?;
            let known = self
                .leaf_node(file, item)
                .and_then(|node| self.cell_known.get(node as usize).cloned().flatten());
            Some((item, (cell, known)))
        })
    }

    /// Where the name of the top-level item `item` of file `file` is
    /// written.
    fn top_level_span(&self, file: u32, item: ItemId) -> Span {
        let Some(source) = self.scan.files.get(file as usize) else {
            return Span::new(0, 0);
        };
        let ast = &source.ast;
        match ast.item(item).kind {
            ItemKind::Field { path, .. } => match ast.path(path) {
                [first, .., last] => first.span.join(last.span),
                [only] => only.span,
                [] => ast.item(item).span,
            },
            _ => ast.item(item).span,
        }
    }

    /// The type of the top-level item `item` of file `file`, whose type is
    /// `ty`, as [`TopLevel::ty`] shows it: its annotation as it is written,
    /// if it has one; else, for a function expression (a `def`'s), `fn(`,
    /// its parameters, each `NAME: TYPE`, and its marker and return type as
    /// they are written, `fn(env: Vm) -> {}`; else its type as a diagnostic
    /// shows it.
    fn shown_top_level(&self, file: u32, item: ItemId, ty: TyId) -> String {
        let names = &self.scan.names;
        let shown = || self.types.display(ty, names).to_string();
        let Some(source) = self.scan.files.get(file as usize) else {
            return shown();
        };
        let ast = &source.ast;
        let ItemKind::Field {
            ty: annotation,
            value,
            ..
        } = ast.item(item).kind
        else {
            return shown();
        };
        if let Some(annotation) = annotation {
            return ast.show_type_with(annotation, names);
        }
        let ExprKind::Fn(function) = ast.get(value).kind else {
            return shown();
        };
        let decl = ast.fn_decl(function);
        let Some(ret) = decl.ret else {
            return shown();
        };
        let params: Vec<String> = ast
            .params(decl.params)
            .iter()
            .map(|param| {
                let ty = ast.show_type_with(param.ty, names);
                format!("{}: {ty}", names.get(param.name))
            })
            .collect();
        format!(
            "fn({}{}{}",
            params.join(", "),
            decl.effect.params_end(),
            ast.show_type_with(ret, names)
        )
    }

    /// The def-type of path `def_type` and its definition of path `path`,
    /// as [`Workspace::invoke`] and [`Workspace::meta_field`] find them: the
    /// def-type, the definition's place among its definitions, and the two
    /// cells.
    ///
    /// # Errors
    ///
    /// The duplicates, with the warnings; no such def-type, or definition.
    fn invoked(&self, def_type: &str, path: &str) -> Result<ResolvedDefinition, InvokeError> {
        let mut found = self.found_def_types();
        let duplicates = duplicates(&found);
        if !duplicates.is_empty() {
            let errors = merge_files(duplicates, self.warnings.clone());
            return Err(InvokeError::Diagnostics(errors));
        }
        let Some(at) = found.iter().position(|found| found.listed.path == def_type) else {
            let mut def_types: Vec<String> = found.into_iter().map(|f| f.listed.path).collect();
            def_types.sort_unstable();
            return Err(InvokeError::Request(RequestError::NoDefType {
                name: def_type.to_owned(),
                def_types,
            }));
        };
        let found = found.swap_remove(at);
        let missing = || InvokeError::Request(missing_definition(&found.listed, path));
        let definitions = &found.listed.definitions;
        let at = chosen_definition(&found.listed, path).map_err(InvokeError::Request)?;
        let (Some(definition), Some(&item)) = (definitions.get(at), found.items.get(at)) else {
            return Err(missing());
        };
        let cells = (
            self.leaf_cell(found.listed.file, found.item),
            self.leaf_cell(definition.file, item),
        );
        let (Some(def_type_cell), Some(definition_cell)) = cells else {
            // A definition a later one of another path shadows in its own
            // directory, which is in no tree: no source can reach it either.
            // The later one lies above or under it, so the duplicates have
            // failed already; kept for safety.
            return Err(missing());
        };
        Ok(ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        })
    }

    /// Every def-type of the workspace's own files, with its definitions,
    /// as [`Workspace::def_types`] lists them, duplicates and all.
    fn found_def_types(&self) -> Vec<FoundDefType> {
        let names = &self.scan.names;
        let text = |segments: &[PathSegment]| {
            segments
                .iter()
                .filter_map(PathSegment::name)
                .map(|name| names.get(name))
                .collect::<Vec<_>>()
                .join(".")
        };
        let span = |segments: &[PathSegment]| match (segments.first(), segments.last()) {
            (Some(first), Some(last)) => first.span.join(last.span),
            _ => Span::new(0, 0),
        };
        let mut found = Vec::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        // Each definition, by its namespace's key: the def-type it is made
        // with may be declared in a file after its own.
        let mut definitions: Vec<(Name, Definition, ItemId)> = Vec::new();
        let files = (0u32..).zip(&self.scan.files);
        for (index, file) in files.skip(self.scan.standard as usize) {
            let ast = &file.ast;
            for &item in ast.items(ast.root()) {
                match ast.item(item).kind {
                    ItemKind::Field {
                        path,
                        function: true,
                        ..
                    } if ast.is_def_type(item) => {
                        let segments = ast.path(path);
                        let path = text(segments);
                        let number = u32::try_from(item.index()).unwrap_or(u32::MAX);
                        // Its namespace's key, as the scan made it (see
                        // [Def-types](#def-types)).
                        let key = names::namespace_key(&path, index, number);
                        by_key.insert(key, found.len());
                        let params = invocation_params(ast, item);
                        found.push(FoundDefType {
                            listed: DefType {
                                path,
                                file: index,
                                span: span(segments),
                                parameters: params
                                    .iter()
                                    .map(|param| names.get(param.name).to_owned())
                                    .collect(),
                                optional: params
                                    .iter()
                                    .map(|param| param.default.is_some())
                                    .collect(),
                                definitions: Vec::new(),
                            },
                            item,
                            written: params
                                .iter()
                                .map(|param| ast.show_type_with(param.ty, names))
                                .collect(),
                            items: Vec::new(),
                        });
                    }
                    ItemKind::DefTypeDef { def, .. } => {
                        let def = ast.def_type(def);
                        // Every definition of a workspace that checked has
                        // its def-type's key.
                        let Some(key) = def.key else {
                            continue;
                        };
                        let segments = ast.path(def.path);
                        let dir = self.scan.dirs.get(file.dir as usize);
                        let definition = Definition {
                            path: text(segments),
                            dir: dir.map(|dir| dir.path.clone()).unwrap_or_default(),
                            file: index,
                            span: span(segments),
                        };
                        definitions.push((key, definition, item));
                    }
                    _ => {}
                }
            }
        }
        for (key, definition, item) in definitions {
            let at = by_key.get(names.get(key)).copied();
            if let Some(def_type) = at.and_then(|at| found.get_mut(at)) {
                def_type.listed.definitions.push(definition);
                def_type.items.push(item);
            }
        }
        found
    }

    /// The values of `args`, each the name of an invocation parameter of the
    /// def-type `found`, whose type is `ty`, and an argument for it, for its
    /// definition of path `path`, in the order of `args`, each with its
    /// parameter's name, an argument with no text left out (see
    /// [`Workspace::arguments`]).
    ///
    /// # Errors
    ///
    /// In this order: the first invocation parameter, as it declares them,
    /// that has no default and is not one of the four types an argument can
    /// be given for, its type as it is written; for each argument in order,
    /// a name that is no invocation parameter's, a parameter already named,
    /// a parameter not of the four types, or a text that is not a value of
    /// its type, or a value that is not one, as [`Given`] shows it; then the
    /// invocation parameters that have no default given none.
    fn given<'a>(
        &self,
        found: &FoundDefType,
        (ty, path): (TyId, &str),
        args: impl Iterator<Item = (&'a str, Argument<'a>)>,
    ) -> Result<Vec<(String, Given)>, RequestError> {
        let listed = &found.listed;
        let types = self
            .types
            .fn_sig(ty)
            .and_then(|sig| sig.params.get(2..))
            .unwrap_or_default();
        let params: Vec<Invocable> = listed
            .parameters
            .iter()
            .zip(&found.written)
            .zip(&listed.optional)
            .zip(types)
            .map(|(((name, written), &optional), &(_, ty))| Invocable {
                name,
                written,
                optional,
                scalar: self.scalar(ty),
            })
            .collect();
        let def_type = || listed.path.clone();
        let cannot_be_given = |param: &Invocable| RequestError::CannotBeGiven {
            def_type: def_type(),
            parameter: param.name.to_owned(),
            ty: param.written.to_owned(),
        };
        if let Some(param) = params
            .iter()
            .find(|param| !param.optional && param.scalar.is_none())
        {
            return Err(cannot_be_given(param));
        }
        let mut given = vec![false; params.len()];
        let mut values = Vec::new();
        for (name, argument) in args {
            let at = params.iter().position(|param| param.name == name);
            let (Some(param), Some(seen)) = (
                at.and_then(|at| params.get(at)),
                at.and_then(|at| given.get_mut(at)),
            ) else {
                return Err(RequestError::UnknownParameter {
                    def_type: def_type(),
                    parameter: name.to_owned(),
                    parameters: listed.parameters.clone(),
                });
            };
            if mem::replace(seen, true) {
                return Err(RequestError::DuplicateArgument {
                    def_type: def_type(),
                    parameter: name.to_owned(),
                });
            }
            let Some(scalar) = param.scalar else {
                return Err(cannot_be_given(param));
            };
            let (value, text) = match argument {
                Argument::Text(None) => continue,
                Argument::Text(Some(text)) => (scalar.read(text), text.to_owned()),
                Argument::Given(value) => {
                    (scalar.fits(value).then(|| value.clone()), value.to_string())
                }
            };
            let Some(value) = value else {
                return Err(RequestError::CannotParse {
                    def_type: def_type(),
                    path: path.to_owned(),
                    parameter: name.to_owned(),
                    expected: scalar.name(),
                    text,
                });
            };
            values.push((name.to_owned(), value));
        }
        let missing: Vec<String> = params
            .iter()
            .zip(&given)
            .filter(|&(param, &given)| !given && !param.optional)
            .map(|(param, _)| param.name.to_owned())
            .collect();
        if !missing.is_empty() {
            return Err(RequestError::MissingArguments {
                def_type: def_type(),
                parameters: missing,
            });
        }
        Ok(values)
    }

    /// Which of the four types an argument can be given for (see [`Given`])
    /// an invocation parameter of type `ty` is, if one: its declared type,
    /// an impure def-type's either shape (`String | Future<String>` of a
    /// bare `String`) taken off, the same type as one of them, each a
    /// subtype of the other. A union, an explicit `Future`, a record, a
    /// list or a function is none.
    fn scalar(&self, ty: TyId) -> Option<Scalar> {
        let ty = match self.types.laziness(ty) {
            Laziness::Either => self.types.strip(ty),
            _ => ty,
        };
        Scalar::ALL.into_iter().find(|scalar| {
            self.types.is_subtype(scalar.ty(), ty) && self.types.is_subtype(ty, scalar.ty())
        })
    }

    /// The cell of the leaf the item `item` of file `index` declares, a
    /// `def` or a definition: the slot it is read from, its function's, and
    /// its type, once its file is checked; `None` if the item is a leaf of
    /// no tree, a later declaration shadowing it in its own directory.
    fn leaf_cell(&self, index: u32, item: ItemId) -> Option<(Slot, TyId)> {
        let node = self.leaf_node(index, item)?;
        Some((
            *self.cell_slots.get(node as usize)?,
            *self.cell_types.get(node as usize)?,
        ))
    }

    /// The merged node of the leaf the item `item` of file `index`
    /// declares, as [`Workspace::leaf_cell`] finds it.
    fn leaf_node(&self, index: u32, item: ItemId) -> Option<u32> {
        let file = self.scan.files.get(index as usize)?;
        let position = file
            .ast
            .items(file.ast.root())
            .iter()
            .position(|&at| at == item)?;
        let own = (*file.mentions.tree.item_leaf.get(position)?)?;
        file.global.get(own as usize).copied()
    }

    /// What a call of the def-type `found` needs to know of its parameters
    /// (see [`Context::known`]), its defaults among them, once its file is
    /// checked: so the call that invokes one of its definitions by name
    /// gives an invocation parameter it leaves out its default.
    fn def_type_known(&self, found: &FoundDefType) -> Option<Rc<KnownParams>> {
        let node = self.leaf_node(found.listed.file, found.item)?;
        self.cell_known.get(node as usize)?.clone()
    }

    /// Calls the def-type whose cell is `def_type` with the path of
    /// `definition`, the definition, whose cell is `definition_cell`, and
    /// the arguments `invoked` gives, their files run: the input
    /// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`, or the read of a
    /// field of the definition's meta, its names no source can write bound
    /// to the two cells,
    /// the def-type's with what a call of it needs to know of its
    /// parameters, `known`, so that the call is of a known callee and gives
    /// an invocation parameter it leaves out its default, checked from the
    /// workspace's slots, as an input of no workspace, and lowered as one
    /// placed in the definition's directory, its caller, numbered after
    /// every file (see [Def-types by name](#def-types-by-name)).
    fn call_definition(
        &mut self,
        (definition, invoked): (&Definition, Invocation),
        (def_type, definition_cell): ((Slot, TyId), (Slot, TyId)),
        known: Option<Rc<KnownParams>>,
    ) -> Result<Outcome, InvokeError> {
        let at = (definition.file, definition.span);
        let bound = (
            self.scan.names.intern(INVOKED_DEF_TYPE),
            self.scan.names.intern(INVOKED_DEFINITION),
        );
        let cells = [(bound.0, def_type, known), (bound.1, definition_cell, None)];
        let path = &definition.path;
        let lowered = self.lower_built(
            &cells,
            |names| invocation(names, (path, invoked), bound, at.1),
            at,
        )?;
        self.run_built(lowered, at)
    }

    /// Checks and lowers a call built rather than parsed, the input `build`
    /// makes, given the workspace's names, which it reads the cells of
    /// `bound` by, each by its name, one no source can write, bound in a
    /// scope of the workspace's slots as a field of the cell's slot, with its
    /// type and what a call of it needs to know of its parameters, so that a
    /// call of it is of a known callee and gives each optional parameter
    /// given none its default: checked as an input of no workspace, so that
    /// it looks nothing up in any file's view and needs no alias, and
    /// lowered as one placed in the directory of file `at.0`, its caller,
    /// numbered after every file (see [Def-types by
    /// name](#def-types-by-name)). Returns its IR, with its own warnings,
    /// located as [`invoked_here`] locates them. It takes no cell and
    /// commits nothing, so it changes nothing any file sees. What
    /// [`Workspace::invoke`], [`Workspace::meta_field`] and
    /// [`Workspace::call_composed`] run, and [`Workspace::check_composed`]
    /// checks.
    ///
    /// # Errors
    ///
    /// The diagnostics of its check or lowering, each part of them in the
    /// call itself, which has no source, located at `at.1` of file `at.0`,
    /// labelled "invoked here" ([`invoked_here`]), with the workspace's
    /// warnings.
    fn lower_built(
        &mut self,
        bound: &[BoundCell],
        build: impl FnOnce(Names) -> Ast,
        at: (u32, Span),
    ) -> Result<(Ir, Vec<Diagnostic>), InvokeError> {
        let call = count(self.scan.files.len());
        let ast = build(mem::take(&mut self.scan.names));
        let fields = bound
            .iter()
            .map(|(name, (slot, ty), known)| (*name, *slot, *ty, known.clone()));
        let scope = Scope::of_slots(self.slot_names.clone()).with_fields(fields);
        let place = self.scan.place(at.0);
        let lowered = Checker::with_scope(&ast, scope, &mut self.types)
            .check()
            .and_then(|mut output| {
                output.checked.resolution.set_place(place);
                session::lower_checked(&ast, output, &self.types, call)
            });
        let mut ast = ast;
        self.scan.names = ast.replace_names(Names::new());
        let (ir, output) = lowered.map_err(|errors| {
            let errors = invoked_here(errors, call, at);
            InvokeError::Diagnostics(merge_files(errors, self.warnings.clone()))
        })?;
        Ok((ir, invoked_here(output.warnings, call, at)))
    }

    /// Runs `ir`, a call [`Workspace::lower_built`] lowered, whose own
    /// warnings are `own`, on the workspace's frame and memo tables, as
    /// [`Workspace::run`] runs a file, and returns its value, with the
    /// warnings of the whole workspace and its own. Run again, it runs
    /// again, a call like any other.
    ///
    /// # Errors
    ///
    /// Its runtime error, a part of it in the call itself, which has no
    /// source, located at `at.1` of file `at.0`, labelled "invoked here"
    /// ([`invoked_here`]), with the warnings.
    fn run_built(
        &mut self,
        (mut ir, own): (Ir, Vec<Diagnostic>),
        at: (u32, Span),
    ) -> Result<Outcome, InvokeError> {
        let call = count(self.scan.files.len());
        // Numbered after every file, its slots end after theirs; kept
        // nonetheless, as a file's are (see [`Ir::reserve_frame`]).
        ir.reserve_frame(count(self.frame.len()));
        let context = (
            &mut self.memo,
            Some(&self.exec_result),
            &self.scan.dir_texts[..],
            &self.temp_dirs,
        );
        let mut warnings = self.warnings.clone();
        warnings.extend(own);
        match session::run_input(ir, &mut self.frame, context) {
            Ok(value) => Ok(Outcome { value, warnings }),
            Err(error) => {
                let errors = invoked_here(vec![error], call, at);
                Err(InvokeError::Diagnostics(merge_files(errors, warnings)))
            }
        }
    }
}

/// A cell a call built rather than parsed reads (see
/// [`Workspace::lower_built`]): the name, no source can write it, it is read
/// by, its slot and type, and what a call of it needs to know of its
/// parameters, if it is a `def`'s.
type BoundCell = (Name, (Slot, TyId), Option<Rc<KnownParams>>);

/// A top-level function's cell, its slot and type, with what a call of it
/// needs to know of its parameters, if it is a `def`'s (see
/// [`Workspace::top_level`]).
type CellOf = ((Slot, TyId), Option<Rc<KnownParams>>);

/// The functions of a composed call (see [`Workspace::call_composed`]): the
/// cell of each, in order, and where the first's name is written, its file
/// and span.
type Composed = (Vec<CellOf>, (u32, Span));

/// A top-level field or `def` of a file of a workspace, as
/// [`Workspace::top_level`] finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevel {
    /// Its type as it is written: its annotation, or for a `def`, `fn(`,
    /// its parameters, each `NAME: TYPE`, and its marker and return type,
    /// `fn() ->> Vm`; or, for a field with neither, as a diagnostic shows
    /// it.
    pub ty: String,
    /// If its type is a function type, what the function may do, as it is
    /// declared, pure (`:`), memoised (`->>`) or impure (`->`), and how many
    /// parameters it takes.
    pub function: Option<(ast::Effect, usize)>,
}

/// The tree of the input of a composed call (see
/// [`Workspace::call_composed`]), its names `names`: `<function0>(
/// <function1>( ... ))`, its result, each name one of `functions`, in
/// order, the last called with no argument and each other with what the
/// next returns, each node at `span`. An empty `functions` is the empty
/// block.
fn composition(names: Names, functions: &[Name], span: Span) -> Ast {
    let mut builder = AstBuilder::with_capacity(2 * functions.len() + 2);
    let mut inner = None;
    for &function in functions.iter().rev() {
        let callee = builder.alloc(Expr {
            kind: ExprKind::Name(function),
            span,
        });
        let arguments: Vec<Arg> = inner
            .into_iter()
            .map(|value| Arg {
                kind: ArgKind::Positional,
                value,
            })
            .collect();
        let args = builder.push_args(&arguments);
        inner = Some(builder.alloc(Expr {
            kind: ExprKind::Call { callee, args },
            span,
        }));
    }
    let items = match inner {
        Some(call) => vec![builder.alloc_item(Item {
            kind: ItemKind::Result(call),
            span,
        })],
        None => Vec::new(),
    };
    let root = builder.push_item_list(&items);
    builder.finish(root, names, Strings::new())
}

/// The index among the definitions made with the def-type `listed` of the
/// one `asked` names (see the module documentation, "Def-types by name"):
/// a path qualified by a directory, `dir/sub/name` or `/name` for the root,
/// names the definition of that path in that directory; a path alone, the
/// definition of that path, if it is the only one.
///
/// # Errors
///
/// [`missing_definition`]'s, if none is of the path, in the directory if
/// one is given; and if a path alone is several definitions' path, in
/// directories that cannot see each other, [`RequestError::Ambiguous`].
fn chosen_definition(listed: &DefType, asked: &str) -> Result<usize, RequestError> {
    let (dir, path) = split_qualified(asked);
    let mut matching = (0usize..)
        .zip(&listed.definitions)
        .filter(|(_, definition)| {
            definition.path == path && dir.is_none_or(|dir| definition.dir == dir)
        });
    let first = matching.next();
    let others: Vec<usize> = matching.map(|(at, _)| at).collect();
    match first {
        None => Err(missing_definition(listed, asked)),
        Some((at, _)) if others.is_empty() => Ok(at),
        Some((at, _)) => {
            let mut candidates: Vec<String> = std::iter::once(at)
                .chain(others)
                .filter_map(|at| listed.definitions.get(at))
                .map(Definition::qualified)
                .collect();
            candidates.sort_unstable();
            Err(RequestError::Ambiguous {
                def_type: listed.path.clone(),
                path: path.to_owned(),
                candidates,
            })
        }
    }
}

/// The directory and the path of `asked`, a definition's path as asked for:
/// the text before its last `/`, without a leading `/` (so `""`, the root,
/// for `/name`), and the text after it; `None` and `asked` for a path with
/// no `/`, which a definition's path never has.
fn split_qualified(asked: &str) -> (Option<&str>, &str) {
    match asked.rsplit_once('/') {
        Some((dir, path)) => (Some(dir.strip_prefix('/').unwrap_or(dir)), path),
        None => (None, asked),
    }
}

/// The names the definitions made with the def-type `listed` are asked for
/// by, sorted: each path alone, but those several definitions have, each
/// qualified by its directory ([`Definition::qualified`]).
fn definition_names(listed: &DefType) -> Vec<String> {
    let definitions = &listed.definitions;
    let shared = |definition: &Definition| {
        definitions
            .iter()
            .filter(|other| other.path == definition.path)
            .count()
            > 1
    };
    let mut names: Vec<String> = definitions
        .iter()
        .map(|definition| {
            if shared(definition) {
                definition.qualified()
            } else {
                definition.path.clone()
            }
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The error of `asked`, which names no definition made with the def-type
/// `listed` (see [`chosen_definition`]): "`dev.peios` is not a `package`
/// definition" if it is above some of them, in its directory if it gives
/// one, else "`package` has no definition `dev.peios.nope`".
fn missing_definition(listed: &DefType, asked: &str) -> RequestError {
    let definitions = definition_names(listed);
    let (dir, path) = split_qualified(asked);
    let above = listed.definitions.iter().any(|definition| {
        lies_under(&definition.path, path) && dir.is_none_or(|dir| definition.dir == dir)
    });
    let (def_type, path) = (listed.path.clone(), asked.to_owned());
    if above {
        RequestError::NotADefinition {
            def_type,
            path,
            definitions,
        }
    } else {
        RequestError::NoDefinition {
            def_type,
            path,
            definitions,
        }
    }
}

/// The duplicates among `found` (see [`Workspace::def_types`]): an error for
/// each path of which two def-types or more are declared, for each path of
/// which a def-type has two definitions or more in directories that see
/// each other ([`sees`]), and for each definition that lies under another
/// of its def-type in such a directory ([`overlaps`]), in path then source
/// order of the first, or of the definition below.
fn duplicates(found: &[FoundDefType]) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let declared = found.iter().map(|found| {
        (
            found.listed.path.as_str(),
            found.listed.file,
            found.listed.span,
        )
    });
    for group in groups(declared) {
        errors.extend(duplicate(
            &group,
            |path, times| format!("def-type `{path}` is declared {times} times in the workspace"),
            "also declared here",
            "a def-type invoked by name must be the only one of its name in the workspace",
        ));
    }
    for found in found {
        let definitions = &found.listed.definitions;
        let defined = definitions
            .iter()
            .map(|definition| (definition.path.as_str(), definition.file, definition.span));
        let def_type = &found.listed.path;
        for group in groups(defined) {
            // The group's definitions, by where they are, gathered into those
            // whose directories see each other, one through another.
            let dir_of = |(_, file, span): &Declared| {
                definitions
                    .iter()
                    .find(|definition| definition.file == *file && definition.span == *span)
                    .map_or("", |definition| definition.dir.as_str())
            };
            for chain in chains(&group, |a, b| sees(dir_of(a), dir_of(b))) {
                errors.extend(duplicate(
                    &chain,
                    |path, times| {
                        format!(
                            "`{def_type} {path}` is defined {times} times in directories that \
                             see each other"
                        )
                    },
                    "also defined here",
                    "a definition invoked by name must be the only one of its path in its \
                     directory, the directories above it and those below it",
                ));
            }
        }
        errors.extend(overlaps(&found.listed));
    }
    errors.sort_by_key(|error| (error.input, error.span.start));
    errors
}

/// The errors of the definitions made with the def-type `listed` that lie
/// under others of it (see [`Workspace::def_types`]): for each definition
/// whose path is below another's, `app.core` below `app`, "`package
/// app.core` lies under another `package` definition", at it, labelled
/// "`app` is defined here" at each definition above it, in the order of
/// `listed`'s.
fn overlaps(listed: &DefType) -> Vec<Diagnostic> {
    let definitions = &listed.definitions;
    let def_type = &listed.path;
    let mut errors = Vec::new();
    for definition in definitions {
        let mut above = definitions
            .iter()
            .filter(|other| {
                lies_under(&definition.path, &other.path) && sees(&definition.dir, &other.dir)
            })
            .peekable();
        if above.peek().is_none() {
            continue;
        }
        let path = &definition.path;
        let mut error = Diagnostic::error(
            format!("`{def_type} {path}` lies under another `{def_type}` definition"),
            definition.span,
        )
        .in_input(definition.file)
        .with_note(
            "a definition invoked by name must not lie under another in its directory, the \
             directories above it or those below it",
        );
        for other in above {
            let text = format!("`{}` is defined here", other.path);
            error = error.with_context_in(other.file, other.span, text);
        }
        errors.push(error);
    }
    errors
}

/// Whether the directories of root-relative paths `a` and `b` see each
/// other's definitions, one through another: whether they are one, or one
/// is above the other (the root, `""`, is above every directory). Two
/// directories neither of which is above the other, siblings or cousins,
/// see nothing of each other's.
fn sees(a: &str, b: &str) -> bool {
    let above = |upper: &str, lower: &str| {
        upper.is_empty()
            || lower
                .strip_prefix(upper)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    a == b || above(a, b) || above(b, a)
}

/// `group`, the declarations of one path, gathered into the chains of those
/// `related`, one through others: each chain in the order of `group`, the
/// chains in the order of their first.
fn chains<'a>(
    group: &[Declared<'a>],
    related: impl Fn(&Declared<'a>, &Declared<'a>) -> bool,
) -> Vec<Vec<Declared<'a>>> {
    let mut chains: Vec<Vec<Declared<'a>>> = Vec::new();
    for &entry in group {
        let (joined, apart): (Vec<_>, Vec<_>) = chains
            .into_iter()
            .partition(|chain| chain.iter().any(|other| related(other, &entry)));
        let mut merged: Vec<Declared<'a>> = joined.into_iter().flatten().collect();
        merged.push(entry);
        chains = apart;
        chains.push(merged);
    }
    chains.sort_by_key(|chain| chain.first().map(|&(_, file, span)| (file, span.start)));
    for chain in &mut chains {
        chain.sort_by_key(|&(_, file, span)| (file, span.start));
    }
    chains
}

/// Whether the path `path` lies under the path `above`, its segments
/// following all of `above`'s: `app.core` under `app`, but not `apple`.
fn lies_under(path: &str, above: &str) -> bool {
    path.strip_prefix(above)
        .is_some_and(|rest| rest.starts_with('.'))
}

/// A declaration of a path by [`duplicates`]: its path, its file and where.
type Declared<'a> = (&'a str, u32, Span);

/// `declared`, grouped by path, each group in the order of `declared`, the
/// groups in the order of their first.
fn groups<'a>(declared: impl Iterator<Item = Declared<'a>>) -> Vec<Vec<Declared<'a>>> {
    let mut groups: Vec<Vec<Declared<'a>>> = Vec::new();
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for entry in declared {
        let at = *by_path.entry(entry.0).or_insert(groups.len());
        match groups.get_mut(at) {
            Some(group) => group.push(entry),
            None => groups.push(vec![entry]),
        }
    }
    groups
}

/// The error of `group`, the declarations of one path, if there are two or
/// more: `message` of the path and their number, at the first, labelled
/// `label` at each other, with `note`.
fn duplicate(
    group: &[Declared],
    message: impl Fn(&str, usize) -> String,
    label: &str,
    note: &str,
) -> Option<Diagnostic> {
    let ([(path, file, span), rest @ ..], true) = (group, group.len() > 1) else {
        return None;
    };
    let mut error = Diagnostic::error(message(path, group.len()), *span)
        .in_input(*file)
        .with_note(note);
    for &(_, file, span) in rest {
        error = error.with_context_in(file, span, label);
    }
    Some(error)
}

/// The invocation parameters of the def-type whose `def` is the item `item`
/// of `ast`: its parameters after its second, the definition's path's and
/// the definition's, as it declares them; none if it declares none, or if
/// the item is no `def` of a function.
fn invocation_params(ast: &Ast, item: ItemId) -> &[Param] {
    let ItemKind::Field { value, .. } = ast.item(item).kind else {
        return &[];
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        return &[];
    };
    ast.params(ast.fn_decl(function).params)
        .get(2..)
        .unwrap_or_default()
}

/// The tree of the input of the call that invokes a definition by name
/// (see [Def-types by name](#def-types-by-name)), its names `names`:
/// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`, its result, PATH
/// `path`, each named argument one of `args`, in order, its value a literal
/// of its kind, the two names those of `bound`, each node at `span`.
fn invocation(
    mut names: Names,
    (path, invoked): (&str, Invocation),
    bound: (Name, Name),
    span: Span,
) -> Ast {
    let args = match invoked {
        Invocation::Call(args) => args,
        Invocation::MetaField(field) => {
            return meta_read(names.intern(field), names, bound.1, span);
        }
    };
    let mut builder = AstBuilder::with_capacity(4 + args.len());
    let mut strings = Strings::new();
    let callee = builder.alloc(Expr {
        kind: ExprKind::Name(bound.0),
        span,
    });
    let name = builder.alloc(Expr {
        kind: ExprKind::Str(strings.push(path)),
        span,
    });
    let definition = builder.alloc(Expr {
        kind: ExprKind::Name(bound.1),
        span,
    });
    // The path and the definition positionally, then the arguments by name.
    let mut arguments = vec![
        Arg {
            kind: ArgKind::Positional,
            value: name,
        },
        Arg {
            kind: ArgKind::Positional,
            value: definition,
        },
    ];
    for (name, given) in args {
        let kind = match given {
            Given::Str(text) => ExprKind::Str(strings.push(text)),
            Given::Int(value) => ExprKind::Int(*value),
            Given::Float(value) => ExprKind::Float(*value),
            Given::Bool(value) => ExprKind::Bool(*value),
            Given::Quantity(Quantity::Duration, amount) => ExprKind::Duration(*amount),
            Given::Quantity(Quantity::Size, amount) => ExprKind::Size(*amount),
        };
        let value = builder.alloc(Expr { kind, span });
        arguments.push(Arg {
            kind: ArgKind::Named {
                name: names.intern(name),
                span,
                shorthand: false,
            },
            value,
        });
    }
    let args = builder.push_args(&arguments);
    let call = builder.alloc(Expr {
        kind: ExprKind::Call { callee, args },
        span,
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Result(call),
        span,
    });
    let root = builder.push_item_list(&[item]);
    builder.finish(root, names, strings)
}

/// What the input [`Workspace::call_definition`] runs does with the
/// definition: call its def-type with it and these arguments, or read this
/// field of its meta.
#[derive(Clone, Copy)]
enum Invocation<'a> {
    /// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`.
    Call(&'a [(String, Given)]),
    /// `<definition>.FIELD`.
    MetaField(&'a str),
}

/// The tree of the input that reads the field `field` of a definition's
/// meta (see [`Workspace::meta_field`]), its names `names`:
/// `<definition>.FIELD`, its result, the name that of `definition`, each
/// node at `span`.
fn meta_read(field: Name, names: Names, definition: Name, span: Span) -> Ast {
    let mut builder = AstBuilder::with_capacity(2);
    let read = builder.alloc(Expr {
        kind: ExprKind::Name(definition),
        span,
    });
    let field = builder.alloc(Expr {
        kind: ExprKind::Field {
            expr: read,
            name: field,
        },
        span,
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Result(field),
        span,
    });
    let root = builder.push_item_list(&[item]);
    builder.finish(root, names, Strings::new())
}

/// `diagnostics` of the call that invokes a definition by name, input
/// number `call`, as its caller can show them: the call has no source, so
/// every diagnostic and label located in it, of input `call` or of no input,
/// is located instead at `at`, the definition's path in its file, labelled
/// "invoked here" (see [Def-types by name](#def-types-by-name)).
fn invoked_here(
    mut diagnostics: Vec<Diagnostic>,
    call: u32,
    (file, span): (u32, Span),
) -> Vec<Diagnostic> {
    const INVOKED: &str = "invoked here";
    for diagnostic in &mut diagnostics {
        let own = diagnostic.input.is_none_or(|input| input == call);
        for label in &mut diagnostic.labels {
            if label.input.map_or(own, |input| input == call) {
                label.input = Some(file);
                label.span = span;
                INVOKED.clone_into(&mut label.text);
            }
        }
        if own {
            diagnostic.input = Some(file);
            diagnostic.span = span;
            diagnostic.label = Some(INVOKED.into());
        }
    }
    diagnostics
}

/// `diagnostics` of the file `index` of a scan of one file beside the
/// standard library, as the diagnostics of the input being run name it: by
/// no input, as a label in it does, a label in a standard file keeping that
/// file's number, which is its input number in a session too.
#[must_use]
pub fn being_run(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        if diagnostic.input == Some(index) {
            diagnostic.input = None;
        }
        for label in &mut diagnostic.labels {
            if label.input == Some(index) {
                label.input = None;
            }
        }
    }
    diagnostics
}

/// The standard library as a session ran it once, as it started: what each
/// of its inputs is scanned and checked beside ([`check_input`]).
#[derive(Clone, Debug)]
pub(crate) struct Standard {
    /// The number of its cells, the built-in functions' and every standard
    /// file's nodes, which are the first nodes of the scan of every input,
    /// which loads every standard file too, an input's after them. (A
    /// standard file that is a value file has no cell of its value here;
    /// none is.)
    cells: u32,
    /// The type of each of its cells.
    cell_types: Vec<TyId>,
    /// The slot each of its cells is read from: its own, or for a `def` its
    /// function's.
    cell_slots: Vec<Slot>,
    /// For each of its cells that is a `def`'s, what a call of it needs to
    /// know of its parameters.
    cell_known: Vec<Option<Rc<KnownParams>>>,
    /// The table of the directories of the run ([`Scan::locate`]): the
    /// session's root, the working directory it started in, first, at index
    /// 0, where every input is, both its `fileroot.this` and its
    /// `fileroot.workspace`, and the standard files' `<stdlib>` after it.
    /// Every input's code names its directory by it, the root's, and its run
    /// reads it, so no input needs it extended.
    pub(crate) dirs: Vec<Rc<str>>,
}

/// What a session continues from once it has run the standard library
/// ([`start`]): the standard library's cells, and the names, types, frame,
/// memo tables, temporary directories and names of slots of the run. (The
/// standard library has no warnings to keep.)
#[derive(Debug)]
pub(crate) struct Started {
    /// The standard library's cells.
    pub(crate) standard: Standard,
    /// The names interned.
    pub(crate) names: Names,
    /// The types interned.
    pub(crate) types: TypeTable,
    /// The top level's frame, the standard library's cells and slots
    /// written.
    pub(crate) frame: Vec<Value>,
    /// The memo tables of the run.
    pub(crate) memo: MemoTables,
    /// The layout of the record `exec` returns, built once for the run.
    pub(crate) exec_result: Rc<Layout>,
    /// The temporary directories of the run, which `fs.temp_dir` makes, for
    /// the session to own from then on.
    pub(crate) temp_dirs: TempDirs,
    /// The name of each slot of the frame.
    pub(crate) slot_names: Vec<Name>,
}

/// Checks and runs the standard library as a session starts: through the
/// driver of a workspace with no file of its own, whose root is `root`, the
/// session's, that loads every standard file, each a scope of its own, the
/// built-in functions' cells written as the frame is laid out and every
/// standard file run.
///
/// # Errors
///
/// The diagnostics of the standard library, which are a bug.
pub(crate) fn start(root: &str) -> Result<Started, Vec<Diagnostic>> {
    let mut workspace = check_loading((&[], Placing::Workspace(&[])), Library::EVERY, root).1?;
    for file in 0..workspace.scan.standard {
        workspace.run(file)?;
    }
    let Workspace {
        scan,
        frame,
        memo,
        exec_result,
        temp_dirs,
        types,
        slot_names,
        cell_types,
        cell_slots,
        cell_known,
        ..
    } = workspace;
    Ok(Started {
        standard: Standard {
            cells: count(scan.cells()),
            cell_types,
            cell_slots,
            cell_known,
            dirs: scan.dir_texts,
        },
        names: scan.names,
        types,
        frame,
        memo,
        exec_result,
        temp_dirs,
        slot_names,
    })
}

/// Checks and lowers `source`, session input number `input`, beside the
/// standard library `standard`, from `scope`, the session's: scanned as a
/// lone file is ([`check_lone`]), its names interned into `names` and its
/// types into `types`, so that it may import `builtins` and the standard
/// files and has the prelude's names as the yielding last source of its
/// view; its cells, after the standard library's, take the slots after the
/// scope's ([`Scan::slot`]), and its check is a session input's, which
/// reads the scope's variables as a session's input does (see
/// [`crate::session`]). It inherits what `earlier` holds of the session's
/// earlier inputs: their imports, as the imports of its own that follow its
/// `import`s ([`inherit_imports`]), and their types, as the last source of
/// its view of types, its own types defined in their store after theirs
/// (see [Types](#types)). Its diagnostics name it as the input being run,
/// with no input number; a label in a standard file names that file's.
/// Returns, with its IR and its check's output, what a later input
/// inherits, once it has run: the imports it has, its own and those it
/// inherited and did not shadow, each with no item or span of this
/// input's, and its view of types, each type with its declaration's number
/// in the store.
///
/// # Errors
///
/// The diagnostics of the scan, the check or the lowering.
pub(crate) fn check_input(
    standard: &Standard,
    (source, earlier): (&SourceFile, Earlier),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    input: u32,
) -> Result<(Ir, CheckOutput, Inherited), Vec<Diagnostic>> {
    let (lowered, inherited) = check_input_then(
        standard,
        (source, earlier, input),
        (names, types),
        scope,
        |ast, checked, types| {
            checked.and_then(|output| session::lower_checked(ast, output, types, input))
        },
    )?;
    let (ir, output) = lowered?;
    Ok((ir, output, inherited))
}

/// What a session's input inherits from the session's earlier inputs (see
/// [`check_input`]).
#[derive(Debug)]
pub(crate) struct Earlier<'a> {
    /// Their imports that no later input shadowed.
    pub(crate) imports: &'a [Alias],
    /// Their types that no later input shadowed, each with its
    /// declaration's number in `store`.
    pub(crate) types: &'a [(Name, u32)],
    /// The definitions of their types, which the input's are added to.
    pub(crate) store: &'a mut TypeStore,
    /// Their def-types that no later input shadowed, each by its path, with
    /// its namespace's key (see [Def-types](#def-types)).
    pub(crate) def_types: &'a [(Vec<Name>, Name)],
}

/// What a later input inherits of a session's input once it has run: its
/// imports, its view of types and the def-types it declares, each by its
/// path with its namespace's key (see [`check_input`]).
pub(crate) type Inherited = (Vec<Alias>, Vec<(Name, u32)>, Vec<(Vec<Name>, Name)>);

/// [`check_input`] up to its check, whose outcome `then` is given with the
/// input's tree, its names in it, and the types: what lowering is done by,
/// or a test looks at. Returns what `then` does, with what a later input
/// inherits as [`check_input`] does. The input's `type` items are checked
/// before it, as a workspace's are ([`check_types`]), what is wrong in
/// them among the check's errors.
///
/// # Errors
///
/// The diagnostics of the scan.
fn check_input_then<R>(
    standard: &Standard,
    (source, earlier, input): (&SourceFile, Earlier, u32),
    (names, types): (&mut Names, &mut TypeTable),
    scope: Scope,
    then: impl FnOnce(&Ast, Result<CheckOutput, Vec<Diagnostic>>, &mut TypeTable) -> R,
) -> Result<(R, Inherited), Vec<Diagnostic>> {
    // Every standard file, as the session's start loaded them, so that its
    // cells are numbered as they were there.
    let mut loaded = Vec::new();
    let placed = (
        std::slice::from_ref(source),
        Placing::Lone(earlier.imports, true),
    );
    let store = earlier.store;
    let mut scan = scan_into_with(
        placed,
        Library::EVERY,
        (names, &mut loaded),
        (store, earlier.types),
        (earlier.def_types, Some(input)),
    )
    .map_err(|errors| being_run(errors, count(loaded.len())))?;
    // In the session's root, as its start located the standard files.
    scan.locate(standard.dirs.first().map_or("/", |root| root));
    let index = scan.standard;
    // The input's cells, but its `def`s', take the slots past the scope's,
    // in order.
    let cells = scan.cells();
    let cell_names = scan.cell_names();
    let mut next = count(scope.slot_count());
    let mut taken = Vec::new();
    let relocated = (standard.cells..count(cells))
        .map(|cell| {
            if scan.is_def(cell) {
                return Slot::NONE;
            }
            if let Some(&name) = cell_names.get(cell as usize) {
                taken.push(name);
            }
            next = next.saturating_add(1);
            Slot::at(next - 1)
        })
        .collect();
    scan.relocation = Some((standard.cells, relocated));
    let mut cell_types = standard.cell_types.clone();
    cell_types.resize(cells, TyId::ERROR);
    let mut cell_slots = standard.cell_slots.clone();
    cell_slots.extend((count(cell_slots.len())..count(cells)).map(|cell| scan.slot(cell)));
    let mut cell_known = standard.cell_known.clone();
    cell_known.resize(cells, None);
    let scope = scope.with_slots(taken);
    let file_names = mem::take(&mut scan.names);
    let Some(file) = scan.files.get_mut(index as usize) else {
        *names = file_names;
        *store = mem::take(&mut scan.type_store);
        return Err(Vec::new());
    };
    file.ast.replace_names(file_names);
    let finished = scan.files.get(index as usize).and_then(|file| {
        let reads = (&cell_slots[..], &cell_known[..]);
        let context = Context::new(&scan, index, &cell_types, reads)?;
        let context = Context {
            session: true,
            ..context
        };
        let declared = if file.mentions.types.is_empty() {
            Vec::new()
        } else {
            // A session's input with a syntax error is not checked.
            check_types(&file.ast, types, false)
        };
        let checked = Checker::in_workspace(&file.ast, scope, types, &context).check();
        let checked = match (declared.is_empty(), checked) {
            (true, checked) => checked,
            (false, Ok(output)) => Err(diagnostic::merge(declared, output.warnings)),
            (false, Err(found)) => Err(diagnostic::merge(declared, found)),
        };
        Some(then(&file.ast, checked, types))
    });
    let mut imports = Vec::new();
    let mut view = Vec::new();
    let mut def_types = Vec::new();
    if let Some(file) = scan.files.get_mut(index as usize) {
        *names = file.ast.replace_names(Names::new());
        imports.extend(file.aliases.iter().filter(|a| !a.implicit).map(|a| Alias {
            span: Span::new(0, 0),
            item: None,
            inherited: true,
            ..*a
        }));
        view.extend(file.type_view.iter().map(|(&name, &decl)| (name, decl)));
        view.sort_unstable();
        def_types = declared_def_types(file, names, input);
    }
    *store = mem::take(&mut scan.type_store);
    finished
        .map(|r| (r, (imports, view, def_types)))
        .ok_or_else(Vec::new)
}

/// The def-types `file`, session input number `input` whose names are
/// `names`, declares: each `def` marked `#[def_type]` that stands, by its
/// path, with its namespace's key, as [`name_namespaces`] makes it (see
/// [Def-types](#def-types)).
fn declared_def_types(file: &File, names: &mut Names, input: u32) -> Vec<(Vec<Name>, Name)> {
    let mut found = Vec::new();
    for node in &file.mentions.tree.nodes {
        let Some(leaf) = node.leaf else {
            continue;
        };
        let ItemKind::Field {
            path,
            function: true,
            ..
        } = file.ast.item(leaf.item).kind
        else {
            continue;
        };
        if !file.ast.is_def_type(leaf.item) {
            continue;
        }
        let path: Vec<Name> = file
            .ast
            .path(path)
            .iter()
            .filter_map(PathSegment::name)
            .collect();
        let shown: Vec<&str> = path.iter().map(|&name| names.get(name)).collect();
        let item = u32::try_from(leaf.item.index()).unwrap_or(u32::MAX);
        let text = names::namespace_key(&shown.join("."), input, item);
        found.push((path, names.intern(&text)));
    }
    found
}

/// For the tests of the other modules, which check a text alone: `text`
/// checked as a session's first input is, beside the standard library, so
/// that the prelude's names, `len` among them, are its, with the tree it
/// was parsed to, which holds every name, and the types. Its top-level
/// slots come after the standard library's.
#[cfg(test)]
pub(crate) fn check_beside_prelude(
    text: &str,
) -> (Ast, TypeTable, Result<CheckOutput, Vec<Diagnostic>>) {
    let started = start("/").unwrap();
    let (mut names, mut types) = (started.names, started.types);
    let source = SourceFile::new("test.gcsl", text).unwrap();
    let scope = Scope::of_slots(started.slot_names);
    let earlier = Earlier {
        imports: &[],
        types: &[],
        store: &mut TypeStore::new(),
        def_types: &[],
    };
    // A session's first input, after the standard files.
    let input = count(crate::STANDARD.len());
    let checked = check_input_then(
        &started.standard,
        (&source, earlier, input),
        (&mut names, &mut types),
        scope,
        |ast, checked, _| (ast.clone(), checked),
    );
    match checked {
        Ok(((ast, checked), _)) => (ast, types, checked),
        Err(errors) => panic!("{text}: {errors:?}"),
    }
}

#[cfg(test)]
mod tests;

/// The duration or the size `text` is, if it is one: one of its literals,
/// read by the lexer as a program's is, with `-` before it if negative, and
/// nothing else, not even a space.
fn quantity_literal(text: &str) -> Option<(Quantity, i64)> {
    let (negative, literal) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let source = SourceFile::new("<argument>", literal).ok()?;
    let (tokens, _, _) = crate::lexer::tokenize(&source).ok()?;
    let (quantity, amount) = match tokens.as_slice() {
        [first, end] if end.kind == TokenKind::Eof && first.span.start == 0 => match first.kind {
            TokenKind::Duration(amount) => (Quantity::Duration, amount),
            TokenKind::Size(amount) => (Quantity::Size, amount),
            _ => return None,
        },
        _ => return None,
    };
    let amount = if negative {
        amount.checked_neg()?
    } else {
        amount
    };
    Some((quantity, amount))
}
