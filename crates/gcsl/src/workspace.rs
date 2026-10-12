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
//! # Imports
//!
//! An `import` gives a file the rest of the workspace, by name. Its path is
//! absolute from the root: each segment but the last a directory, the last
//! a directory or a file's stem (`import lib.x` is `lib/x.gcsl`); the root
//! directory itself has no path. The scan resolves every import of every
//! file, mentioned or not ([`scan_with`]): a path that names nothing is "no
//! module `a.b`", with a note listing what the last directory that exists
//! has (its subdirectories, with `/`, then its file stems), or saying that
//! the directory is a workspace of its own. The library cannot see that
//! from the files alone, so [`scan_with`] and [`check_with`] take the
//! root-relative directories the caller left out as nested workspaces;
//! [`scan`] and [`check`] pass none. A directory and a file of one name are
//! an error once per pair, at the first import that names them.
//!
//! An imported name is an [`Alias`] of the file ([`File::aliases`]), naming
//! a [`Target`]: a directory's merged tree without its subdirectories, a
//! file's own tree (or, for a value file, its value), or a root of either
//! (`import x from lib`, and each root of `import * from lib`). A whole
//! import (`import lib`, `import lib.x as f`) is a private top-level name
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
//! - A qualified name, `t.PackageDef` ([`Scan::qualified_type`]), names
//!   through a whole import of the file (`import lib.types as t`) a type of
//!   the module it names: "`t` has no type `Foo`", with a note listing its
//!   types, if it has none of that name; "unknown type `a.b`" if `a` is no
//!   module the file imports whole; and "`t.Foo` is a type, not a module"
//!   for a path that goes on after a type. A module's name written as a
//!   type, `t`, is "`t` is a module, not a type", and a type applied to a
//!   type argument "type `Foo` takes no type argument" ([`Scan::type_links`]).
//!
//! Then the types are defined and the files' trees expanded
//! ([`define_types`]): each declaration's definition is copied into the
//! scan's [`TypeStore`], each name in it of another declared type a
//! reference to it, as the declaring file's view resolves it; the
//! references are a graph, whose cycles are each "type `A` refers to
//! itself", at the first declaration of the cycle in path then source
//! order, labelled at each step ("`B` uses `A` here", in `B`'s file);
//! every declaration is settled, those it refers to first, a reference
//! made what it refers to and a spread merged, one on a cycle, with a
//! spread's problem or nested deeper than [`crate::MAX_DEPTH`] once
//! expanded erroneous; and each file's tree is expanded
//! ([`Ast::expand_types`]): each name of a declared type in it made a copy
//! of the definition, as if written there, each record type with spreads
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
//! invocation parameters, as it declares them, and every definition made
//! with it ([`Definition`]): found by a search of every file, whichever
//! views see it, so a definition in a directory the root does not see is
//! found, and one a later one shadows too; grouped by the def-type its `DT`
//! names, its namespace's key (see [Def-types](#def-types)), whatever
//! import or alias names it, so `def pkg x()` after `import package as pkg
//! from lib` is `package`'s `x`; each by its path as it writes it; all in
//! path order of their files, then in source order. Invoking by name needs
//! a name to mean one thing, so the listing fails, and every invocation
//! with it, if a def-type has two definitions of one path anywhere in the
//! workspace, "`package dev.peios.x` is defined 2 times in the workspace",
//! or two def-types are declared of one path, "def-type `package` is
//! declared 2 times in the workspace", each at the first in path then
//! source order, labelled "also defined here" (or "also declared here") at
//! each other. A definition whose path lies under another's of its
//! def-type anywhere in the workspace, `app.core` under `app`, whether in
//! sibling directories or in one through `#[shadowable]`, is a duplicate
//! too: "`package app.core` lies under another `package` definition", one
//! error for each definition under one or more others, at it, labelled
//! "`app` is defined here" at each definition above it; a definition both
//! defined twice and under another has both errors. None of these is an
//! error of the language, which allows one path in two sibling directories:
//! the workspace checks and runs regardless. A path above definitions that
//! is none itself is no duplicate, but names no definition to invoke.
//!
//! [`Workspace::invoke`] calls the definition as `DT PATH("ARG", ...)`
//! would be called from a file of the definition's directory, with its
//! arguments, given as text, each a string: one for each of the def-type's
//! invocation parameters, in the order it declares them, as many as there
//! are. Only a parameter that a present `String` fits can be given so, a
//! `String` or a union or an alias with it, and so a bare `version: String`
//! of an impure def-type, which is `String | Future<String>` (of a memoised
//! one, `String`); a `Future` cannot. What it asks for may name nothing: no
//! def-type of the name, no definition of the path, or a path above
//! definitions; or it may not fit the def-type, which has an invocation
//! parameter a string does not fit, "`build`'s parameter `n` is `Int`, but
//! only `String` parameters can be given here yet", the first such, its
//! type as it is written, whatever arguments are given, or takes another
//! number of arguments, "`package` takes 1 argument (`version`), but 0 were
//! given" ("takes 2 arguments (`target`, `mode`)", "takes no arguments",
//! "but 1 was given"); each is a [`RequestError`], an error of no source, in
//! that order. Then the
//! definition's file and the def-type's run as a reference to the
//! definition needs them to, each as a whole, with the files they need (its
//! meta's among them, if it has one), unless they have run, as for
//! [`Workspace::run`], and the call is made: an input of its own, built
//! rather than parsed, `<def-type>("PATH", <definition>, "ARG", ...)` as its
//! result, each argument a string literal of the text given, read as
//! nothing else (no `{` in it interpolates), whose two names no source can
//! write are bound, in a scope of the workspace's slots, as fields of the
//! def-type's cell and the definition's (the slots of their closures), with
//! their types. It is checked as an input of no workspace, so that it looks
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
//! out, the driver also builds the one layout of the records `exec`
//! returns (`exec_layout`), its field names interned in the run's names,
//! and gives it to every file it runs, beside the memo tables, so that
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
//! with it. A process killed before then leaves the directory behind. A
//! session owns those of its inputs, and of the standard library's run as
//! it starts, as a workspace does (see [`crate::session`]).
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
use std::rc::Rc;

use crate::Outcome;
use crate::ast::{
    self, Arg, ArgKind, Ast, AstBuilder, DefTypeId, DefTypeRole, Expr, ExprKind, ImportDecl,
    ImportForm, ImportMember, Item, ItemId, ItemKind, Param, PathSegment, TypeExprId, TypeExprKind,
    TypeLink, TypeStore, TypeStoreMark,
};
#[cfg(test)]
use crate::check::ViewRecord;
use crate::check::{
    BUILTIN_TYPE_NAMES, Builtin, CheckOutput, Checker, Fileroot, Place, Scope, Slot, Types,
    check_types,
};
use crate::diagnostic::{self, Diagnostic, InternalError};
use crate::eval::{MemoTables, TempDirs};
use crate::ir::Ir;
use crate::lexer::{self, TokenKind};
use crate::mentions::{self, FieldTree, Mentions, Need, Resolve, Schedule, Sweep, TypeRead};
use crate::names::{self, Name, Names};
use crate::parser;
use crate::session;
use crate::source::{SourceFile, Span};
use crate::strings::Strings;
use crate::types::{TyId, TypeTable};
use crate::value::{Layout, Value};

/// The key of a root in [`Scan`]'s index of nodes by parent and name.
const ROOT: u32 = u32::MAX;

/// The path of the standard library's directories, which no path of a
/// workspace can name, `<` being in no name: each standard file is
/// `<stdlib>/STEM.gcsl`, which is how a diagnostic of one names it, in a
/// directory of its own with this path.
pub const STANDARD_DIR: &str = "<stdlib>";

/// The name of the module of built-in functions.
const BUILTINS: &str = "builtins";

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

/// The layout of the record `exec` returns, its fields
/// ([`Builtin::EXEC_FIELDS`]) interned into `names`, the run's, as the
/// checker interns them for `exec`'s type, so that `r.status` reads it:
/// built once per run as the frame is laid out, and shared by every record
/// `exec` builds in the run, whatever file, input or function calls it (see
/// [`crate::eval`]).
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
    builtins_dir: u32,
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
    type_base: u32,
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
}

impl Scan {
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
    fn value_cell(&self, file: u32) -> Option<u32> {
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
    fn target_child(&self, target: Target, name: Name) -> Option<Target> {
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
    fn view_names(&self, view: u32) -> Vec<Name> {
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
    fn child_names(&self, target: Target) -> Vec<Name> {
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
/// Every syntax error of every file (numbered as if the standard files the
/// files that parsed need were loaded); else every file or directory of the
/// root named like a module of the standard library; else every conflict
/// between files' declarations in a directory; else every conflict of
/// every file's view; else every cycle of files that need each other. Each
/// names its file in [`Diagnostic::input`], its index among the files.
pub fn scan(sources: &[SourceFile]) -> Result<Scan, Vec<Diagnostic>> {
    scan_with(sources, &[])
}

/// [`scan`], told the directories left out of the workspace as workspaces
/// of their own, each by its path relative to the root without a trailing
/// `/`: an import of one is "no module" with a note saying so.
///
/// # Errors
///
/// As [`scan`]; before the view's conflicts, every import that names
/// nothing or clashes with a name of its file or directory chain, and with
/// the cycles, every reference through an import that reaches back into
/// its own file.
pub fn scan_with(sources: &[SourceFile], nested: &[String]) -> Result<Scan, Vec<Diagnostic>> {
    scan_into(
        (sources, Placing::Workspace(nested)),
        Library::NEEDED,
        &mut Names::new(),
        &mut Vec::new(),
    )
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
/// ([`Scan::names`]) and which are given back to `names` if it fails. The
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
    let (standard, parsed) = parse_files(scan, &sorted, library, loaded)?;
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
    let mut errors = Vec::new();
    match placing {
        Placing::Workspace(_) => reserved(scan, library, &mut errors),
        Placing::Lone(_, true) => no_prelude_in_input(scan, &mut errors),
        Placing::Lone(_, false) => {}
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // A definition made with a def-type is a leaf of its def-type's
    // namespace, which is known only once the files' views are: the trees
    // are made again with the definitions under their namespaces' keys, if
    // any definition has one (see [Def-types](#def-types)).
    let mark = scan.type_store.mark();
    for pass in 0..2 {
        scan_trees(scan, placing, &dirs)?;
        if !name_namespaces(scan) || pass > 0 {
            break;
        }
        // A meta declared twice in a directory, or with no definition there,
        // is its own error, before the trees with the metas in them would
        // report it as a conflict (see [Def-types](#def-types)).
        let errors = unpaired_metas(scan);
        if !errors.is_empty() {
            return Err(errors);
        }
        unscan(scan, mark);
    }
    mark_unpaired(scan);
    type_views(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    define_types(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    value_cells(scan);
    resolve(scan);
    let mentioned = find_needs(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    report_cycles(scan, &mentioned, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    scan.order = order(&scan.needs);
    scan.rank = scan.ranks();
    schedule(scan);
    Ok(())
}

/// Merges the trees of `scan`'s files, placed as `placing` says, into their
/// directories' trees, resolves their imports, the directories being
/// `dirs`, by path, and makes their views: the part of [`scan_files`] that
/// a def-type's namespace needs done before it is known, and done again
/// once the definitions are under their namespaces' keys (see
/// [Def-types](#def-types)).
///
/// # Errors
///
/// The conflicts of the directories' trees, the types declared twice, the
/// imports' errors and the conflicts of the views, each stage's stopping
/// the scan.
fn scan_trees(
    scan: &mut Scan,
    placing: Placing,
    dirs: &HashMap<String, u32>,
) -> Result<(), Vec<Diagnostic>> {
    let mut errors = Vec::new();
    shadow(scan);
    scan.roots = vec![Vec::new(); scan.dirs.len()];
    builtin_nodes(scan);
    for file in 0..count(scan.files.len()) {
        merge(scan, file, &mut errors);
    }
    declare_types(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
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
    imports(scan, dirs, (nested, inherited), &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    views(scan, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(())
}

/// Forgets what [`scan_trees`] made of `scan`'s files, and their files'
/// declarations, so that it can make them again from the files' trees,
/// which have changed: every file's declarations found again, the merged
/// nodes, functions, views and types gone, the type store put back at
/// `mark`, where it was before the types were declared.
fn unscan(scan: &mut Scan, mark: TypeStoreMark) {
    let files = mem::take(&mut scan.files);
    scan.files = files
        .into_iter()
        .map(|file| File::new(&file.path, file.dir, file.ast, file.spelled))
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
/// [Def-types](#def-types)): that leaf, its file and its use of its
/// def-type.
type MetaLeaf = ((Name, Name), u32, DefTypeId);

/// The definitions and the metas of `scan`'s files whose def-types are
/// known, each by its leaf in the namespace of the metas of its def-type's
/// definitions in its directory, in path order of their files and then in
/// source order: so a definition and a meta of one path, def-type and
/// directory have one leaf.
fn meta_leaves(scan: &Scan) -> (Vec<MetaLeaf>, Vec<MetaLeaf>) {
    let (mut definitions, mut metas) = (Vec::new(), Vec::new());
    for (index, file) in (0u32..).zip(&scan.files) {
        let ast = &file.ast;
        for &item in ast.items(ast.root()) {
            let (ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. }) =
                ast.item(item).kind
            else {
                continue;
            };
            let Some(leaf) = ast.def_type(def).meta else {
                continue;
            };
            match ast.item(item).kind {
                ItemKind::DefMeta { .. } => metas.push((leaf, index, def)),
                _ => definitions.push((leaf, index, def)),
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
/// source order.
fn unpaired_metas(scan: &Scan) -> Vec<Diagnostic> {
    let (definitions, metas) = meta_leaves(scan);
    let defined: HashSet<(Name, Name)> = definitions.iter().map(|&(leaf, ..)| leaf).collect();
    let mut first: HashMap<(Name, Name), (u32, Span)> = HashMap::new();
    let mut errors = Vec::new();
    for &(leaf, index, def) in &metas {
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
        }
    }
    errors
}

/// Records in each file of `scan` its definitions whose directory declares
/// no meta of their path and def-type ([`File::unpaired`]), which the
/// checker reports if their def-type declares a meta (see the checker's
/// "Metas").
fn mark_unpaired(scan: &mut Scan) {
    let (definitions, metas) = meta_leaves(scan);
    let declared: HashSet<(Name, Name)> = metas.iter().map(|&(leaf, ..)| leaf).collect();
    for (leaf, index, def) in definitions {
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

/// A file parsed: its tree and the names it spells (see [`parse`]).
type Parsed = (Ast, Vec<Name>);

/// Parses the files `sorted`, in path order, and the standard files of
/// `library` that the scan loads, interning into the scan's names, and
/// leaves in `loaded` the indices in the library's table of those it loads,
/// in the table's order: every one if the library says so (a session's),
/// else the prelude and each standard file that a file, or a standard file
/// loaded, imports by its bare stem, in any form ([`standard_imports`]).
/// The prelude, and what it imports, is parsed first, as it always was (a
/// session's every standard file, in the table's order), then the files,
/// then the standard files they need. Returns the trees of the standard
/// files loaded, in the table's order, and the files'.
///
/// # Errors
///
/// Every syntax error, each named by its input number: the standard files
/// loaded first, then the files. A file that fails to parse imports
/// nothing, so a standard file only it would need is not loaded and takes
/// no number.
fn parse_files(
    scan: &mut Scan,
    sorted: &[&SourceFile],
    library: Library,
    loaded: &mut Vec<usize>,
) -> Result<(Vec<Parsed>, Vec<Parsed>), Vec<Diagnostic>> {
    // Each standard file's parse, by its index in the table, once loaded.
    let mut standard: Vec<Option<Result<Parsed, Diagnostic>>> =
        library.table.iter().map(|_| None).collect();
    let mut queue: Vec<usize> = if library.every {
        (0..library.table.len()).rev().collect()
    } else {
        library.find(PRELUDE).into_iter().collect()
    };
    load_standard(scan, library, &mut standard, &mut queue);
    let files: Vec<Result<Parsed, Diagnostic>> = sorted
        .iter()
        .map(|source| parse(source, &mut scan.names))
        .collect();
    for (ast, _) in files.iter().flatten() {
        queue.extend(standard_imports(ast, &scan.names, library));
    }
    load_standard(scan, library, &mut standard, &mut queue);
    loaded.clear();
    loaded.extend((0..standard.len()).filter(|&at| matches!(standard.get(at), Some(Some(_)))));
    let mut errors = Vec::new();
    let mut trees = Vec::with_capacity(loaded.len());
    let numbered = standard.into_iter().flatten().chain(files);
    for (index, parsed) in (0u32..).zip(numbered) {
        match parsed {
            Ok(parsed) => trees.push(parsed),
            Err(error) => errors.push(error.in_input(index)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let files = trees.split_off(loaded.len());
    Ok((trees, files))
}

/// Parses each standard file of `library` that `queue` holds, and each it
/// imports in turn, into `standard`, by its index in the library's table,
/// unless it is there already, emptying `queue`.
fn load_standard(
    scan: &mut Scan,
    library: Library,
    standard: &mut [Option<Result<Parsed, Diagnostic>>],
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
        if let Ok((ast, _)) = &parsed {
            queue.extend(standard_imports(ast, &scan.names, library));
        }
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

/// Lexes and parses `source`, interning into `names`; the tree's own table
/// is left empty. With the tree, the names its checker can look up (see
/// [`File::spelled`]).
fn parse(source: &SourceFile, names: &mut Names) -> Result<(Ast, Vec<Name>), Diagnostic> {
    let (tokens, mut strings) = lexer::tokenize_with(source, names)?;
    let (builder, root) = parser::parse_parts(&tokens, source, names, &mut strings)?;
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
    Ok((ast, spelled))
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

/// Adds `file`'s own tree to its directory's merged tree, reporting the
/// conflicts with the files before it, and numbers its functions.
fn merge(scan: &mut Scan, file: u32, errors: &mut Vec<Diagnostic>) {
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
    let mut gone_from = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let (mut nodes, mut merged, conflicts, gone) = file_view(scan, index);
        for (parent, name, parts) in conflicts {
            report_conflicts(
                scan,
                index,
                &nodes,
                (parent, name, &parts),
                &mut reported,
                errors,
            );
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
        gone_from.push(gone);
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
    for ((file, view), gone) in scan.files.iter_mut().zip(found).zip(gone_from) {
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
struct Modules<'a> {
    /// The directories by path.
    dirs: &'a HashMap<String, u32>,
    /// The files by path.
    files: HashMap<&'a str, u32>,
    /// The directories that are workspaces of their own.
    nested: &'a [String],
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
        let mut import = |item: ItemId, bound: (&mut Vec<Alias>, &mut Vec<TypeImport>)| {
            let ItemKind::Import(id) = file.ast.item(item).kind else {
                return;
            };
            let decl = file.ast.import_decl(id);
            let path = file.ast.path(decl.path);
            match module(scan, &modules, path, &mut both) {
                Ok(target) => members(scan, (file, item, decl), target, bound, &mut own_errors),
                Err(error) => own_errors.extend(error),
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
        clashes(scan, index, &aliases, &mut own_errors);
        errors.extend(own_errors.into_iter().map(|e| e.in_input(index)));
        found.push((aliases, types));
    }
    for (file, (aliases, types)) in scan.files.iter_mut().zip(found) {
        file.aliases = aliases;
        file.type_imports = types;
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
fn module(
    scan: &Scan,
    modules: &Modules,
    path: &[PathSegment],
    both: &mut Vec<String>,
) -> Result<Target, Option<Diagnostic>> {
    let written = match (path.first(), path.last()) {
        (Some(first), Some(last)) => first.span.join(last.span),
        _ => return Err(None),
    };
    let shown: Vec<&str> = path
        .iter()
        .filter_map(|segment| Some(scan.names.get(segment.name()?)))
        .collect();
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
                let note = if modules.nested.contains(&candidate) {
                    format!("`{candidate}/` is a workspace of its own")
                } else {
                    let listed = contents(scan, modules.dirs.get(&at).copied());
                    if at.is_empty() {
                        format!("the workspace has {listed}")
                    } else {
                        format!("`{at}/` has {listed}")
                    }
                };
                return Err(Some(
                    Diagnostic::error(format!("no module `{}`", shown.join(".")), written)
                        .with_note(note),
                ));
            }
        }
    }
    Err(None)
}

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
                            errors.push(Diagnostic::error(
                                format!(
                                    "`{}` has no `{}`",
                                    scan.show_target(target),
                                    scan.names.get(name.name)
                                ),
                                name.span,
                            ));
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
fn target_roots(scan: &Scan, target: Target) -> Vec<Name> {
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
    fn target_types(&self, target: Target) -> Vec<u32> {
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
    /// ([`Scan::qualified_type`]); a name of a type applied to a type
    /// argument, "type `Foo` takes no type argument"; a name of a module the
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
                    Some(&decl) => TypeLink::Decl(decl),
                    None if file.aliases.iter().any(|a| a.whole && a.name == name) => {
                        let text = self.names.get(name);
                        TypeLink::Error(Diagnostic::error(
                            format!("`{text}` is a module, not a type"),
                            span,
                        ))
                    }
                    None => continue,
                },
                TypeRead::Applied(name) => {
                    if !file.type_view.contains_key(&name) {
                        continue;
                    }
                    let text = self.names.get(name);
                    TypeLink::Error(Diagnostic::error(
                        format!("type `{text}` takes no type argument"),
                        span,
                    ))
                }
                TypeRead::Path(path) => self.qualified_type(file, file.ast.path(path), span),
            };
            links.push((id, link));
        }
        links
    }

    /// What the qualified type name `path`, written at `span` in `file`,
    /// names (see [Types](#types)): its first name must be a module the
    /// file imports whole (`import lib.types as t`), and its second a type
    /// of that module, and its last; a path of nothing else is resolved as a
    /// value's reference through an import is, the module's type named by
    /// the name after it. Otherwise the error: "unknown type `a.b`" for a
    /// first name that is no module the file imports, "`t` has no type
    /// `Foo`" with a note listing the module's types, and "`t.Foo` is a
    /// type, not a module" for a path that goes on after a type.
    fn qualified_type(&self, file: &File, path: &[PathSegment], span: Span) -> TypeLink {
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
    for (at, decl) in (0u32..).zip(&types) {
        let Some(dir) = scan.files.get(decl.file as usize).map(|file| file.dir) else {
            continue;
        };
        match first
            .get(&(dir, decl.name))
            .and_then(|&e| types.get(e as usize))
        {
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
            }
        }
    }
    for (file, mine) in scan.files.iter_mut().zip(own) {
        file.types = mine;
    }
    scan.type_base = scan.type_store.reserve(types.len());
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
                }
            }
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
        found.push(numbered);
    }
    for (file, view) in scan.files.iter_mut().zip(found) {
        file.type_view = view;
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
/// first, those on a cycle erroneous ([`TypeStore::settle`]); and, if
/// nothing is wrong, expands each file's tree with them
/// ([`Ast::expand_types`]). A scan with no type to define and nothing that
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
            scan.type_store.settle(base.saturating_add(member), cyclic);
        }
    }
    if !errors.is_empty() {
        return;
    }
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
/// ([`whole_imports_shadow`]).
fn clashes(scan: &Scan, index: u32, aliases: &[Alias], errors: &mut Vec<Diagnostic>) {
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

/// Reports each cycle of files that need each other, once.
fn report_cycles(
    scan: &Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) {
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
    /// For each cell that is a `def`'s, by its number, which parameters of
    /// its function are present because its body measures them, known once
    /// its file is checked: what a call of it through an import needs for
    /// the call-site form of the implicit-await error.
    cell_measured: &'a [Option<Box<[bool]>>],
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
/// for each `def`'s cell which parameters its body measures.
type CellReads<'a> = (&'a [Slot], &'a [Option<Box<[bool]>>]);

impl<'a> Context<'a> {
    /// The context of file `index` of `scan`, where the scan located it
    /// ([`Scan::place`]).
    fn new(
        scan: &'a Scan,
        index: u32,
        cell_types: &'a [TyId],
        (cell_slots, cell_measured): CellReads<'a>,
    ) -> Option<Self> {
        let file = scan.files.get(index as usize)?;
        Some(Self {
            scan,
            file,
            scope: FileScope::new(scan, file),
            index,
            cell_types,
            cell_slots,
            cell_measured,
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

    /// Whether the import item `item` of a lone file, or of a session's
    /// input, names nothing it can: a path of anything but `builtins` and
    /// the standard files, which the checker reports as needing a
    /// workspace (see the module documentation, "The standard library").
    pub(crate) fn needs_workspace(&self, item: ItemId) -> bool {
        self.scan.lone && !self.file.aliases.iter().any(|a| a.item == Some(item))
    }

    /// The slot of the cell numbered `cell` (see [`Scan::slot`]).
    pub(crate) fn slot(&self, cell: u32) -> Slot {
        self.scan.slot(cell)
    }

    /// Which parameters of the `def` whose merged node is `node` its body
    /// measures, if `node` is a `def`'s cell of a file already checked.
    pub(crate) fn measured(&self, node: u32) -> Option<&[bool]> {
        self.cell_measured.get(node as usize)?.as_deref()
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

/// A workspace checked and lowered, ready to run any of its files: driven
/// like a [`crate::session::Session`], one frame and one memo table for
/// every file, but with each file's declarations committed once it is
/// checked, and only the files a run needs evaluated. See the module
/// documentation.
#[derive(Debug)]
pub struct Workspace {
    /// The scanned workspace.
    scan: Scan,
    /// Each file's IR, by input number, until it runs.
    irs: Vec<Option<Ir>>,
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
    /// workspace is dropped (see the module documentation, "The standard
    /// library").
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
    /// For each cell that is a `def`'s, which parameters its body measures
    /// (see [`Context::measured`]), kept for a session's inputs.
    cell_measured: Vec<Option<Box<[bool]>>>,
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
/// lowering fails, with the warnings found, in path then source order;
/// each names its file in [`Diagnostic::input`]. A file that needs a file
/// that failed is not checked: its errors would follow from the first.
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
/// into `types` with `names`; each one's slot; and no `def`'s measured
/// parameters.
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
/// workspace's (see [`Workspace::dump_ir`]), but for those of its
/// temporaries past the `committed` slots it committed, which a later file
/// may take and name otherwise, which are kept, with `committed`.
fn own_slot_names(ir: &mut Ir, committed: usize) -> (usize, Vec<Name>) {
    let own = ir.replace_slot_names(Vec::new());
    (committed, own.get(committed..).unwrap_or_default().to_vec())
}

/// The tables of a workspace's cells, by number: each one's type, the slot
/// it is read from, and for a `def`'s which parameters its body measures.
type CellTables = (Vec<TyId>, Vec<Slot>, Vec<Option<Box<[bool]>>>);

/// Checks and lowers every file of `scan` in the order (see [`check`]), each
/// where the scan located it ([`Scan::locate`]).
fn check_scan(mut scan: Scan) -> Result<Workspace, Vec<Diagnostic>> {
    let mut names = mem::take(&mut scan.names);
    let mut types = TypeTable::new();
    let (mut cell_types, mut cell_slots, mut cell_measured) =
        cell_tables(&scan, &mut types, &mut names);
    let mut slot_names: Vec<Name> = scan.cell_names();
    let mut scopes = vec![Committed::default(); scan.dirs.len()];
    let mut irs: Vec<Option<Ir>> = (0..scan.files.len()).map(|_| None).collect();
    let mut tails: Vec<(usize, Vec<Name>)> = vec![(0, Vec::new()); scan.files.len()];
    let mut checked: Vec<Option<Types>> = (0..scan.files.len()).map(|_| None).collect();
    let mut warnings = vec![Vec::new(); scan.files.len()];
    let mut errors = check_type_items(&mut scan, &mut names, &mut types);
    // The files whose check failed, or that need one that did: those are
    // not checked, since their errors would follow from the first.
    let mut failed = vec![false; scan.files.len()];
    #[cfg(test)]
    let mut view_records: Vec<Vec<ViewRecord>> = vec![Vec::new(); scan.files.len()];
    for index in scan.order.clone() {
        if blocked(&scan, &mut failed, index) {
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
        let reads = (&cell_slots[..], &cell_measured[..]);
        let compiled = Context::new(&scan, index, &cell_types, reads).map(|context| {
            let output = Checker::in_workspace(&file.ast, scope, &mut types, &context).check();
            output.and_then(|output| session::lower_checked(&file.ast, output, &types, index))
        });
        // What the file declares, committed for the files after it.
        let committed = match &compiled {
            Some(Ok((_, output))) => computed_cells(&scan, index, output),
            _ => Vec::new(),
        };
        let keep = |name: Name| commits(&scan, index, name);
        #[cfg(test)]
        if let Some(Ok((_, output))) = &compiled {
            let records = output.checked.resolution.view_records();
            set(&mut view_records, index, records.to_vec());
        }
        let result = match compiled {
            Some(Ok((ir, output))) => {
                if let Some(dir) = scopes.get_mut(file.dir as usize) {
                    dir.commit(&output.scope, start, keep);
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
        for (global, slot, ty, measured) in committed {
            set(&mut cell_types, global, ty);
            set(&mut cell_slots, global, slot);
            set(&mut cell_measured, global, measured);
        }
        match result {
            Ok((mut ir, file_types, found)) => {
                set(&mut tails, index, own_slot_names(&mut ir, slot_names.len()));
                set(&mut irs, index, Some(ir));
                set(&mut checked, index, Some(file_types));
                set(&mut warnings, index, in_file(found, index));
            }
            Err(found) => {
                errors.extend(in_file(found, index));
                set(&mut failed, index, true);
            }
        }
    }
    if !errors.is_empty() || failed.contains(&true) {
        let warnings = warnings.into_iter().flatten().collect();
        return Err(merge_files(reported(errors), warnings));
    }
    scan.names = names;
    Ok(Workspace {
        values: vec![Value::Empty; scan.files.len()],
        // Built before `scan` moves; its names are interned already.
        exec_result: exec_layout(&mut scan.names),
        temp_dirs: TempDirs::new(),
        scan,
        irs,
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
        cell_measured,
        #[cfg(test)]
        view_records,
    })
}

/// Checks the `type` items of every file of `scan` that has any, before any
/// file is checked, since a file may use a type of a file checked after it
/// (see [Types](#types)): what is wrong in each declaration is reported
/// here, once, and every file that uses it fails quietly. Lends each file
/// `names`, and interns into `types`. Returns the errors, each naming its
/// file.
fn check_type_items(scan: &mut Scan, names: &mut Names, types: &mut TypeTable) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for (index, file) in (0u32..).zip(&mut scan.files) {
        if file.mentions.types.is_empty() {
            continue;
        }
        file.ast.replace_names(mem::take(names));
        let found = check_types(&file.ast, types);
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
    /// `keep` accepts, as [`Scope::commit`] does, and indexes them.
    fn commit(&mut self, from: &Scope, start: usize, keep: impl Fn(Name) -> bool) {
        let before = self.scope.len();
        self.scope.commit(from, start, keep);
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
    let measured_by_slot = output.scope.measured_by_slot();
    let nodes = file.map_or(&[][..], |file| &file.mentions.tree.nodes);
    let mut cells = Vec::new();
    for (own, (&global, &ty)) in global.iter().zip(&output.cell_types).enumerate() {
        if scan.computed_by(global) == Some(index) {
            // A `def`'s cell is read from its function's slot.
            let slot = slots.get(own).copied().unwrap_or_else(|| scan.slot(global));
            let mut measured = measured_by_slot
                .get(&slot)
                .copied()
                .flatten()
                .map(Box::from);
            // A `def` named by a dotted path has no binding: what its body
            // measures is its function's, which the cell's type is.
            if measured.is_none()
                && ty != TyId::ERROR
                && let Some(function) = nodes
                    .get(own)
                    .and_then(|node| node.leaf)
                    .and_then(|leaf| leaf.function)
                    .and_then(|function| file?.mentions.functions.get(function as usize))
                    .filter(|function| function.name.is_none())
            {
                let resolution = output.checked.resolution.function(function.function);
                measured = Some(
                    resolution
                        .param_demands
                        .iter()
                        .map(Option::is_some)
                        .collect(),
                );
            }
            cells.push((global, slot, ty, measured));
        }
    }
    // An imported value file's value is a cell of the workspace.
    if let Some(cell) = scan.value_cell(index) {
        cells.push((cell, scan.slot(cell), output.ty, None));
    }
    cells
}

/// A cell a file computes, as [`computed_cells`] gives it: its number, the
/// slot it is read from, its type, and for a `def` which parameters its body
/// measures.
type ComputedCell = (u32, Slot, TyId, Option<Box<[bool]>>);

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
        let ir = self.irs.get_mut(index as usize)?.as_mut()?;
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
    /// with the workspace's warnings.
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
            let Some(ir) = self.irs.get_mut(at as usize).and_then(Option::take) else {
                continue;
            };
            let context = (
                &mut self.memo,
                Some(&self.exec_result),
                &self.scan.dir_texts[..],
                &self.temp_dirs,
            );
            match session::run_input(ir, &mut self.frame, context) {
                Ok(value) => {
                    if let Some(entry) = self.values.get_mut(at as usize) {
                        *entry = value;
                    }
                }
                Err(mut error) => {
                    error.input.get_or_insert(at);
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
    /// what invoking one of its definitions takes an argument for each of.
    pub parameters: Vec<String>,
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
    /// invoked by it.
    pub path: String,
    /// The input number of its file.
    pub file: u32,
    /// Where its path is written in its file.
    pub span: Span,
}

/// Why [`Workspace::invoke`] could not invoke a definition.
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
    /// An invocation parameter of the def-type is of a type that a string
    /// does not fit, and only strings can be given by name yet: "`build`'s
    /// parameter `n` is `Int`, but only `String` parameters can be given
    /// here yet". The first such parameter, as it declares them.
    CannotBeGiven {
        /// The def-type's path.
        def_type: String,
        /// The parameter's name.
        parameter: String,
        /// The parameter's type, as it is written.
        ty: String,
    },
    /// The number of arguments given is not the number of the def-type's
    /// invocation parameters: "`package` takes 1 argument (`version`), but
    /// 0 were given".
    ArgumentCount {
        /// The def-type's path.
        def_type: String,
        /// The names of its invocation parameters, as it declares them, in
        /// order.
        parameters: Vec<String>,
        /// The number of arguments given.
        given: usize,
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
            RequestError::CannotBeGiven {
                def_type,
                parameter,
                ty,
            } => format!(
                "`{def_type}`'s parameter `{parameter}` is `{ty}`, but only `String` parameters \
                 can be given here yet"
            ),
            RequestError::ArgumentCount {
                def_type,
                parameters,
                given,
            } => {
                let takes = match &parameters[..] {
                    [] => "no arguments".to_owned(),
                    [_] => format!("1 argument ({})", quoted_list(parameters)),
                    _ => format!(
                        "{} arguments ({})",
                        parameters.len(),
                        quoted_list(parameters)
                    ),
                };
                let was = if *given == 1 { "was" } else { "were" };
                format!("`{def_type}` takes {takes}, but {given} {was} given")
            }
        }
    }

    /// The notes that follow the message: what there is to ask for instead,
    /// "the def-types are `build`, `package`", or with one, "the only
    /// def-type is `package`", and so "its definitions are `a`, `b`" and
    /// "its only definition is `a`".
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
            RequestError::CannotBeGiven { .. } | RequestError::ArgumentCount { .. } => Vec::new(),
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
    /// One error for each path of which a def-type has two definitions or
    /// more anywhere in the workspace, "`package dev.peios.x` is defined 2
    /// times in the workspace", and each path of which two def-types or
    /// more are declared, "def-type `package` is declared 2 times in the
    /// workspace", at the first in path then source order, labelled "also
    /// defined here", or "also declared here", at each other; and one error
    /// for each definition whose path lies under another's of its def-type
    /// anywhere in the workspace, "`package app.core` lies under another
    /// `package` definition", at it, labelled "`app` is defined here" at
    /// each definition above it; all in path order of their files, then in
    /// source order, with the workspace's warnings, as [`Workspace::run`]
    /// returns its errors. None is an error of the language: the workspace
    /// checks and runs regardless.
    pub fn def_types(&self) -> Result<Vec<DefType>, Vec<Diagnostic>> {
        let found = self.found_def_types();
        let duplicates = duplicates(&found);
        if !duplicates.is_empty() {
            return Err(merge_files(duplicates, self.warnings.clone()));
        }
        Ok(found.into_iter().map(|found| found.listed).collect())
    }

    /// Invokes the definition of path `path` made with the def-type of path
    /// `def_type`, both as written (`"package"`, `"dev.peios.testpkg"`),
    /// with the arguments `args`, as if `DT PATH("ARG", ...)` were called
    /// from a file of the definition's directory, each argument a string
    /// literal of its text (see the module documentation, "Def-types by
    /// name"): the definition's file and the def-type's run first, each as a
    /// whole, with the files they need, unless they have run, and then the
    /// def-type is called with the definition's path, the definition and
    /// the arguments, one for each of its invocation parameters, in order,
    /// its caller the directory of the definition's file. Returns the value
    /// the def-type returns, with the warnings of the whole workspace.
    /// Invoked again, the call runs again.
    ///
    /// # Errors
    ///
    /// In this order: the duplicates [`Workspace::def_types`] fails with,
    /// definitions under others among them, with the warnings; no def-type
    /// of path `def_type`, no definition of it of path `path`, or `path`
    /// above some, an invocation parameter of the def-type that a present
    /// `String` does not fit, or another number of `args` than of its
    /// invocation parameters, each a [`RequestError`]; and the runtime
    /// error of a file that runs or of the call, as
    /// [`Workspace::run`] returns it, a part of it in the call itself,
    /// which has no source, located at the definition's path, labelled
    /// "invoked here".
    pub fn invoke(
        &mut self,
        def_type: &str,
        path: &str,
        args: &[String],
    ) -> Result<Outcome, InvokeError> {
        let (found, at, (def_type_cell, definition_cell)) = self.invoked(def_type, path)?;
        if let Some(error) = self.cannot_give(&found, def_type_cell.1, args) {
            return Err(InvokeError::Request(error));
        }
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
        self.call_definition(
            (definition, Invocation::Call(args)),
            (def_type_cell, definition_cell),
        )
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
        let (found, at, (def_type_cell, definition_cell)) = self.invoked(def_type, path)?;
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
        )
        .map(Some)
    }

    /// The def-type of path `def_type` and its definition of path `path`,
    /// as [`Workspace::invoke`] and [`Workspace::meta_field`] find them: the
    /// def-type, the definition's place among its definitions, and the two
    /// cells.
    ///
    /// # Errors
    ///
    /// The duplicates, with the warnings; no such def-type, or definition.
    #[allow(clippy::type_complexity)]
    fn invoked(
        &self,
        def_type: &str,
        path: &str,
    ) -> Result<(FoundDefType, usize, ((Slot, TyId), (Slot, TyId))), InvokeError> {
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
        let at = definitions.iter().position(|d| d.path == path);
        let (Some(at), Some(definition), Some(&item)) = (
            at,
            at.and_then(|at| definitions.get(at)),
            at.and_then(|at| found.items.get(at)),
        ) else {
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
        Ok((found, at, (def_type_cell, definition_cell)))
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
                        let definition = Definition {
                            path: text(segments),
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

    /// Why `args` cannot be given to the def-type `found`, whose type is
    /// `ty`, if they cannot (see [Def-types by name](#def-types-by-name)):
    /// the first of its invocation parameters, as it declares them, that a
    /// present `String` does not fit, its type as it is written; else
    /// `args`' number, if it is not its invocation parameters'.
    fn cannot_give(&self, found: &FoundDefType, ty: TyId, args: &[String]) -> Option<RequestError> {
        let listed = &found.listed;
        let params = self
            .types
            .fn_sig(ty)
            .and_then(|sig| sig.params.get(2..))
            .unwrap_or_default();
        let parameters = listed.parameters.iter().zip(&found.written);
        for (&(_, param), (parameter, written)) in params.iter().zip(parameters) {
            if !self.types.is_subtype(TyId::STRING, param) {
                return Some(RequestError::CannotBeGiven {
                    def_type: listed.path.clone(),
                    parameter: parameter.clone(),
                    ty: written.clone(),
                });
            }
        }
        (args.len() != listed.parameters.len()).then(|| RequestError::ArgumentCount {
            def_type: listed.path.clone(),
            parameters: listed.parameters.clone(),
            given: args.len(),
        })
    }

    /// The cell of the leaf the item `item` of file `index` declares, a
    /// `def` or a definition: the slot it is read from, its function's, and
    /// its type, once its file is checked; `None` if the item is a leaf of
    /// no tree, a later declaration shadowing it in its own directory.
    fn leaf_cell(&self, index: u32, item: ItemId) -> Option<(Slot, TyId)> {
        let file = self.scan.files.get(index as usize)?;
        let position = file
            .ast
            .items(file.ast.root())
            .iter()
            .position(|&at| at == item)?;
        let own = (*file.mentions.tree.item_leaf.get(position)?)?;
        let node = *file.global.get(own as usize)?;
        Some((
            *self.cell_slots.get(node as usize)?,
            *self.cell_types.get(node as usize)?,
        ))
    }

    /// Calls the def-type whose cell is `def_type` with the path of
    /// `definition`, the definition, whose cell is `definition_cell`, and
    /// `args`, their files run: the input `<def-type>("PATH", <definition>,
    /// "ARG", ...)`, its names no source can write bound to the two cells,
    /// checked from the workspace's slots, as an input of no workspace, and
    /// lowered as one placed in the definition's directory, its caller,
    /// numbered after every file (see [Def-types by
    /// name](#def-types-by-name)).
    fn call_definition(
        &mut self,
        (definition, invoked): (&Definition, Invocation),
        (def_type, definition_cell): ((Slot, TyId), (Slot, TyId)),
    ) -> Result<Outcome, InvokeError> {
        let call = count(self.scan.files.len());
        let at = (definition.file, definition.span);
        let mut names = mem::take(&mut self.scan.names);
        let bound = (
            names.intern(INVOKED_DEF_TYPE),
            names.intern(INVOKED_DEFINITION),
        );
        let ast = invocation(names, (&definition.path, invoked), bound, at.1);
        let scope = Scope::of_slots(self.slot_names.clone()).with_fields([
            (bound.0, def_type.0, def_type.1),
            (bound.1, definition_cell.0, definition_cell.1),
        ]);
        let place = self.scan.place(definition.file);
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
        let context = (
            &mut self.memo,
            Some(&self.exec_result),
            &self.scan.dir_texts[..],
            &self.temp_dirs,
        );
        let mut warnings = self.warnings.clone();
        warnings.extend(invoked_here(output.warnings, call, at));
        match session::run_input(ir, &mut self.frame, context) {
            Ok(value) => Ok(Outcome { value, warnings }),
            Err(error) => {
                let errors = invoked_here(vec![error], call, at);
                Err(InvokeError::Diagnostics(merge_files(errors, warnings)))
            }
        }
    }
}

/// The error of `path`, which no definition made with the def-type `listed`
/// has: "`dev.peios` is not a `package` definition" if it is above some of
/// them, else "`package` has no definition `dev.peios.nope`".
fn missing_definition(listed: &DefType, path: &str) -> RequestError {
    let mut definitions: Vec<String> = listed
        .definitions
        .iter()
        .map(|definition| definition.path.clone())
        .collect();
    definitions.sort_unstable();
    let above = definitions
        .iter()
        .any(|definition| lies_under(definition, path));
    let (def_type, path) = (listed.path.clone(), path.to_owned());
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
/// which a def-type has two definitions or more, and for each definition
/// that lies under another of its def-type ([`overlaps`]), in path then
/// source order of the first, or of the definition below.
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
        let defined = found
            .listed
            .definitions
            .iter()
            .map(|definition| (definition.path.as_str(), definition.file, definition.span));
        let def_type = &found.listed.path;
        for group in groups(defined) {
            errors.extend(duplicate(
                &group,
                |path, times| {
                    format!("`{def_type} {path}` is defined {times} times in the workspace")
                },
                "also defined here",
                "a definition invoked by name must be the only one of its path in the workspace",
            ));
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
            .filter(|other| lies_under(&definition.path, &other.path))
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
        .with_note("a definition invoked by name must not lie under another in the workspace");
        for other in above {
            let text = format!("`{}` is defined here", other.path);
            error = error.with_context_in(other.file, other.span, text);
        }
        errors.push(error);
    }
    errors
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
/// `<def-type>("PATH", <definition>, "ARG", ...)`, its result, PATH `path`,
/// each ARG one of `args`, in order, a string literal, the two names those
/// of `bound`, each node at `span`.
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
    let mut positional = vec![
        Arg {
            kind: ArgKind::Positional,
            value: name,
        },
        Arg {
            kind: ArgKind::Positional,
            value: definition,
        },
    ];
    for arg in args {
        let value = builder.alloc(Expr {
            kind: ExprKind::Str(strings.push(arg)),
            span,
        });
        positional.push(Arg {
            kind: ArgKind::Positional,
            value,
        });
    }
    let args = builder.push_args(&positional);
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
    /// `<def-type>("PATH", <definition>, "ARG", ...)`.
    Call(&'a [String]),
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
            diagnostic.label = Some(INVOKED.to_owned());
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
    /// For each of its cells that is a `def`'s, which parameters its body
    /// measures.
    cell_measured: Vec<Option<Box<[bool]>>>,
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
        cell_measured,
        ..
    } = workspace;
    Ok(Started {
        standard: Standard {
            cells: count(scan.cells()),
            cell_types,
            cell_slots,
            cell_measured,
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
    let mut cell_measured = standard.cell_measured.clone();
    cell_measured.resize(cells, None);
    let scope = scope.with_slots(taken);
    let file_names = mem::take(&mut scan.names);
    let Some(file) = scan.files.get_mut(index as usize) else {
        *names = file_names;
        *store = mem::take(&mut scan.type_store);
        return Err(Vec::new());
    };
    file.ast.replace_names(file_names);
    let finished = scan.files.get(index as usize).and_then(|file| {
        let reads = (&cell_slots[..], &cell_measured[..]);
        let context = Context::new(&scan, index, &cell_types, reads)?;
        let context = Context {
            session: true,
            ..context
        };
        let declared = if file.mentions.types.is_empty() {
            Vec::new()
        } else {
            check_types(&file.ast, types)
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
mod tests {
    use super::*;
    use crate::ir::IrKind;

    /// Scans the workspace of `files`, each a path and a text.
    fn ws(files: &[(&str, &str)]) -> Result<Scan, Vec<Diagnostic>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        scan(&sources)
    }

    /// The single error of scanning `files`: its message, input, and the
    /// inputs of its labels.
    fn error(files: &[(&str, &str)]) -> (String, Option<u32>, Vec<Option<u32>>) {
        let errors = ws(files).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let error = &errors[0];
        let labels = error.labels.iter().map(|l| l.input).collect();
        (error.message.clone(), error.input, labels)
    }

    /// The files of the summary, in path order: app, common, lib,
    /// workspace.
    const SUMMARY: [(&str, &str); 4] = [
        ("workspace.gcsl", ""),
        (
            "common.gcsl",
            "version = \"1.2\"\ndef tagged(n: String): String { \"{n}-{version}\" }",
        ),
        (
            "app.gcsl",
            "com.x.app.name = tagged(\"app\")\ncom.x.app.deps = [com.x.lib]",
        ),
        (
            "lib.gcsl",
            "com.x.lib.name = tagged(\"lib\")\ncom.x.lib.src = \"lib-{version}.tar\"",
        ),
    ];

    #[test]
    fn the_summary_runs_in_need_order() {
        let scan = ws(&SUMMARY).unwrap();
        let paths: Vec<&str> = scan.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "<stdlib>/prelude.gcsl",
                "app.gcsl",
                "common.gcsl",
                "lib.gcsl",
                "workspace.gcsl"
            ]
        );
        assert_eq!(scan.needs, [vec![], vec![2, 3], vec![], vec![2], vec![]]);
        assert_eq!(scan.order, [0, 2, 3, 1, 4]);
        assert_eq!(scan.needed(1), [0, 2, 3, 1]);
        assert_eq!(scan.needed(3), [0, 2, 3]);
        assert_eq!(scan.needed(4), [0, 4]);
    }

    #[test]
    fn a_reference_means_its_prefix_in_the_merged_tree() {
        let scan = ws(&SUMMARY).unwrap();
        let app = &scan.files[1];
        // `com.x.lib` in app's second item is lib's interior node, not
        // app's own `com.x`.
        let foreign = &app.mentions.foreign[1];
        assert_eq!(foreign.len(), 1);
        let Need::Field(node) = foreign[0].0 else {
            panic!("{foreign:?}")
        };
        let merged = FileScope::new(&scan, app).merged(node).unwrap();
        assert_eq!(scan.show(merged), "com.x.lib");
        assert_eq!(scan.nodes[merged as usize].contributors, [3]);
    }

    #[test]
    fn dotted_paths_merge_and_the_last_contributor_assembles() {
        let scan = ws(&[("a.gcsl", "com.a = 1"), ("b.gcsl", "all = com\ncom.b = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1, 2]);
        let com = scan.by_name[&(0, ROOT, scan.files[1].mentions.tree.nodes[0].name)];
        assert_eq!(scan.nodes[com as usize].contributors, [1, 2]);
        assert_eq!(scan.computed_by(com), Some(2));
        // a leaves `com` out; b computes it before its first mention.
        assert!(!scan.files[1].schedule.order.contains(&0));
        let b = &scan.files[2];
        let local = b.global.iter().position(|&g| g == com).unwrap();
        assert!(b.schedule.order.contains(&count(local)));
        assert_eq!(b.schedule.point[local], 0);
    }

    #[test]
    fn a_shared_node_read_whole_from_a_third_file() {
        let scan = ws(&[
            ("a.gcsl", "com.a = 1"),
            ("b.gcsl", "com.b = 2\nz = 3"),
            ("c.gcsl", "all = com"),
        ])
        .unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![], vec![1, 2]]);
        // b, the last contributor, assembles `com` after its last child.
        let b = &scan.files[2];
        assert_eq!(b.schedule.point[0], 1);
        assert!(!scan.files[1].schedule.order.contains(&0));
    }

    #[test]
    fn a_def_of_one_file_called_from_another_is_a_need() {
        let scan = ws(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "x = f()")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
    }

    #[test]
    fn siblings_and_subdirectories_are_hidden() {
        let scan = ws(&[("x/a.gcsl", "v = 1"), ("y/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "w = v"), ("sub/b.gcsl", "v = 1")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![]]);
        let scan = ws(&[("a.gcsl", "v = 1"), ("sub/b.gcsl", "w = v")]).unwrap();
        assert_eq!(scan.needs, [vec![], vec![], vec![1]]);
    }

    #[test]
    fn conflicts_between_files() {
        assert_eq!(
            error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x.y = 2")]),
            (
                "field `x.y` is defined in two files".to_owned(),
                Some(2),
                vec![Some(1)]
            )
        );
        assert_eq!(
            error(&[("a.gcsl", "x = 1"), ("b.gcsl", "x.y = 2")]).0,
            "field `x` is already defined"
        );
        assert_eq!(
            error(&[("a.gcsl", "x.y = 1"), ("b.gcsl", "x = 2")]).0,
            "field `x` is already defined"
        );
        assert_eq!(
            error(&[("a.gcsl", "def f(): Int { 1 }"), ("b.gcsl", "f = 2")]).0,
            "field `f` is defined in two files"
        );
        // A root of a directory above merges; the same leaf in both is a
        // conflict, reported at the directory above.
        assert_eq!(
            error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
            (
                "field `version` is defined in two files".to_owned(),
                Some(1),
                vec![Some(2)]
            )
        );
    }

    #[test]
    fn files_that_need_each_other() {
        let (message, input, labels) = error(&[("a.gcsl", "p = q"), ("b.gcsl", "q = p")]);
        assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
        assert_eq!((input, labels), (Some(1), vec![Some(2)]));
        let (message, _, labels) = error(&[
            ("a.gcsl", "p = q"),
            ("b.gcsl", "q = r"),
            ("c.gcsl", "r = p"),
        ]);
        assert_eq!(message, "`a.gcsl`, `b.gcsl` and `c.gcsl` need each other");
        assert_eq!(labels, [Some(2), Some(3)]);
    }

    #[test]
    fn path_order_breaks_ties_and_errors_name_their_file() {
        let scan = ws(&[("b.gcsl", "x = 1"), ("a.gcsl", "y = 2")]).unwrap();
        assert_eq!(scan.order, [0, 1, 2]);
        assert_eq!(scan.files[1].path, "a.gcsl");
        let errors = ws(&[("a.gcsl", "x = 1"), ("b.gcsl", "y = (")]).unwrap_err();
        assert_eq!(errors[0].input, Some(2));
    }

    /// Checks the workspace of `files` and runs the file at `path`,
    /// returning its value shown, or the messages and inputs of its
    /// diagnostics.
    fn run_file(files: &[(&str, &str)], path: &str) -> Result<String, Vec<(String, Option<u32>)>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let shown = |errors: Vec<Diagnostic>| {
            errors
                .into_iter()
                .map(|error| (error.message, error.input))
                .collect::<Vec<_>>()
        };
        let mut workspace = check(&sources).map_err(shown)?;
        assert_imports_stored(&workspace);
        assert_assemblies_stored(&workspace);
        let index = workspace.file(path).unwrap();
        let outcome = workspace.run(index).map_err(shown)?;
        Ok(outcome.value.to_string())
    }

    /// Asserts that the driver wrote the cell of every built-in function
    /// as it laid the frame out, before any file runs: a `Value::Builtin`,
    /// never `Value::Empty`, which would read as a record-kinded `{}` that
    /// nothing else can see. Returns their slots, which no file stores.
    fn assert_builtins_written(workspace: &Workspace) -> Vec<Slot> {
        Builtin::ALL
            .into_iter()
            .zip(0u32..)
            .map(|(builtin, node)| {
                let slot = workspace.cell_slots[node as usize];
                assert_eq!(workspace.scan.builtin(node), Some(builtin));
                assert_eq!(
                    workspace.frame.get(slot.index()),
                    Some(&Value::Builtin(builtin)),
                    "the cell of `{}` is not written by the driver",
                    builtin.name()
                );
                slot
            })
            .collect()
    }

    /// Asserts that every cell an import of `workspace` reaches, directly
    /// or as a field of a record read whole, is stored by a node of its
    /// owner's top-level block: slice 24's schedule test sees one file
    /// only, and a cell no owner writes would read as `{}`. A built-in
    /// function's cell has no owner: the driver wrote it before any file
    /// ran ([`assert_builtins_written`]).
    fn assert_imports_stored(workspace: &Workspace) {
        let scan = &workspace.scan;
        let types = vec![TyId::ERROR; scan.cells()];
        let builtins = assert_builtins_written(workspace);
        for (index, file) in (0u32..).zip(&scan.files) {
            let reads = (&workspace.cell_slots[..], &[][..]);
            let context = Context::new(scan, index, &types, reads).unwrap();
            let mut pending: Vec<Target> = file
                .mentions
                .foreign
                .iter()
                .flatten()
                .filter_map(|&(need, _)| match need {
                    Need::Field(n) => context.scope.target(n),
                    Need::Function(_) => None,
                })
                .collect();
            while let Some(target) = pending.pop() {
                let cell = match target {
                    Target::Node(node) => Some(context.cell_slot(node)),
                    other => context.value_of(other).map(Slot::at),
                };
                let Some(slot) = cell else {
                    pending.extend(context.target_fields(target).into_iter().map(|(_, t)| t));
                    continue;
                };
                if builtins.contains(&slot) {
                    continue;
                }
                let owners = scan.target_files(target);
                let stored = owners.iter().any(|&owner| {
                    let Some(Some(ir)) = workspace.irs.get(owner as usize) else {
                        return false;
                    };
                    let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                        return false;
                    };
                    ir.items(items)
                        .iter()
                        .any(|&item| matches!(ir.get(item).kind, IrKind::Store(s, _) if s == slot))
                });
                assert!(
                    stored,
                    "{}: `{}` is not stored by {owners:?}",
                    file.path,
                    scan.show_target(target)
                );
            }
        }
    }

    /// The slots the top-level block of file `file` of `workspace` stores,
    /// in order.
    fn top_stores(workspace: &Workspace, file: u32) -> Vec<Slot> {
        let Some(Some(ir)) = workspace.irs.get(file as usize) else {
            return Vec::new();
        };
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            return Vec::new();
        };
        ir.items(items)
            .iter()
            .filter_map(|&item| match ir.get(item).kind {
                IrKind::Store(slot, _) => Some(slot),
                _ => None,
            })
            .collect()
    }

    /// Asserts that every record a file of `workspace` assembles of a node
    /// of its view with a leaf of the file under it reads only cells stored
    /// before it: the file's own by an earlier node of its top-level block,
    /// every other one by the top-level block of a file it needs, which runs
    /// whole before it. And that every record any file assembles at its top
    /// level (an interior node's, a shared node's, an imported one) reads
    /// only cells stored earlier in its block or by a file earlier in the
    /// order: slice 25's shared nodes read the cells of the files that
    /// contribute to them, which come earlier but need not be needed. A cell
    /// nobody stored first would read as `{}`. A built-in function's cell
    /// counts as stored before any file: the driver wrote it as it laid the
    /// frame out ([`assert_builtins_written`]).
    fn assert_assemblies_stored(workspace: &Workspace) {
        let scan = &workspace.scan;
        let builtins = assert_builtins_written(workspace);
        for (index, file) in (0u32..).zip(&scan.files) {
            let own = top_stores(workspace, index);
            let needed: Vec<Slot> = scan
                .needed(index)
                .into_iter()
                .filter(|&other| other != index)
                .flat_map(|other| top_stores(workspace, other))
                .chain(builtins.iter().copied())
                .collect();
            let rank = scan.order.iter().position(|&f| f == index).unwrap_or(0);
            let earlier: Vec<Slot> = scan.order[..rank]
                .iter()
                .flat_map(|&other| top_stores(workspace, other))
                .chain(builtins.iter().copied())
                .collect();
            let records = workspace.view_records.get(index as usize);
            for (slot, _, fields) in records.into_iter().flatten() {
                let Some(at) = own.iter().position(|s| s == slot) else {
                    let path = &file.path;
                    assert_eq!(*slot, Slot::NONE, "{path}: a view record is not stored");
                    continue;
                };
                for &(name, cell, _) in fields {
                    assert!(
                        own[..at].contains(&cell) || needed.contains(&cell),
                        "{}: the view record in {slot:?} reads `{}` from {cell:?}, stored by \
                         nothing before it",
                        file.path,
                        scan.names.get(name)
                    );
                }
            }
            let Some(Some(ir)) = workspace.irs.get(index as usize) else {
                continue;
            };
            let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
                continue;
            };
            let mut stored: Vec<Slot> = Vec::new();
            for &item in ir.items(items) {
                let IrKind::Store(slot, value) = ir.get(item).kind else {
                    continue;
                };
                if let IrKind::MakeRecord { values, .. } = ir.get(value).kind {
                    for &field in ir.items(values) {
                        if let IrKind::Load(cell, _) = ir.get(field).kind {
                            assert!(
                                stored.contains(&cell) || earlier.contains(&cell),
                                "{}: a record stored in {slot:?} reads {cell:?}, stored by \
                                 nothing before it",
                                file.path
                            );
                        }
                    }
                }
                stored.push(slot);
            }
        }
    }

    /// The value of the file at `path` of the workspace of `files`.
    fn value(files: &[(&str, &str)], path: &str) -> String {
        run_file(files, path).unwrap_or_else(|errors| panic!("{path}: {errors:?}"))
    }

    /// The messages of the diagnostics of running `path` in `files`.
    fn messages(files: &[(&str, &str)], path: &str) -> Vec<String> {
        match run_file(files, path) {
            Ok(value) => panic!("{path}: unexpected value {value}"),
            Err(errors) => errors.into_iter().map(|(message, _)| message).collect(),
        }
    }

    #[test]
    fn the_summary_runs_each_file_to_its_own_value() {
        let mut files = SUMMARY.to_vec();
        // A file nothing needs is checked but never run: running it fails.
        files.push(("unused.gcsl", "def boom(): Int { [1][5] }\nnever = boom()"));
        assert_eq!(
            value(&files, "app.gcsl"),
            "{com = {x = {app = {name = \"app-1.2\"; deps = [{name = \"lib-1.2\"; \
             src = \"lib-1.2.tar\"}]}}}}"
        );
        assert_eq!(
            value(&files, "lib.gcsl"),
            "{com = {x = {lib = {name = \"lib-1.2\"; src = \"lib-1.2.tar\"}}}}"
        );
        // A `def` is a field of its file's record, as in one file.
        assert_eq!(
            value(&files, "common.gcsl"),
            "{version = \"1.2\"; tagged = fn(n: String): String}"
        );
        assert_eq!(value(&files, "workspace.gcsl"), "{}");
    }

    #[test]
    fn dotted_paths_merge_in_either_path_order() {
        for (a, b) in [("a.gcsl", "b.gcsl"), ("b.gcsl", "a.gcsl")] {
            let files = [
                ("workspace.gcsl", ""),
                (a, "com.x.one = 1"),
                (b, "com.x.two = 2"),
                ("c.gcsl", "all = com\nx = com.x"),
            ];
            // The merged node's fields come in path order of their files.
            let x = if a == "a.gcsl" {
                "{one = 1; two = 2}"
            } else {
                "{two = 2; one = 1}"
            };
            assert_eq!(
                value(&files, "c.gcsl"),
                format!("{{all = {{x = {x}}}; x = {x}}}")
            );
            assert_eq!(value(&files, a), "{com = {x = {one = 1}}}");
        }
    }

    #[test]
    fn defs_and_function_values_cross_files() {
        let files = [
            (
                "a.gcsl",
                "def twice(n: Int): Int { n * 2 }\nf = fn(n: Int): Int { n + 1 }",
            ),
            (
                "b.gcsl",
                "g = f\nh = twice(3)\nfn k(): Int { twice(4) }\nm = k()",
            ),
            ("c.gcsl", "r = g(10)"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "b.gcsl"),
            "{g = fn(n: Int): Int; h = 6; m = 8}"
        );
        assert_eq!(value(&files, "c.gcsl"), "{r = 11}");
    }

    #[test]
    fn a_contributor_reads_a_shared_node_whole() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "com.one = 1"),
            ("b.gcsl", "com.two = 2\nall = com"),
        ];
        assert_eq!(
            value(&files, "b.gcsl"),
            "{com = {two = 2}; all = {one = 1; two = 2}}"
        );
    }

    #[test]
    fn private_names_and_other_directories_are_not_shared() {
        let base = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "let p = 1\nfn q(): Int { 2 }\nx = 3\ndef d(): Int { 4 }\n...{ s = 5 }",
            ),
        ];
        for (text, message) in [
            ("y = p", "undefined variable `p`"),
            ("y = q()", "undefined variable `q`"),
            ("y = s", "undefined variable `s`"),
            ("let x = 1", "`x` is a field of this directory"),
            ("let mut x = 1", "`x` is a field of this directory"),
            ("x := 1", "`x` is a field of this directory"),
            ("let x.y = 1", "`x` is a field of this directory"),
            ("fn x(): Int { 1 }", "`x` is a field of this directory"),
            ("let d = 1", "`d` is a field of this directory"),
        ] {
            let mut files = base.to_vec();
            files.push(("b.gcsl", text));
            assert!(
                messages(&files, "b.gcsl").iter().any(|m| m == message),
                "{text}: {:?}",
                messages(&files, "b.gcsl")
            );
            // From a directory below, likewise.
            let mut files = base.to_vec();
            files.push(("sub/b.gcsl", text));
            assert!(
                messages(&files, "sub/b.gcsl").iter().any(|m| m == message),
                "sub: {text}"
            );
        }
        let mut files = base.to_vec();
        files.push(("sub/c.gcsl", "y = x + d()"));
        files.push(("other/d.gcsl", "z = 1"));
        assert_eq!(value(&files, "sub/c.gcsl"), "{y = 7}");
        files.push(("e.gcsl", "w = z"));
        assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `z`"]);
        // The directory above does not see a subdirectory's declarations.
        files.pop();
        files.push(("e.gcsl", "w = y"));
        assert_eq!(messages(&files, "e.gcsl"), ["undefined variable `y`"]);
    }

    #[test]
    fn a_clash_within_a_file_is_reported_as_in_one_file() {
        // A root of the file itself: one file's errors only, never also "of
        // this directory", though the root is a declaration of the directory.
        let files = [
            ("a.gcsl", "x = 1\nlet x = 2\ny.z = 3\nfn y(): Int { 1 }"),
            ("b.gcsl", "w = 1"),
        ];
        assert_eq!(
            messages(&files, "a.gcsl"),
            [
                "`x` is a field of this file",
                "`y` is a function of this file"
            ]
        );
    }

    #[test]
    fn a_future_of_another_file_runs_once() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "p = await f\nq = await f"),
            ("b.gcsl", "r = { a = 20 }\nf = lazy { v = r.a + 1 }"),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        let index = workspace.file("a.gcsl").unwrap();
        let Value::Record(record) = workspace.run(index).unwrap().value else {
            panic!("expected a record");
        };
        // Both awaits give the one record its body built, reading the
        // field of `b.gcsl` from the frame.
        let (Some(Value::Record(p)), Some(Value::Record(q))) =
            (record.values.first(), record.values.get(1))
        else {
            panic!("expected records");
        };
        assert!(std::rc::Rc::ptr_eq(p, q));
        assert_eq!(Value::Record(p.clone()).to_string(), "{v = 21}");
    }

    #[test]
    fn spreads_and_paths_read_other_files() {
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "com.lib.name = \"n\"\ncom.lib.other = [1][0]\nr = { k = 1 }",
            ),
            ("b.gcsl", "n = com.lib.name\n...r\n...{ z = com.lib.name }"),
            ("c.gcsl", "...com.lib\nall = com"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "{n = \"n\"; k = 1; z = \"n\"}");
        assert_eq!(
            value(&files, "c.gcsl"),
            "{name = \"n\"; other = 1; all = {lib = {name = \"n\"; other = 1}}}"
        );
    }

    #[test]
    fn empty_and_value_files() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x = 3"),
            ("b.gcsl", "= x + 1"),
            ("c.gcsl", "y = x"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "4");
        assert_eq!(value(&files, "workspace.gcsl"), "{}");
        assert_eq!(value(&files, "c.gcsl"), "{y = 3}");
    }

    #[test]
    fn a_needed_file_runs_whole_first_and_its_error_names_it() {
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def boom(): Int { [1][5] }\nx = boom()"),
            ("b.gcsl", "y = x"),
        ];
        let errors = run_file(&files, "b.gcsl").unwrap_err();
        // `a.gcsl` is input 1, after the prelude: the error names it, and
        // `b.gcsl` never ran.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(1));
        let erroneous = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x = 1"),
            ("z.gcsl", "y = 1 + \"\""),
        ];
        let errors = run_file(&erroneous, "a.gcsl").unwrap_err();
        // The error is `z.gcsl`'s, input 3.
        assert!(errors.iter().all(|error| error.1 == Some(3)), "{errors:?}");
        // Every failing file is reported, in path order; a file that needs
        // one that failed is not checked.
        let erroneous = [
            ("a.gcsl", "x = 1 + \"\""),
            ("b.gcsl", "y = x\nz = undefined"),
            ("c.gcsl", "w = 2 + \"\""),
            ("workspace.gcsl", ""),
        ];
        let errors = run_file(&erroneous, "workspace.gcsl").unwrap_err();
        let inputs: Vec<_> = errors.iter().map(|error| error.1).collect();
        assert_eq!(inputs, [Some(1), Some(3)], "{errors:?}");
        // A closure of another file names that file when it fails.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def boom(): Int { [1][5] }"),
            ("b.gcsl", "y = boom()"),
        ];
        let errors = run_file(&files, "b.gcsl").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, Some(1), "{errors:?}");
    }

    /// The time to check a workspace of `count` files of ten fields each,
    /// every file reading a field of the one before, and to run its last
    /// file, which needs every other: linear in `count`. Run with
    /// `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_chain_of_files() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let sources: Vec<SourceFile> = (0..count)
                .map(|file| {
                    let mut text = if file == 0 {
                        String::from("f0_0 = 1\n")
                    } else {
                        format!("f{file}_0 = f{}_9 + 1\n", file - 1)
                    };
                    for field in 1..10 {
                        writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                    }
                    SourceFile::new(format!("f{file:04}.gcsl"), text).unwrap()
                })
                .collect();
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace.file(&format!("f{:04}.gcsl", count - 1)).unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            let last = format!("{}", outcome.value);
            assert!(last.contains(&format!("= {}", count * 10)), "{last}");
            writeln!(
                report,
                "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// The time to check and run a lone file of `count` fields, each reading
    /// the one before, beside the standard library ([`check_lone`]), against
    /// checking, lowering and running it alone as [`crate::check::check`]
    /// does, with no standard library: what the prelude and the path of a
    /// workspace cost a lone file. Run with `cargo test --release --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_lone_file_beside_the_prelude() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [1_u32, 1000, 2000, 4000, 8000] {
            let mut text = String::from("f0 = 1\n");
            for field in 1..count {
                writeln!(text, "f{field} = f{} + 1", field - 1).unwrap();
            }
            let source = SourceFile::new("lone.gcsl", text).unwrap();
            let start = std::time::Instant::now();
            let (tokens, names, strings) = lexer::tokenize(&source).unwrap();
            let ast = parser::parse(&tokens, names, strings, &source).unwrap();
            let mut types = TypeTable::new();
            let (checked, _) = crate::check::check(&ast, &mut types).unwrap();
            let ir = std::rc::Rc::new(crate::lower::lower(&ast, &checked, &types, 0).unwrap());
            let alone = crate::eval::eval(&ir).unwrap();
            let bare = start.elapsed();
            // Its scan and its check, apart.
            let start = std::time::Instant::now();
            let scanned = scan_into(
                (std::slice::from_ref(&source), Placing::Lone(&[], false)),
                Library::NEEDED,
                &mut Names::new(),
                &mut Vec::new(),
            )
            .unwrap();
            let scanning = start.elapsed();
            let start = std::time::Instant::now();
            check_scan(scanned).unwrap();
            let checking = start.elapsed();
            let start = std::time::Instant::now();
            let lone = crate::run(&source).unwrap().value;
            let beside = start.elapsed();
            assert_eq!(alone.to_string(), lone.to_string());
            writeln!(
                report,
                "{count} fields: alone {bare:?}; beside the standard library {beside:?} \
                 (scan {scanning:?}, check {checking:?})"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// [`benchmark_a_chain_of_files`] through imports: each file in a
    /// directory of its own, importing the previous file's directory and
    /// reading one leaf through it. Linear in `count`, as the chain is.
    /// Run with `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_chain_of_imports() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let sources: Vec<SourceFile> = (0..count)
                .map(|file| {
                    let mut text = if file == 0 {
                        String::from("f0_0 = 1\n")
                    } else {
                        let before = file - 1;
                        format!("import d{before:04}\nf{file}_0 = d{before:04}.f{before}_9 + 1\n")
                    };
                    for field in 1..10 {
                        writeln!(text, "f{file}_{field} = f{file}_{} + 1", field - 1).unwrap();
                    }
                    SourceFile::new(format!("d{file:04}/f.gcsl"), text).unwrap()
                })
                .collect();
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace
                .file(&format!("d{:04}/f.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            let last = format!("{}", outcome.value);
            assert!(last.contains(&format!("= {}", count * 10)), "{last}");
            writeln!(
                report,
                "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// The files of the imports summary, in path order.
    const IMPORTS: [(&str, &str); 6] = [
        (
            "app/main.gcsl",
            "import lib\nimport tools.build as builder\nimport com as libs from lib\n\
             import lib.y\ncom.x.app.deps = [lib.com.x.lib, libs.y.lib]\n\
             com.x.app.tool = builder\ncom.x.app.all = y.com.y.lib.name",
        ),
        ("common.gcsl", "version = \"1.2\""),
        ("lib/x.gcsl", "com.x.lib.name = \"lib-{version}\""),
        ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
        ("tools/build.gcsl", "= \"build-{version}\""),
        ("workspace.gcsl", ""),
    ];

    /// The scan of `files` told that `nested` are workspaces of their own.
    fn ws_nested(files: &[(&str, &str)], nested: &[&str]) -> Result<Scan, Vec<Diagnostic>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let nested: Vec<String> = nested.iter().map(|&n| n.to_owned()).collect();
        scan_with(&sources, &nested)
    }

    /// The message and note of the single error of scanning `files`.
    fn noted(files: &[(&str, &str)], nested: &[&str]) -> (String, Option<String>) {
        let errors = ws_nested(files, nested).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        (errors[0].message.clone(), errors[0].notes.first().cloned())
    }

    #[test]
    fn imports_resolve_to_their_targets() {
        let scan = ws(&IMPORTS).unwrap();
        let main = &scan.files[1];
        // Its own imports; the implicit prelude's names follow them.
        let named: Vec<(&str, Target)> = main
            .aliases
            .iter()
            .filter(|a| !a.implicit)
            .map(|a| (scan.names.get(a.name), a.target))
            .collect();
        let lib = scan.dirs.iter().position(|d| d.path == "lib").unwrap();
        let com = scan.by_name[&(count(lib), ROOT, scan.names.lookup("com").unwrap())];
        assert_eq!(
            named,
            [
                ("lib", Target::Dir(count(lib))),
                ("builder", Target::File(5)),
                ("libs", Target::Node(com)),
                ("y", Target::File(4)),
            ]
        );
        // Needs per leaf: x, y and build, each through an import; `common`
        // only through them.
        assert_eq!(scan.needs[1], [3, 4, 5]);
        assert_eq!(scan.needed(1), [0, 2, 3, 4, 5, 1]);
        assert_eq!(scan.value_slot(5), Some(Slot::at(count(scan.nodes.len()))));
        assert_eq!(scan.value_slot(2), None);
    }

    #[test]
    fn a_reference_through_an_import_needs_only_its_leaf() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport lib.y\nimport tools\na = lib.com.x.lib.name\nb = y";
        let scan = ws(&files).unwrap();
        // `lib.com.x.lib.name` is x's leaf; `y` whole is y; nothing reads
        // `tools`, so it needs nothing.
        assert_eq!(scan.needs[1], [3, 4]);
        files[0].1 = "import lib\na = lib";
        assert_eq!(ws(&files).unwrap().needs[1], [3, 4]);
        files[0].1 = "import lib\na = lib.com.y";
        assert_eq!(ws(&files).unwrap().needs[1], [4]);
    }

    #[test]
    fn a_file_part_and_a_whole_node_are_told_apart() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib.y\na = y.com\nb = y.com.y";
        let scan = ws(&files).unwrap();
        let main = &scan.files[1];
        let scope = FileScope::new(&scan, main);
        let targets: Vec<Target> = main
            .mentions
            .foreign
            .iter()
            .flatten()
            .filter_map(|&(need, _)| match need {
                Need::Field(n) => scope.target(n),
                Need::Function(_) => None,
            })
            .collect();
        // `com` of `lib/` is shared with x: y's part of it. `com.y` is y's
        // alone: the merged node.
        assert!(
            matches!(targets[0], Target::Part { file: 4, .. }),
            "{targets:?}"
        );
        assert!(matches!(targets[1], Target::Node(_)), "{targets:?}");
    }

    #[test]
    fn no_module_says_what_there_is() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import nope";
        let (message, note) = noted(&files, &["vendor"]);
        assert_eq!(message, "no module `nope`");
        assert_eq!(
            note.as_deref(),
            Some("the workspace has `app/`, `lib/`, `tools/`, `common`, `workspace`")
        );
        files[0].1 = "import lib.z";
        let (message, note) = noted(&files, &[]);
        assert_eq!(message, "no module `lib.z`");
        assert_eq!(note.as_deref(), Some("`lib/` has `x`, `y`"));
        files[0].1 = "import lib.x.z";
        assert_eq!(noted(&files, &[]).0, "no module `lib.x.z`");
        files[0].1 = "import vendor.pkg";
        let (_, note) = noted(&files, &["vendor"]);
        assert_eq!(note.as_deref(), Some("`vendor/` is a workspace of its own"));
    }

    /// An import that nothing mentions needs nothing, but is still resolved:
    /// a broken one stops the run of every file, even one that does not
    /// import anything.
    #[test]
    fn an_import_nothing_mentions_is_still_resolved() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport nope\na = 1";
        let errors = run_file(&files, "lib/x.gcsl").unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].0, "no module `nope`", "{errors:?}");
        files[0].1 = "import lib\na = 1";
        assert!(run_file(&files, "app/main.gcsl").is_ok());
    }

    #[test]
    fn a_directory_and_a_file_of_one_name_are_reported_once() {
        let mut files = IMPORTS.to_vec();
        files[0].1 = "import lib\nimport lib as again";
        files.push(("lib.gcsl", "z = 1"));
        let (message, _) = noted(&files, &[]);
        assert_eq!(message, "`lib` is both a directory and a file");
        files[0].1 = "a = 1";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn imported_names_clash() {
        let mut files = IMPORTS.to_vec();
        let imported = "is imported in this file";
        for (text, message) in [
            ("import lib\nimport tools as lib", imported),
            ("import lib\nfn lib(): Int { 1 }", imported),
            ("import lib\nlet lib = 1", imported),
            ("import lib\nlib.a = 1", imported),
            ("import tools as version", "is a field of this directory"),
            ("import x from lib.x", "has no `x`"),
        ] {
            files[0].1 = text;
            assert!(noted(&files, &[]).0.ends_with(message), "{text}");
        }
        // A member or a `*` root merges with the file's own root of its
        // name.
        for text in [
            "import com from lib\ncom.a = 1",
            "import * from lib\ncom.a = 1",
        ] {
            files[0].1 = text;
            assert!(ws(&files).is_ok(), "{text}");
        }
        files[0].1 = "import * from lib\na = com.x.lib.name";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn reaching_into_your_own_file_through_an_import_needs_itself() {
        let mut files = IMPORTS.to_vec();
        files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nall = lib";
        let (message, _) = noted(&files, &[]);
        assert_eq!(message, "`lib/x.gcsl` needs itself");
        files[2].1 = "import lib\ncom.x.lib.name = \"x\"\nother = lib.com.y.lib.name";
        assert!(ws(&files).is_ok());
    }

    #[test]
    fn files_importing_each_other_are_a_cycle() {
        let (message, _) = noted(
            &[
                ("a.gcsl", "import b\nx = b.y"),
                ("b.gcsl", "import a\ny = a.x"),
                ("workspace.gcsl", ""),
            ],
            &[],
        );
        assert_eq!(message, "`a.gcsl` and `b.gcsl` need each other");
        // A whole read of a directory one of whose files needs this one is
        // the one-file form, at the read, and no cycle of two files.
        let files = [
            ("app.gcsl", "import lib\nall = lib"),
            ("lib/y.gcsl", "import app\ny = app.all"),
            ("workspace.gcsl", ""),
        ];
        let errors = match ws(&files) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        };
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["`app.gcsl` needs itself"]);
    }

    #[test]
    fn the_imports_summary_runs() {
        assert_eq!(
            value(&IMPORTS, "app/main.gcsl"),
            "{com = {x = {app = {deps = [{name = \"lib-1.2\"}, {name = \"ylib\"}]; \
             tool = \"build-1.2\"; all = \"ylib\"}}}}"
        );
    }

    /// A workspace of a library directory with a `def`, a leaf and an
    /// interior node in one file, a file failing at runtime if it runs, and
    /// `main` as given.
    fn library(main: &str) -> Vec<(&str, &str)> {
        vec![
            ("app/main.gcsl", main),
            ("lib/bad.gcsl", "def bad(): Int { [1][5] }\nboom = bad()"),
            (
                "lib/f.gcsl",
                "def twice(n: Int): Int { n * 2 }\nleafy = 3\ncom.k.v = 1",
            ),
            ("lib/sub/s.gcsl", "import lib\nv = lib.leafy"),
            ("workspace.gcsl", ""),
        ]
    }

    #[test]
    fn every_import_form_binds_its_names() {
        let files = library("import lib.f as lf\nimport lib.f\na = lf.leafy\nb = f.com.k.v");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3; b = 1}");
        let main = "import twice, leafy as l, com from lib\na = twice(l)\nb = com.k\nc = com.k.v";
        let files = library(main);
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{a = 6; b = {v = 1}; c = 1}"
        );
        let files = library("import * from lib.f\na = twice(leafy)\nb = com");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 6; b = {k = {v = 1}}}");
        // Each root of `*` is a member: `lib/bad.gcsl`'s `boom` is not read.
        let files = library("import * from lib\na = twice(leafy)");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 6}");
    }

    #[test]
    fn a_def_is_read_from_its_functions_slot() {
        let files = library("import lib\nimport twice from lib.f\n= lib.twice(1) + twice(2)");
        assert_eq!(value(&files, "app/main.gcsl"), "6");
        let main = "import lib.f\nfn g(): Int { let r = f\nr.twice(5) }\nr = f.com\nn = g()";
        assert_eq!(
            value(&library(main), "app/main.gcsl"),
            "{r = {k = {v = 1}}; n = 10}"
        );
        let main = "import lib.f\nlet r = f\n= r.twice(4) + r.leafy";
        assert_eq!(value(&library(main), "app/main.gcsl"), "11");
    }

    /// A file mentioning a record-valued leaf through an import from every
    /// kind of position, reading the directory whole and calling a `def`
    /// out of the record, at the top level and inside functions, a nested
    /// `def` and lazies.
    const POSITIONS: &str = r#"import lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := lib.com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy lib.com.x.lib.r
let lw = lazy lib
let g = fn(): {a: Int} { lib.com.x.lib.r }
let gw = fn(): Int { let w = lib; = w.twice(lib.com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = lib
    = lib.com.x.lib.r.a + w.com.x.lib.r.a
  }
}
let w = lib
p01 = "{lib.com.x.lib.r}"
p02 = xs[lib.com.x.lib.r.a]
p03 = m
p04 = for v in [lib.com.x.lib.r] { v.a }
p05 = for v in [1] if lib.com.x.lib.r.a == 42 { v }
p06 = for let i = lib.com.x.lib.r.a; i < lib.com.x.lib.r.a + 1; i := i + lib.com.x.lib.r.a { i }
p07 = for k < lib.com.x.lib.r.a { k := k + 50; lib.com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < lib.com.x.lib.r.a)
p09 = true and lib.com.x.lib.r.a == 42
p10 = lib.com.x.lib.r is {a: Int}
p11 = (lib.com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue lib.com.x.lib.r }; break lib.com.x.lib.r }
p13 = get(config = lib.com.x.lib.r)
p14 = wrap(lib.com.x.lib.r) { ...lib.com.x.lib.r }
p15 = { ...lib.com.x.lib.r }
p16 = len(0..lib.com.x.lib.r.a)
p17 = [lib.com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).com.x.lib.r.a
p20 = g()
p21 = { let q = lib.com.x.lib.r; = q }
p22 = w.twice(w.com.x.lib.r.a)
p23 = gw()
p24 = w.com"#;

    #[test]
    fn records_read_whole_have_the_types_of_their_roots() {
        let files = [
            ("app/main.gcsl", "import lib\nimport lib.x\nd = lib\nf = x"),
            (
                "lib/x.gcsl",
                "com.x.lib.n = 1\ndef twice(n: Int): Int { n * 2 }",
            ),
            ("lib/y.gcsl", "top = \"y\""),
            ("workspace.gcsl", ""),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        let index = workspace.file("app/main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        // A directory's record has every file's roots, a `def`'s its
        // function's type; a file's, its own roots.
        assert!(dump.contains(
            "(name lib : {com: {x: {lib: {n: Int}}}, top: String, twice: fn(n: Int): Int}"
        ));
        assert!(dump.contains("(name x : {com: {x: {lib: {n: Int}}}, twice: fn(n: Int): Int}"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{d = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int; top = \"y\"}; \
             f = {com = {x = {lib = {n = 1}}}; twice = fn(n: Int): Int}}"
        );
    }

    #[test]
    fn a_call_of_an_imported_def_is_of_a_known_callee() {
        // The call-site form of the implicit-await error, as for a `def` of
        // the directory, through a member and through a path.
        let files = [
            (
                "app/main.gcsl",
                "import inc from lib\nimport lib\n\
                 fn p(f: Future<Int>): Int { inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.inc(f) }",
            ),
            ("lib/x.gcsl", "def inc(n: Int): Int { n + 1 }"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            [
                "argument `f` would be awaited inside `inc`, which is pure",
                "argument `f` would be awaited inside `lib.inc`, which is pure",
            ]
        );
    }

    #[test]
    fn an_import_is_read_from_every_position() {
        let files = [
            ("app/main.gcsl", POSITIONS),
            (
                "lib/x.gcsl",
                "let base = 40\ncom.x.lib.r = { a = base + 2 }\ndef twice(n: Int): Int { n * 2 }",
            ),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{outer = fn(): {inner: fn(): Int}; p01 = \"{{a = 42}}\"; p02 = 42; \
             p03 = {a = 42}; p04 = [42]; p05 = [1]; p06 = [42]; p07 = [{a = 42}]; \
             p08 = 42; p09 = true; p10 = true; p11 = 42; p12 = [{a = 42}, {a = 42}]; \
             p13 = 42; p14 = 84; p15 = {a = 42}; p16 = 42; p17 = [{a = 42}]; p18 = 84; \
             p19 = 84; p20 = {a = 42}; p21 = {a = 42}; p22 = 84; p23 = 84; \
             p24 = {x = {lib = {r = {a = 42}}}}}"
        );
    }

    #[test]
    fn needs_through_an_import_are_per_leaf() {
        let files = library("import lib\na = lib.leafy");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
        let files = library("import lib\na = lib.com.k");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = {v = 1}}");
        let files = library("import lib\nfn g(): Int { lib.leafy }\na = g()");
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
        let main =
            "import lib.f\ndef d(): Int { f.com.k.v + (fn(): Int { f.twice(1) })() }\na = d()";
        assert_eq!(
            value(&library(main), "app/main.gcsl"),
            "{d = fn(): Int; a = 3}"
        );
        // Read whole, every file of `lib/` runs, `lib/bad.gcsl` too.
        let messages = messages(
            &library("import lib\nlet r = lib\na = r.leafy"),
            "app/main.gcsl",
        );
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert_eq!(value(&library(""), "lib/sub/s.gcsl"), "{v = 3}");
    }

    #[test]
    fn a_value_file_is_imported_by_its_value() {
        let files = [
            ("a.gcsl", "import b\nimport c\nx = b + 1\ny = c.z"),
            ("b.gcsl", "= 41"),
            ("c.gcsl", "z = 2"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a.gcsl"), "{x = 42; y = 2}");
        assert_eq!(value(&files, "b.gcsl"), "41");
    }

    #[test]
    fn imported_names_are_the_files_own() {
        let main =
            "a = leafy + g(10)\nfn g(leafy: Int): Int { leafy + 1 }\nimport leafy from lib.f";
        assert_eq!(value(&library(main), "app/main.gcsl"), "{a = 14}");
        let mut files = library("import leafy from lib.f\na = leafy");
        files.push(("app/other.gcsl", "b = leafy"));
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            ["undefined variable `leafy`"]
        );
        files.pop();
        assert_eq!(value(&files, "app/main.gcsl"), "{a = 3}");
    }

    #[test]
    fn an_importing_files_items_are_in_any_order() {
        let lines = [
            "import lib.f",
            "a = f.leafy + b",
            "b = twice(2)",
            "import twice from lib.f",
        ];
        // Every rotation, and (as slice 23's helper in lowering, which runs
        // a file alone, compares) the reverse order.
        let reversed: Vec<&str> = lines.iter().rev().copied().collect();
        let orders = (0..lines.len())
            .map(|start| {
                let mut rotated = lines.to_vec();
                rotated.rotate_left(start);
                rotated
            })
            .chain([reversed]);
        for order in orders {
            let main = order.join("\n");
            let mut record: Vec<String> = value(&library(&main), "app/main.gcsl")
                .trim_matches(['{', '}'])
                .split("; ")
                .map(str::to_owned)
                .collect();
            record.sort();
            assert_eq!(record, ["a = 7", "b = 4"], "{main}");
        }
    }

    /// The files of the merging summary (slice 27), in path order: `pkgs/`
    /// is the root.
    const MERGE: [(&str, &str); 5] = [
        (
            "app/main.gcsl",
            "import * from lib\ncom.x.app.deps = [com.x.lib, com.y.lib]\n\
             com.x.app.base = com.base.name",
        ),
        ("app/vendor/v.gcsl", "com.vendor.thing = 1\nall = com"),
        ("lib/x.gcsl", "com.x.lib.name = \"lib\""),
        ("lib/y.gcsl", "com.y.lib.name = \"ylib\""),
        ("workspace.gcsl", "com.base.name = \"base\""),
    ];

    /// The single error of scanning `files`, shown: its message and file,
    /// its label, each other label's file and text, and its notes.
    fn shown_error(files: &[(&str, &str)]) -> String {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut loaded = Vec::new();
        let placed = (&sources[..], Placing::Workspace(&[]));
        let errors =
            scan_into(placed, Library::NEEDED, &mut Names::new(), &mut loaded).unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        // The standard files the scan loaded take the first input numbers.
        let standard = Library::NEEDED.sources(&loaded);
        let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        all.extend(paths);
        let path = |input: Option<u32>| {
            input
                .and_then(|i| all.get(i as usize))
                .copied()
                .unwrap_or("?")
        };
        let error = &errors[0];
        let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
        if let Some(label) = &error.label {
            parts[0] = format!("{} ({label})", parts[0]);
        }
        for label in &error.labels {
            let file = path(label.input.or(error.input));
            parts.push(format!("{file}: {}", label.text));
        }
        for note in &error.notes {
            parts.push(format!("note: {note}"));
        }
        parts.join("; ")
    }

    /// The checked workspace of `files`.
    fn checked(files: &[(&str, &str)]) -> Workspace {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        check(&sources).unwrap()
    }

    /// Whether the first item of the top-level block of the file at `path`
    /// of `workspace` stores a record: one assembled as the file starts.
    fn assembled_first(workspace: &Workspace, path: &str) -> bool {
        let index = workspace.file(path).unwrap();
        let ir = workspace.irs[index as usize].as_ref().unwrap();
        let IrKind::Block { items, .. } = ir.get(ir.root()).kind else {
            panic!("expected a block")
        };
        let first = ir.items(items)[0];
        matches!(ir.get(first).kind, IrKind::Store(_, value)
            if matches!(ir.get(value).kind, IrKind::MakeRecord { .. }))
    }

    #[test]
    fn the_merging_summary_runs_both_files() {
        assert_eq!(
            value(&MERGE, "app/main.gcsl"),
            "{com = {x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; \
             base = \"base\"}}}}"
        );
        // `all` has the root's part, `app/`'s and v's own, root directory
        // first, but not `lib/`'s, which v does not import.
        assert_eq!(
            value(&MERGE, "app/vendor/v.gcsl"),
            "{com = {vendor = {thing = 1}}; all = {base = {name = \"base\"}; \
             x = {app = {deps = [{name = \"lib\"}, {name = \"ylib\"}]; base = \"base\"}}; \
             vendor = {thing = 1}}}"
        );
        // v owns a leaf of `com`: its view's `com` is a node of its own
        // schedule, after its own child and before `all`.
        let scan = ws(&MERGE).unwrap();
        let v = &scan.files[2];
        assert_eq!((v.view.own, v.view.end - v.view.start), (1, 1));
        let number = count(v.global.len());
        assert!(v.schedule.order.contains(&number));
        assert_eq!(v.schedule.point[number as usize], 1);
        // main reads no node of its view whole: nothing is assembled.
        let main = &scan.files[1];
        let own = count(main.global.len());
        assert!(main.schedule.order.iter().all(|&n| n < own));
        // Its `com` is the root's, `app/`'s and `lib/`'s, root first.
        let com = &scan.views[main.view.start as usize];
        let sources: Vec<u32> = com.parts.iter().map(|p| p.source).collect();
        assert_eq!(sources, [0, 1, 2]);
        // v's type of `all` is the record of the merged children.
        let mut workspace = checked(&MERGE);
        let dump = workspace.dump_types(2).unwrap();
        assert!(
            dump.contains(
                "(name com : {base: {name: String}, vendor: {thing: Int}, x: {app: {base: \
                 String, deps: List<{name: String}>}}}"
            ),
            "{dump}"
        );
    }

    /// [`POSITIONS`] through a merged node: `com` is the file's own and
    /// `lib/`'s, imported with `*`, and `com.x.lib.r` is `lib/`'s leaf, read
    /// from every kind of position; `com` is read whole too (a node of the
    /// file's schedule, the file owning `com.x.app.name`), and a function
    /// out of that record called. A `def` is a root, never a leaf under
    /// `com`: `com.x.lib.f` is a field holding one, and `twice` itself is
    /// called through the `*`.
    const MERGED_POSITIONS: &str = r#"import * from lib
fn get(config: {a: Int}): Int { config.a }
fn wrap(v: {a: Int}, w: {a: Int}): Int { v.a + w.a }
let xs = for i in 0..50 { i }
let mut m = { a = 0 }
m := com.x.lib.r
let mut k = 0
let mut t = 0
let lz = lazy com.x.lib.r
let lw = lazy com
let g = fn(): {a: Int} { com.x.lib.r }
let gw = fn(): Int { let w = com; = w.x.lib.f(com.x.lib.r.a) }
def outer(): {inner: fn(): Int} {
  def inner(): Int {
    let w = com
    = com.x.lib.r.a + w.x.lib.r.a
  }
}
let w = com
com.x.app.name = "app"
p01 = "{com.x.lib.r}"
p02 = xs[com.x.lib.r.a]
p03 = m
p04 = for v in [com.x.lib.r] { v.a }
p05 = for v in [1] if com.x.lib.r.a == 42 { v }
p06 = for let i = com.x.lib.r.a; i < com.x.lib.r.a + 1; i := i + com.x.lib.r.a { i }
p07 = for k < com.x.lib.r.a { k := k + 50; com.x.lib.r }
p08 = len(do { t := t + 1; t } for t < com.x.lib.r.a)
p09 = true and com.x.lib.r.a == 42
p10 = com.x.lib.r is {a: Int}
p11 = (com.x.lib.r as {a: Int}).a
p12 = for v in [1, 2] { if v == 1 { continue com.x.lib.r }; break com.x.lib.r }
p13 = get(config = com.x.lib.r)
p14 = wrap(com.x.lib.r) { ...com.x.lib.r }
p15 = { ...com.x.lib.r }
p16 = len(0..com.x.lib.r.a)
p17 = [com.x.lib.r]
p18 = outer().inner()
p19 = (await lz).a + (await lw).x.lib.r.a
p20 = g()
p21 = { let q = com.x.lib.r; = q }
p22 = w.x.lib.f(w.x.lib.r.a)
p23 = gw()
p24 = twice(21)
p25 = com
p26 = com.x"#;

    #[test]
    fn a_merged_node_is_read_from_every_position() {
        let files = [
            ("app/main.gcsl", MERGED_POSITIONS),
            (
                "lib/x.gcsl",
                "let base = 40\ncom.x.lib.r = { a = base + 2 }\ncom.x.lib.f = twice\n\
                 def twice(n: Int): Int { n * 2 }",
            ),
            ("workspace.gcsl", ""),
        ];
        let lib = "lib = {r = {a = 42}; f = fn(n: Int): Int}";
        assert_eq!(
            value(&files, "app/main.gcsl"),
            format!(
                "{{outer = fn(): {{inner: fn(): Int}}; com = {{x = {{app = {{name = \"app\"}}}}}}; \
                 p01 = \"{{{{a = 42}}}}\"; p02 = 42; p03 = {{a = 42}}; p04 = [42]; p05 = [1]; \
                 p06 = [42]; p07 = [{{a = 42}}]; p08 = 42; p09 = true; p10 = true; p11 = 42; \
                 p12 = [{{a = 42}}, {{a = 42}}]; p13 = 42; p14 = 84; p15 = {{a = 42}}; \
                 p16 = 42; p17 = [{{a = 42}}]; p18 = 84; p19 = 84; p20 = {{a = 42}}; \
                 p21 = {{a = 42}}; p22 = 84; p23 = 84; p24 = 42; \
                 p25 = {{x = {{app = {{name = \"app\"}}; {lib}}}}}; \
                 p26 = {{app = {{name = \"app\"}}; {lib}}}}}"
            )
        );
    }

    #[test]
    fn star_and_member_imports_merge_with_the_files_own_root() {
        let lib = [
            ("lib/x.gcsl", "com.x.lib.name = \"x\""),
            ("lib/y.gcsl", "com.y.lib.name = \"y\""),
            ("workspace.gcsl", ""),
        ];
        // A directory's tree, a file's own tree, and a member, each with an
        // own `com`: the import side's children after the file's own.
        for (main, all) in [
            (
                "import * from lib\ncom.app = 1\nall = com",
                "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
            ),
            (
                "import * from lib.y\ncom.app = 1\nall = com",
                "{app = 1; y = {lib = {name = \"y\"}}}",
            ),
            (
                "import com from lib\ncom.app = 1\nall = com",
                "{app = 1; x = {lib = {name = \"x\"}}; y = {lib = {name = \"y\"}}}",
            ),
        ] {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            assert_eq!(
                value(&files, "app/main.gcsl"),
                format!("{{com = {{app = 1}}; all = {all}}}"),
                "{main}"
            );
        }
        // A member that is a leaf, and one that is a `def`.
        let mut files = lib.to_vec();
        files.push(("lib/z.gcsl", "n = 1\ndef twice(k: Int): Int { k * 2 }"));
        files.push((
            "app/main.gcsl",
            "import n, twice from lib\nm = twice(n + 1)",
        ));
        assert_eq!(value(&files, "app/main.gcsl"), "{m = 4}");
    }

    #[test]
    fn a_subdirectory_merges_under_a_root_above_two_levels_deep() {
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("x/b.gcsl", "com.b = 2"),
            ("x/y/c.gcsl", "com.c = 3\nall = com\nb = com.b"),
        ];
        assert_eq!(
            value(&files, "x/y/c.gcsl"),
            "{com = {c = 3}; all = {a = 1; b = 2; c = 3}; b = 2}"
        );
        assert_eq!(value(&files, "x/b.gcsl"), "{com = {b = 2}}");
    }

    /// A whole read of each kind of node of the view: who assembles it, and
    /// when, with its contents and their order.
    #[test]
    fn a_view_node_read_whole_of_each_kind() {
        // One directory's tree, without an import: slice 25's rule, the
        // last contributor's schedule; the view has no node of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "com.one = 1"),
            ("b.gcsl", "com.two = 2\nall = com"),
        ];
        let scan = ws(&files).unwrap();
        assert!(scan.views.is_empty());
        assert_eq!(
            value(&files, "b.gcsl"),
            "{com = {two = 2}; all = {one = 1; two = 2}}"
        );
        // Across directories, with no leaf of the reading file: assembled
        // as the file starts, the root directory's part first.
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("sub/b.gcsl", "com.b = 2"),
            ("sub/c.gcsl", "x = 0\nall = com"),
        ];
        assert_eq!(value(&files, "sub/c.gcsl"), "{x = 0; all = {a = 1; b = 2}}");
        assert!(assembled_first(&checked(&files), "sub/c.gcsl"));
        // Across directories, with a leaf of the reading file: a node of
        // its schedule, after its own leaf, so not first.
        let files = [
            ("workspace.gcsl", "com.a = 1"),
            ("sub/b.gcsl", "com.b = 2\nall = com"),
        ];
        assert_eq!(
            value(&files, "sub/b.gcsl"),
            "{com = {b = 2}; all = {a = 1; b = 2}}"
        );
        assert!(!assembled_first(&checked(&files), "sub/b.gcsl"));
        // Across imports only, with no leaf of the reading file: as the
        // file starts, in the imports' source order.
        let files = [
            (
                "app/main.gcsl",
                "import * from tools\nimport * from lib\nall = com",
            ),
            ("lib/x.gcsl", "com.x = 1"),
            ("tools/t.gcsl", "com.t = 2"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "app/main.gcsl"), "{all = {t = 2; x = 1}}");
        assert!(assembled_first(&checked(&files), "app/main.gcsl"));
        // Mixed: a directory above, the file's own directory, an import and
        // a leaf of the file, two levels of the view merged; `com.x` is
        // read whole too, and a part of it alone.
        let files = [
            (
                "app/main.gcsl",
                "import * from lib\ncom.x.app = 1\nall = com\nx = com.x",
            ),
            ("app/other.gcsl", "com.other = 2"),
            ("lib/x.gcsl", "com.x.lib = 3"),
            ("workspace.gcsl", "com.x.root = 4\ncom.base = 5"),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {x = {app = 1}}; all = {x = {root = 4; app = 1; lib = 3}; base = 5; \
             other = 2}; x = {root = 4; app = 1; lib = 3}}"
        );
        let scan = ws(&files).unwrap();
        let main = &scan.files[1];
        // `com` and `com.x`, both with a leaf of main under them.
        assert_eq!((main.view.own, main.view.end - main.view.start), (2, 2));
    }

    #[test]
    fn the_needs_of_a_view_node_are_per_leaf() {
        let base = [
            ("workspace.gcsl", ""),
            ("bad.gcsl", "def bad(): Int { [1][5] }\ncom.bad.v = bad()"),
            ("ok.gcsl", "com.ok.v = 1"),
        ];
        // A leaf read alone needs its own file only: `bad.gcsl` never runs.
        let mut files = base.to_vec();
        files.push(("app/main.gcsl", "com.app.v = 2\na = com.ok.v"));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{com = {app = {v = 2}}; a = 1}"
        );
        // Read whole, every owner of every leaf runs, `bad.gcsl` too.
        files.pop();
        files.push(("app/main.gcsl", "com.app.v = 2\na = com"));
        assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
        files.pop();
        files.push(("app/main.gcsl", "a = com"));
        assert_eq!(messages(&files, "app/main.gcsl"), ["index out of range"]);
    }

    #[test]
    fn conflicts_across_directories() {
        let above = "one is in a directory above";
        // Two leaves, at the directory above, whichever path is first.
        assert_eq!(
            shown_error(&[("a.gcsl", "version = 1"), ("sub/b.gcsl", "version = 2")]),
            format!(
                "field `version` is defined in two files at a.gcsl (defined here); \
                 sub/b.gcsl: defined here; note: {above}"
            )
        );
        assert_eq!(
            shown_error(&[("z.gcsl", "version = 1"), ("a/b.gcsl", "version = 2")]),
            format!(
                "field `version` is defined in two files at z.gcsl (defined here); \
                 a/b.gcsl: defined here; note: {above}"
            )
        );
        // A whole value extended by a path below, and the other way.
        let defined = format!(
            "field `com` is already defined at a.gcsl; sub/b.gcsl: defined here; note: {above}"
        );
        assert_eq!(
            shown_error(&[("a.gcsl", "com = 1"), ("sub/b.gcsl", "com.x = 2")]),
            defined
        );
        assert_eq!(
            shown_error(&[("a.gcsl", "com.x = 1"), ("sub/b.gcsl", "com = 2")]),
            defined
        );
        // A `def` is a leaf: with a field, either way, and with a `def`.
        for (a, b) in [
            ("def f(): Int { 1 }", "f = 2"),
            ("f = 2", "def f(): Int { 1 }"),
            ("def f(): Int { 1 }", "def f(): Int { 2 }"),
        ] {
            assert_eq!(
                shown_error(&[("a.gcsl", a), ("sub/b.gcsl", b)]),
                format!(
                    "field `f` is defined in two files at a.gcsl (defined here); \
                     sub/b.gcsl: defined here; note: {above}"
                ),
                "{a} / {b}"
            );
        }
        // Once per pair, however many files' views have it.
        assert_eq!(
            ws(&[
                ("a.gcsl", "version = 1"),
                ("sub/b.gcsl", "version = 2"),
                ("sub/c.gcsl", "x = 1"),
                ("sub/d/e.gcsl", "y = 1"),
            ])
            .unwrap_err()
            .len(),
            1
        );
    }

    #[test]
    fn conflicts_through_imports() {
        let lib = [
            ("lib/x.gcsl", "com.x.lib.name = \"x\""),
            ("workspace.gcsl", ""),
        ];
        let with = |main: &'static str, more: &[(&'static str, &'static str)]| {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            files.extend_from_slice(more);
            shown_error(&files)
        };
        // Two leaves, an own leaf and an imported interior node, and an own
        // interior node and an imported leaf: at the import.
        assert_eq!(
            with("import * from lib\ncom.x.lib.name = \"own\"", &[]),
            "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import * from lib\ncom.x.lib = 1", &[]),
            "field `com.x.lib` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import * from lib\ncom.x.lib.name.deep = 1", &[]),
            "field `com.x.lib.name` is already defined at app/main.gcsl (imported here); \
             lib/x.gcsl: defined here; app/main.gcsl: defined here"
        );
        // A member whose bound name is a plain root of the file, a leaf and
        // an interior node of it.
        let version = [("lib/v.gcsl", "version = 1")];
        assert_eq!(
            with("import version from lib\nversion = 2", &version),
            "field `version` is defined in two files at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
        );
        assert_eq!(
            with("import version from lib\nversion.minor = 2", &version),
            "field `version` is already defined at app/main.gcsl (imported here); \
             lib/v.gcsl: defined here; app/main.gcsl: defined here"
        );
        // Against a directory above the file (not above `lib/`), and
        // between two imports, at the later.
        let mut files = lib.to_vec();
        files.push(("app/sub/main.gcsl", "import * from lib"));
        files.push(("app/up.gcsl", "com.x.lib.name = \"up\""));
        assert_eq!(
            shown_error(&files),
            "field `com.x.lib.name` is defined in two files at app/sub/main.gcsl (imported \
             here); lib/x.gcsl: defined here; app/up.gcsl: defined here; note: one is in a \
             directory above"
        );
        assert_eq!(
            with(
                "import * from lib\nimport * from tools",
                &[("tools/t.gcsl", "com.x.lib.name = \"t\"")]
            ),
            "field `com.x.lib.name` is defined in two files at app/main.gcsl (imported here); \
             tools/t.gcsl: defined here; app/main.gcsl: imported here; lib/x.gcsl: defined here"
        );
    }

    #[test]
    fn a_view_is_the_files_own() {
        // b does not see what a imported with `*`, nor its own directory's
        // tree `lib/`'s part.
        let mut files = vec![
            ("app/a.gcsl", "import * from lib\nx = com.lib.v"),
            ("lib/l.gcsl", "com.lib.v = 1"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "app/a.gcsl"), "{x = 1}");
        files.push(("app/b.gcsl", "y = com.lib.v"));
        assert_eq!(messages(&files, "app/b.gcsl"), ["undefined variable `com`"]);
        files.pop();
        files.push(("app/b.gcsl", "com.app.v = 2\ny = com.lib.v"));
        assert_eq!(
            messages(&files, "app/b.gcsl"),
            ["no field `lib` in `{app: {v: Int}}`"]
        );
    }

    #[test]
    fn star_from_a_value_file_is_an_error() {
        let files = [
            ("app/main.gcsl", "import * from tools.build"),
            ("tools/build.gcsl", "= 1"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            shown_error(&files),
            "a value file has no members to import at app/main.gcsl"
        );
    }

    #[test]
    fn a_whole_import_still_clashes_with_a_root_of_the_view() {
        let files = |main: &'static str| {
            vec![
                ("app/main.gcsl", main),
                ("lib/x.gcsl", "com.x = 1"),
                ("tools/t.gcsl", "t = 1"),
                ("workspace.gcsl", ""),
            ]
        };
        for main in [
            "import * from lib\nimport tools as com",
            "import tools as com\nimport * from lib",
            "import tools as com\ncom.y = 1",
        ] {
            assert!(
                shown_error(&files(main)).starts_with("`com` is imported in this file"),
                "{main}"
            );
        }
    }

    #[test]
    fn merging_the_same_cell_twice_is_idempotent() {
        let lib = [("lib/x.gcsl", "com.x.lib.name = 1"), ("workspace.gcsl", "")];
        // `*` from one directory twice.
        let mut files = lib.to_vec();
        files.push((
            "app/main.gcsl",
            "import * from lib\nimport * from lib\na = com.x.lib.name\nall = com",
        ));
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{a = 1; all = {x = {lib = {name = 1}}}}"
        );
        assert!(ws(&files).unwrap().views.is_empty());
        // `*` from the file's own directory (`app` inside `app/`), from a
        // file of it, and from a directory above, with its own leaves.
        for main in [
            "import * from app\ncom.main.v = 1\nall = com",
            "import * from app.other\ncom.main.v = 1\nall = com",
            "import * from app.main\ncom.main.v = 1\nall = com",
            "import com from app\ncom.main.v = 1\nall = com",
        ] {
            let mut files = lib.to_vec();
            files.push(("app/main.gcsl", main));
            files.push(("app/other.gcsl", "com.other.v = 2"));
            assert_eq!(
                value(&files, "app/main.gcsl"),
                "{com = {main = {v = 1}}; all = {main = {v = 1}; other = {v = 2}}}",
                "{main}"
            );
            assert!(ws(&files).unwrap().views.is_empty(), "{main}");
        }
        let mut files = lib.to_vec();
        files.push((
            "app/sub/main.gcsl",
            "import * from app\ncom.sub = 1\nall = com",
        ));
        files.push(("app/other.gcsl", "com.other = 2"));
        assert_eq!(
            value(&files, "app/sub/main.gcsl"),
            "{com = {sub = 1}; all = {other = 2; sub = 1}}"
        );
    }

    /// A member of the file's own directory bound by another name is not
    /// the same cell at the same path: reaching the file's own nodes through
    /// it is still the one-file cycle, as through a whole import.
    #[test]
    fn a_renamed_member_reaching_the_files_own_nodes_needs_itself() {
        let files = [
            (
                "app/main.gcsl",
                "import com as c from app\ncom.x = 1\ny = c.x",
            ),
            ("workspace.gcsl", ""),
        ];
        assert!(shown_error(&files).starts_with(
            "`app/main.gcsl` needs itself at app/main.gcsl (`com.x` is computed when this \
             file runs)"
        ));
    }

    #[test]
    fn an_importing_files_items_merge_in_any_order() {
        let lines = [
            "import * from lib",
            "com.x.app.a = com.x.lib.name + b",
            "b = 2",
            "all = com.x",
        ];
        let reversed: Vec<&str> = lines.iter().rev().copied().collect();
        let orders = (0..lines.len())
            .map(|start| {
                let mut rotated = lines.to_vec();
                rotated.rotate_left(start);
                rotated
            })
            .chain([reversed]);
        for order in orders {
            let main = order.join("\n");
            let files = [
                ("app/main.gcsl", main.as_str()),
                ("lib/x.gcsl", "com.x.lib.name = 1"),
                ("workspace.gcsl", ""),
            ];
            let shown = value(&files, "app/main.gcsl");
            for field in [
                "com = {x = {app = {a = 3}}}",
                "b = 2",
                "all = {app = {a = 3}; lib = {name = 1}}",
            ] {
                assert!(shown.contains(field), "{main}: {shown}");
            }
        }
    }

    /// The time to check a workspace of `count` sibling directories of the
    /// root, each with one file of ten leaves under the root's `com`, each
    /// reading one leaf of the root's file through the merged `com`, and to
    /// run the last: linear in `count`. Run with `cargo test --release --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_sibling_directories_merging() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let mut sources = vec![
                SourceFile::new(
                    "workspace.gcsl",
                    (0..10).fold(String::new(), |mut text, leaf| {
                        writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                        text
                    }),
                )
                .unwrap(),
            ];
            for dir in 0..count {
                let mut text = format!("com.d{dir}.l0 = com.base.l3 + 1\n");
                for leaf in 1..10 {
                    writeln!(text, "com.d{dir}.l{leaf} = com.d{dir}.l{} + 1", leaf - 1).unwrap();
                }
                sources.push(SourceFile::new(format!("d{dir:04}/f.gcsl"), text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace
                .file(&format!("d{:04}/f.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains("l9 = 13"));
            writeln!(
                report,
                "{count} directories: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// [`benchmark_sibling_directories_merging`] with every file in one
    /// directory below the root: each file's view merges the root's `com`
    /// with the directory's, which has every file's part, so a file's view
    /// must not cost the size of its directory's tree. Linear in `count`.
    /// Run with `cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_one_directory_merging_with_the_root() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for count in [50_u32, 100, 200, 400, 800] {
            let mut sources = vec![
                SourceFile::new(
                    "workspace.gcsl",
                    (0..10).fold(String::new(), |mut text, leaf| {
                        writeln!(text, "com.base.l{leaf} = {leaf}").unwrap();
                        text
                    }),
                )
                .unwrap(),
            ];
            for file in 0..count {
                let mut text = format!("com.f{file}.l0 = com.base.l3 + 1\n");
                for leaf in 1..10 {
                    let before = leaf - 1;
                    writeln!(text, "com.f{file}.l{leaf} = com.f{file}.l{before} + 1").unwrap();
                }
                sources.push(SourceFile::new(format!("app/f{file:04}.gcsl"), text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace
                .file(&format!("app/f{:04}.gcsl", count - 1))
                .unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains("l9 = 13"));
            writeln!(
                report,
                "{count} files: scan {scanned:?}; check {checked:?}, check and run {total:?}"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    /// The cost of a file's view in a chain of `depth` directories, each
    /// below the one before and with one file of ten leaves under `com`,
    /// each reading a leaf of the directory above through the merged `com`:
    /// a file's view merges every directory above it, so the time per file
    /// grows with its depth. Run with `cargo test --release -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a benchmark, not a test"]
    fn benchmark_a_deep_chain_of_directories() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        let mut report = String::new();
        for depth in [10_u32, 25, 50, 100] {
            let mut path = String::new();
            let mut sources = Vec::new();
            for level in 0..depth {
                let mut text = if level == 0 {
                    String::from("com.d0.l0 = 0\n")
                } else {
                    format!("com.d{level}.l0 = com.d{}.l9 + 1\n", level - 1)
                };
                for leaf in 1..10 {
                    let before = leaf - 1;
                    writeln!(text, "com.d{level}.l{leaf} = com.d{level}.l{before} + 1").unwrap();
                }
                let name = if level == 0 {
                    String::from("workspace.gcsl")
                } else {
                    write!(path, "d{level}/").unwrap();
                    format!("{path}f.gcsl")
                };
                sources.push(SourceFile::new(name, text).unwrap());
            }
            let start = std::time::Instant::now();
            scan(&sources).unwrap();
            let scanned = start.elapsed();
            let start = std::time::Instant::now();
            let mut workspace = check(&sources).unwrap();
            let checked = start.elapsed();
            let last = workspace.file(&format!("{path}f.gcsl")).unwrap();
            let outcome = workspace.run(last).unwrap();
            let total = start.elapsed();
            assert!(format!("{}", outcome.value).contains(&format!("l9 = {}", depth * 10 - 1)));
            let per_file = total / depth;
            writeln!(
                report,
                "{depth} deep: scan {scanned:?}; check {checked:?}, check and run {total:?}, \
                 {per_file:?} a file"
            )
            .unwrap();
        }
        std::io::stdout().write_all(report.as_bytes()).unwrap();
    }

    // ----- The standard library -----

    /// The summary of slice 28: the prelude's names, `builtins` and the
    /// prelude imported by name.
    const STANDARD_SUMMARY: &str = "xs = [1, 2, 3]\n\
                                    n = len(xs)\n\
                                    h = half(n)\n\
                                    import builtins\n\
                                    same = builtins.len(xs) == n\n\
                                    import len as count from prelude\n\
                                    c = count(xs)";

    /// The value of `text` run as a lone file, beside the standard library,
    /// or the messages of its diagnostics.
    fn lone(text: &str) -> Result<String, Vec<String>> {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        crate::run(&source)
            .map(|outcome| outcome.value.to_string())
            .map_err(|errors| errors.into_iter().map(|e| e.message).collect())
    }

    #[test]
    fn the_standard_summary_runs_in_a_workspace_and_alone() {
        let expected = "{xs = [1, 2, 3]; n = 3; h = 1.5; same = true; c = 3}";
        let files = [("workspace.gcsl", ""), ("app/main.gcsl", STANDARD_SUMMARY)];
        assert_eq!(value(&files, "app/main.gcsl"), expected);
        assert_eq!(lone(STANDARD_SUMMARY).unwrap(), expected);
        // The standard files are the first files, by input number, in a
        // directory of their own.
        let scan = ws(&files).unwrap();
        assert_eq!(scan.standard(), 1);
        assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
        assert_eq!(scan.dirs[scan.files[0].dir as usize].parent, None);
        assert_eq!(scan.order[0], 0);
        // The built-in functions' cells are the first.
        let builtins: Vec<Option<Builtin>> = (0..24).map(|node| scan.builtin(node)).collect();
        assert_eq!(
            builtins,
            [
                Some(Builtin::Len),
                Some(Builtin::Int),
                Some(Builtin::Float),
                Some(Builtin::Exec),
                Some(Builtin::Panic),
                Some(Builtin::Split),
                Some(Builtin::Join),
                Some(Builtin::StartsWith),
                Some(Builtin::EndsWith),
                Some(Builtin::Trim),
                Some(Builtin::StripPrefix),
                Some(Builtin::StripSuffix),
                Some(Builtin::Lines),
                Some(Builtin::Fields),
                Some(Builtin::CompareVersions),
                Some(Builtin::FsExists),
                Some(Builtin::FsRead),
                Some(Builtin::FsMkdir),
                Some(Builtin::FsCopy),
                Some(Builtin::FsTempDir),
                Some(Builtin::FsWrite),
                Some(Builtin::SortVersions),
                Some(Builtin::Show),
                None
            ]
        );
    }

    #[test]
    fn len_is_reached_by_every_road() {
        for text in [
            "n = len([1, 2])",
            "import len from builtins\nn = len([1, 2])",
            "import builtins\nn = builtins.len([1, 2])",
            "import * from builtins\nn = len([1, 2])",
            "import prelude\nn = prelude.len([1, 2])",
            "let l = len\nn = l([1, 2])",
            "let l = len\nlet m = l\nn = m([1, 2])",
            "l = len\nn = l([1, 2])",
            "import len as count from prelude\nlet l = count\nn = l([1, 2])",
            "import builtins\nl = builtins.len\nn = l([1, 2])",
        ] {
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            let shown = value(&files, "main.gcsl");
            assert!(shown.contains("n = 2"), "{text}: {shown}");
            assert!(lone(text).unwrap().contains("n = 2"), "{text}");
        }
        // An alias of `len` is of its opaque type; the callee of a call of
        // it has the function type of the call.
        let mut workspace =
            checked(&[("workspace.gcsl", ""), ("main.gcsl", "l = len\nn = l([1])")]);
        let index = workspace.file("main.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(dump.contains("(name len : builtin len @4..7)"), "{dump}");
        assert!(
            dump.contains("(name l : fn(List<Int>): Int @12..13)"),
            "{dump}"
        );
    }

    #[test]
    fn an_alias_of_len_crosses_files_directories_and_imports() {
        // A field of the root's, a member import's alias of an alias in
        // another directory, and a `let` of that in a third file.
        let files = [
            ("workspace.gcsl", "measure = len"),
            (
                "lib/a.gcsl",
                "import len as count from prelude\nsize = count",
            ),
            (
                "app/main.gcsl",
                "import size from lib\nlet l = size\nn = l([1, 2]) + measure([1])\n\
                 def twice(v: Int[]): Int { l(v) + size(v) }",
            ),
            ("app/other.gcsl", "m = measure([1, 2, 3]) + twice([1])"),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{n = 3; twice = fn(v: List<Int>): Int}"
        );
        assert_eq!(value(&files, "app/other.gcsl"), "{m = 5}");
        assert_eq!(value(&files, "lib/a.gcsl"), "{size = builtin len}");
        // Read whole, a record holding one is a use of it as a value.
        let mut whole = files.to_vec();
        whole.push(("app/whole.gcsl", "import lib\nall = lib"));
        assert_eq!(
            messages(&whole, "app/whole.gcsl"),
            ["`len` is a built-in function and cannot be used as a value yet"]
        );
    }

    #[test]
    fn int_and_float_are_values_of_their_function_types() {
        for (text, expected) in [
            ("= int(2.7)", "2"),
            ("= int(-2.7)", "-2"),
            ("= int(x = 9.99)", "9"),
            ("= float(3)", "3.0"),
            ("= float(-3)", "-3.0"),
            ("= int", "fn(x: Float): Int"),
            ("import builtins\n= builtins.float", "fn(x: Int): Float"),
            ("import int from builtins\n= int(1.5)", "1"),
            // Passed, stored and called through a variable.
            ("let i = int\n= i(-0.5)", "0"),
            (
                "fn apply(f: fn(x: Float): Int, v: Float): Int { f(v) }\n= apply(int, 7.5)",
                "7",
            ),
            ("let fs = [float, float]\n= fs[1](2)", "2.0"),
            ("let r = { f = float }\n= r.f(4)", "4.0"),
            ("let mut f = int\nf := int\n= f(3.5)", "3"),
            ("= half(3)", "1.5"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // Truncation toward zero fails when the result is no `Int`.
        for (text, message) in [
            ("= int(1e300)", "integer overflow"),
            ("= int(-1e300)", "integer overflow"),
            ("= int(1.0 / 0.0)", "integer overflow"),
            ("= int(0.0 / 0.0)", "cannot truncate NaN to an Int"),
            ("let i = int\n= i(1e300)", "integer overflow"),
        ] {
            assert_eq!(lone(text).unwrap_err(), [message], "{text}");
        }
        assert_eq!(
            lone("= int(9223372036854774784.0)").unwrap(),
            "9223372036854774784"
        );
        assert_eq!(
            lone("= int(-9223372036854775808.0)").unwrap(),
            "-9223372036854775808"
        );
        // Functions have no equality, built-in ones included.
        assert_eq!(
            lone("= int === int").unwrap_err(),
            ["cannot compare functions"]
        );
    }

    #[test]
    fn a_file_s_own_names_shadow_the_prelude_s_silently() {
        // A field, a `def`, a `let` and a `fn`: no clash, and the built-in
        // is gone there.
        for (text, expected) in [
            ("len = 1\nn = len + 1", "{len = 1; n = 2}"),
            (
                "def len(v: Int[]): Int { v[0] + 41 }\nn = len([1])",
                "{len = fn(v: List<Int>): Int; n = 42}",
            ),
            ("let len = 5\nn = len", "{n = 5}"),
            ("fn len(): Int { 7 }\nn = len()", "{n = 7}"),
            ("let int = 2\nn = int * 3", "{n = 6}"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // A `let` shadows from its item on, as it always has.
        assert_eq!(
            lone("n = len([1])\nlet len = 5\nm = len").unwrap(),
            "{n = 1; m = 5}"
        );
        // A field of the directory, or of a directory above, shadows it in
        // every file of the directory, and an explicit import does too.
        let files = [
            ("workspace.gcsl", "len = 3"),
            ("sub/a.gcsl", "n = len + 1"),
            ("sub/b.gcsl", "import int from builtins\nm = len + int(1.5)"),
        ];
        assert_eq!(value(&files, "sub/a.gcsl"), "{n = 4}");
        assert_eq!(value(&files, "sub/b.gcsl"), "{m = 4}");
        // Nothing of the implicit import is an alias the scan knows of.
        let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "len = 1")]).unwrap();
        let names: Vec<&str> = scan.files[2]
            .aliases
            .iter()
            .map(|alias| scan.names.get(alias.name))
            .collect();
        assert_eq!(
            names,
            [
                "show",
                "int",
                "float",
                "half",
                "exec",
                "panic",
                "split",
                "join",
                "starts_with",
                "ends_with",
                "trim",
                "strip_prefix",
                "strip_suffix",
                "lines",
                "fields",
                "compare_versions",
                "sort_versions",
                "sh"
            ]
        );
        assert!(scan.files[2].aliases.iter().all(|alias| alias.implicit));
        assert!(scan.files[0].aliases.iter().all(|alias| !alias.implicit));
    }

    #[test]
    fn an_explicit_star_from_the_prelude_conflicts_as_any() {
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import * from prelude\nlen = 3"),
        ];
        assert_eq!(
            shown_error(&files),
            "field `len` is defined in two files at main.gcsl (imported here); \
             <stdlib>/prelude.gcsl: defined here; main.gcsl: defined here"
        );
        // The same for the `builtins` module, whose side has no definition.
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import * from builtins\nlen = 3"),
        ];
        assert_eq!(
            shown_error(&files),
            "field `len` is defined in two files at main.gcsl (imported here); \
             main.gcsl: defined here; note: `len` is a built-in function"
        );
        // An explicit import of the prelude's name and the implicit one are
        // one cell: no conflict.
        assert_eq!(
            lone("import * from prelude\nn = len([1, 2])").unwrap(),
            "{n = 2}"
        );
        // A `let` of an imported name is the clash it always was.
        assert_eq!(
            lone("import len from prelude\nlet len = 1").unwrap_err(),
            ["`len` is imported in this file"]
        );
    }

    #[test]
    fn the_standard_modules_names_are_reserved() {
        for (files, message, input) in [
            (
                &[("workspace.gcsl", ""), ("prelude.gcsl", "x = 1")][..],
                "`prelude` is the name of a standard module",
                Some(1),
            ),
            (
                &[("builtins/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
                "`builtins` is the name of a built-in module",
                Some(1),
            ),
            (
                &[("builtins.gcsl", ""), ("workspace.gcsl", "")][..],
                "`builtins` is the name of a built-in module",
                Some(1),
            ),
        ] {
            assert_eq!(
                error(files),
                (message.to_owned(), input, vec![]),
                "{files:?}"
            );
        }
        // Below the root, a name of a path of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/prelude.gcsl", "x = 1"),
            ("main.gcsl", "import lib.prelude\nn = prelude.x + len([1])"),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{n = 2}");
        // A path of more than one segment never names a standard module.
        let files = [("workspace.gcsl", ""), ("main.gcsl", "import prelude.len")];
        assert_eq!(error(&files).0, "no module `prelude.len`");
    }

    #[test]
    fn a_lone_file_imports_only_the_standard_library() {
        assert_eq!(
            lone("import nothing").unwrap_err(),
            [
                "`nothing` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
            ]
        );
        assert_eq!(
            lone("x = 1 + true\nimport a from c.d").unwrap_err(),
            [
                "cannot add Int and Bool",
                "`c.d` is not a built-in or standard module, and there is no \
                 `workspace.gcsl` above this file"
            ]
        );
        assert_eq!(lone("import builtins\n= builtins.len([1])").unwrap(), "1");
        assert_eq!(lone("import prelude\n= prelude.half(1)").unwrap(), "0.5");
        // Its diagnostics name no input; a standard file's names its own.
        let source = SourceFile::new("lone.gcsl", "x = 1 + true").unwrap();
        assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
        let source = SourceFile::new("lone.gcsl", "import * from prelude\nlen = 1").unwrap();
        let error = &crate::run(&source).unwrap_err()[0];
        assert_eq!(error.input, None);
        let inputs: Vec<Option<u32>> = error.labels.iter().map(|l| l.input).collect();
        assert_eq!(inputs, [Some(0), None]);
    }

    #[test]
    fn the_table_of_standard_files_is_the_stdlib_directory() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stdlib");
        let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "gcsl"))
            .map(|path| {
                let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
                (stem, std::fs::read_to_string(&path).unwrap())
            })
            .collect();
        found.sort();
        let table: Vec<(String, String)> = crate::STANDARD
            .iter()
            .map(|&(stem, text)| (stem.to_owned(), text.to_owned()))
            .collect();
        assert_eq!(table, found);
        // Each is a standard file, in path order, named under `<stdlib>/`.
        let names: Vec<String> = standard_sources()
            .iter()
            .map(|source| source.name().to_owned())
            .collect();
        let expected: Vec<String> = found
            .iter()
            .map(|(stem, _)| format!("<stdlib>/{stem}.gcsl"))
            .collect();
        assert_eq!(names, expected);
    }

    // ----- `exec`, `panic` and `sh` -----

    /// The summary of slice 29, its `ls` of a directory every machine has.
    const SHELL_SUMMARY: &str = "r = exec([\"ls\", \"-1d\", \"/\"])\n\
                                 lines = r.stdout\n\
                                 ok = r.status == 0\n\
                                 out = sh(\"echo hi\")\n\
                                 fn pkg(name: String): { build: Future<String> } {\n\
                                     { build = lazy { sh(\"echo make {name}\") } }\n\
                                 }\n\
                                 built = await pkg(\"lib\").build\n\
                                 fn guard(n: Int): Int {\n\
                                     if n < 0 { panic(\"negative size {n}\") }\n\
                                     n\n\
                                 }\n\
                                 size = guard(3)";

    /// The one diagnostic of running `text` as a lone file.
    fn lone_error(text: &str) -> Diagnostic {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let mut errors = crate::run(&source).unwrap_err();
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        errors.remove(0)
    }

    #[test]
    fn the_shell_summary_runs_in_a_workspace_and_alone() {
        let expected = "{r = {status = 0; stdout = \"/\\n\"; stderr = \"\"}; lines = \"/\\n\"; \
                        ok = true; out = \"hi\\n\"; built = \"make lib\\n\"; size = 3}";
        assert_eq!(lone(SHELL_SUMMARY).unwrap(), expected);
        let files = [("workspace.gcsl", ""), ("app/main.gcsl", SHELL_SUMMARY)];
        assert_eq!(value(&files, "app/main.gcsl"), expected);
        assert_eq!(
            lone_error("fn guard(n: Int): Int {\nif n < 0 { panic(\"negative size {n}\") }\nn\n}\n= guard(-2)")
                .message,
            "negative size -2"
        );
    }

    #[test]
    fn exec_reports_what_its_program_did() {
        for (text, expected) in [
            (
                "= exec([\"true\"])",
                "{status = 0; stdout = \"\"; stderr = \"\"}",
            ),
            (
                "= exec([\"false\"])",
                "{status = 1; stdout = \"\"; stderr = \"\"}",
            ),
            (
                "= exec([\"sh\", \"-c\", \"echo out; echo err >&2; exit 3\"])",
                "{status = 3; stdout = \"out\\n\"; stderr = \"err\\n\"}",
            ),
            // A non-zero status is data, never an error.
            ("= exec([\"false\"]).status != 0", "true"),
            // Killed by a signal: 128 plus its number.
            ("= exec([\"sh\", \"-c\", \"kill -9 $$\"]).status", "137"),
            // Its standard input is the null device: `cat` ends at once.
            (
                "= exec([\"cat\"])",
                "{status = 0; stdout = \"\"; stderr = \"\"}",
            ),
            // Invalid UTF-8 is replaced.
            (
                "= exec([\"sh\", \"-c\", \"printf '\\\\377ok'\"]).stdout",
                "\u{FFFD}ok",
            ),
            // The arguments are passed as they are, with no shell.
            (
                "= exec([\"printf\", \"%s|\", \"a b\", \"$HOME\", \"*\"]).stdout",
                "a b|$HOME|*|",
            ),
            // Named, through a variable, a field, a `->` function.
            ("= exec(argv = [\"echo\", \"n\"]).stdout", "n\n"),
            ("let e = exec\n= e([\"echo\", \"v\"]).stdout", "v\n"),
            (
                "let r = { run = exec }\n= r.run([\"echo\", \"f\"]).stdout",
                "f\n",
            ),
            (
                "fn run(a: String) -> String { exec([\"echo\", a]).stdout }\n= run(\"i\")",
                "i\n",
            ),
            (
                "fn pick(f: fn(argv: Present<List<String>>) -> {status: Int}): \
                 fn(Present<List<String>>) -> {status: Int} { f }\n\
                 = pick(exec)([\"false\"]).status",
                "1",
            ),
            (
                "import exec as run from builtins\n= run([\"true\"]).status",
                "0",
            ),
            // A future argument is awaited, as any call's.
            (
                "let xs = lazy [\"echo\", \"later\"]\n= exec(xs).stdout",
                "later\n",
            ),
            (
                "let xs = [lazy \"echo\", \"deep\"]\n= exec(xs).stdout",
                "deep\n",
            ),
            // The value shows as its type.
            (
                "= exec",
                "fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}",
            ),
            ("= panic", "fn(message: String): Never"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }

    #[test]
    fn exec_fails_only_when_its_program_cannot_run() {
        let text = "let none: List<String> = []\nr = exec(none)";
        let error = lone_error(text);
        assert_eq!(error.message, "`exec` needs a program to run");
        assert_eq!(error.span, Span::new(32, 42));
        assert!(error.notes.is_empty());
        let text = "r = exec([\"gcsl-no-such-program-slice-29\", \"x\"])";
        let error = lone_error(text);
        assert_eq!(error.message, "cannot run `gcsl-no-such-program-slice-29`");
        assert_eq!(error.span, Span::new(4, 48));
        assert_eq!(error.notes, ["No such file or directory (os error 2)"]);
        assert_eq!(error.input, None);
        // A directory is not a program.
        let error = lone_error("r = exec([\"/\"])");
        assert_eq!(error.message, "cannot run `/`");
        assert_eq!(error.notes, ["Permission denied (os error 13)"]);
    }

    #[test]
    fn panic_ends_the_run_with_its_message_at_the_call() {
        let text = "x = 1\ny = panic(\"x\")";
        let error = lone_error(text);
        assert_eq!(error.message, "x");
        assert_eq!(error.span, Span::new(10, 20));
        assert!(error.labels.is_empty() && error.notes.is_empty());
        assert_eq!(lone_error("panic(\"one\\ntwo\")").message, "one\ntwo");
        // Wherever it stands, whichever walker meets it: a statement, a
        // `let`, a branch, an operand of each kind, an element, a field, an
        // argument, a callee, an interpolation, a function's result, a
        // `lazy`, a loop.
        for text in [
            "panic(\"p\")",
            "let x = panic(\"p\")",
            "let c = true\nx = if c { panic(\"p\") } else { 1 }",
            "x = 1 + panic(\"p\")",
            "x = 1.5 * panic(\"p\")",
            "x = true and panic(\"p\")",
            "x = \"a\" + panic(\"p\")",
            "x = \"a{panic(\"p\")}\"",
            "x = [1, panic(\"p\")]",
            "x = len(panic(\"p\"))",
            "x = { a = panic(\"p\") }",
            "let r: {a: Int} = panic(\"p\")",
            "x = float(panic(\"p\"))",
            "x = panic(\"p\")(1)",
            "fn f(n: Int): Int { if n > 0 { panic(\"p\") }\nn }\nx = f(1)",
            "fn f(): String { panic(\"p\") }\nx = f() + \"!\"",
            "x = await lazy panic(\"p\")",
            "x = for i in [1] { panic(\"p\") }",
            "def f(n: Int) ->> Int { panic(\"p\") }\nx = f(1)",
            // As a value of a parameter of type `fn(String): Int`.
            "fn apply(f: fn(String): Int): Int { f(\"p\") }\nx = apply(panic)",
            "let p = panic\nx = p(\"p\")",
        ] {
            assert_eq!(lone_error(text).message, "p", "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(messages(&files, "main.gcsl"), ["p"], "{text}");
        }
        // What follows a panic never runs.
        assert_eq!(
            lone_error("let c = true\nif c { panic(\"first\") }\n= 1 // 0").message,
            "first"
        );
        // A message of the program's own wording, built at runtime.
        assert_eq!(
            lone_error("let n = 3\n= panic(\"{n} is {n * 2 // 2}\")").message,
            "3 is 3"
        );
    }

    #[test]
    fn sh_returns_its_output_or_panics_in_the_prelude() {
        assert_eq!(lone("= sh(\"echo hi\")").unwrap(), "hi\n");
        assert_eq!(lone("= sh(\"printf hi\")").unwrap(), "hi");
        // The panic is the prelude's, located in it: input 0.
        let error = lone_error("x = sh(\"echo nope >&2; exit 2\")");
        assert_eq!(
            error.message,
            "`echo nope >&2; exit 2` exited with status 2: nope"
        );
        assert_eq!(error.input, Some(0));
        let prelude = Library::NEEDED.find("prelude").unwrap();
        let prelude = crate::STANDARD[prelude].1;
        let at = u32::try_from(prelude.find("panic(\"`").unwrap()).unwrap();
        assert_eq!(error.span.start, at);
        // With no standard error, the message ends in ": ".
        assert_eq!(
            lone_error("x = sh(\"exit 2\")").message,
            "`exit 2` exited with status 2: "
        );
        let files = [("workspace.gcsl", ""), ("main.gcsl", "x = sh(\"exit 2\")")];
        assert_eq!(
            run_file(&files, "main.gcsl").unwrap_err(),
            [("`exit 2` exited with status 2: ".to_owned(), Some(0))]
        );
    }

    #[test]
    fn a_memoised_function_around_exec_runs_its_program_once() {
        let path = std::env::temp_dir().join(format!("gcsl-slice-29-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
        let _ = std::fs::remove_file(path);
        let memo = format!(
            "fn mark(p: String) ->> Int {{ exec([\"sh\", \"-c\", \"echo x >> \" + p]).status }}\n\
             = [mark(\"{path}\"), mark(\"{path}\")]"
        );
        assert_eq!(lone(&memo).unwrap(), "[0, 0]");
        assert_eq!(count(), 1);
        // An impure function that is not memoised runs it each time.
        let impure = memo.replace("->>", "->");
        assert_eq!(lone(&impure).unwrap(), "[0, 0]");
        assert_eq!(count(), 3);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn exec_panic_and_sh_are_shadowed_silently() {
        for (text, expected) in [
            ("exec = 1\nn = exec + 1", "{exec = 1; n = 2}"),
            ("let panic = \"no\"\nn = panic", "{n = \"no\"}"),
            (
                "def sh(n: Int): Int { n * 2 }\nn = sh(2)",
                "{sh = fn(n: Int): Int; n = 4}",
            ),
            (
                "fn panic(m: String): Int { 7 }\nn = panic(\"x\")",
                "{n = 7}",
            ),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }

    #[test]
    fn every_record_exec_builds_shares_the_runs_one_layout() {
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "ra = exec([\"true\"])\n\
                 def run() -> {status: Int, stdout: String, stderr: String} { exec([\"true\"]) }",
            ),
            (
                "b.gcsl",
                "rb = exec([\"false\"])\nlet e = exec\ns = e([\"true\"])\nt = run()",
            ),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        // The driver wrote the twenty-three built-in functions' cells.
        assert_eq!(assert_builtins_written(&workspace).len(), 23);
        let a = workspace.file("a.gcsl").unwrap();
        let b = workspace.file("b.gcsl").unwrap();
        let first = workspace.run(a).unwrap().value;
        let second = workspace.run(b).unwrap().value;
        let layout = |value: &Value, field: &str| {
            let Value::Record(record) = value else {
                panic!("not a record: {value}");
            };
            let name = workspace.scan.names.lookup(field).unwrap();
            let Some(Value::Record(result)) = record.get(name) else {
                panic!("no record {field} in {value}");
            };
            Rc::clone(&result.layout)
        };
        for (value, field) in [
            (&first, "ra"),
            (&second, "rb"),
            (&second, "s"),
            (&second, "t"),
        ] {
            assert!(
                Rc::ptr_eq(&layout(value, field), &workspace.exec_result),
                "{field} of {value}"
            );
        }
        assert_eq!(
            &*workspace.exec_result.texts,
            ["status", "stdout", "stderr"].map(Box::from)
        );
    }

    // ----- Strings, `path`, and standard modules as scopes of their own -----

    /// The summary of slice 30, its `sh` a command every machine has.
    const STRINGS_SUMMARY: &str = "parts = split(\"a/b/c\", \"/\")\n\
                                   s = join(parts, \"-\")\n\
                                   yes = starts_with(\"/usr\", \"/\")\n\
                                   gz = ends_with(\"x.tar.gz\", \".gz\")\n\
                                   hash = trim(sh(\"echo ' abc '\"))\n\
                                   import path\n\
                                   p = path.join(\"/src/pkg\", \"../lib\")\n\
                                   q = path.join(\"/src/pkg\", \"/etc\")\n\
                                   d = path.dirname(\"/src/pkg/a.c\")\n\
                                   b = path.basename(\"/src/pkg/a.c\")\n\
                                   n = path.normalize(\"a/./b/../c/\")\n\
                                   abs = path.is_absolute(\"a\")";

    /// The prelude's entry of the table of standard files, for a test's
    /// own table.
    fn prelude_entry() -> (&'static str, &'static str) {
        crate::STANDARD[Library::NEEDED.find("prelude").unwrap()]
    }

    /// The workspace of `files` checked beside the standard library of
    /// `table`, which it loads as a workspace does, with the names of the
    /// standard files it loaded.
    fn checked_beside(
        table: &[(&str, &str)],
        files: &[(&str, &str)],
    ) -> (Vec<String>, Result<Workspace, Vec<Diagnostic>>) {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let library = Library {
            table,
            every: false,
        };
        let (standard, checked) = check_in((&sources, Placing::Workspace(&[])), library, "/");
        let names = standard.iter().map(|s| s.name().to_owned()).collect();
        (names, checked)
    }

    /// The value of the file at `path` of the workspace of `files`, checked
    /// beside the standard library of `table`, with the names of the
    /// standard files it loaded.
    fn value_beside(
        table: &[(&str, &str)],
        files: &[(&str, &str)],
        path: &str,
    ) -> (Vec<String>, String) {
        let (standard, checked) = checked_beside(table, files);
        let mut workspace = checked.unwrap();
        let index = workspace.file(path).unwrap();
        (standard, workspace.run(index).unwrap().value.to_string())
    }

    #[test]
    fn the_strings_summary_runs_in_a_workspace_and_alone() {
        let expected = "{parts = [\"a\", \"b\", \"c\"]; s = \"a-b-c\"; yes = true; gz = true; \
                        hash = \"abc\"; p = \"/src/lib\"; q = \"/etc\"; d = \"/src/pkg\"; \
                        b = \"a.c\"; n = \"a/c\"; abs = false}";
        assert_eq!(lone(STRINGS_SUMMARY).unwrap(), expected);
        let files = [("workspace.gcsl", ""), ("app/main.gcsl", STRINGS_SUMMARY)];
        assert_eq!(value(&files, "app/main.gcsl"), expected);
        // `path` is loaded, and takes the first input number, before the
        // prelude, in path order; each is a directory of its own.
        let scan = ws(&files).unwrap();
        assert_eq!(scan.standard(), 2);
        assert_eq!(scan.files[0].path, "<stdlib>/path.gcsl");
        assert_eq!(scan.files[1].path, "<stdlib>/prelude.gcsl");
        assert_ne!(scan.files[0].dir, scan.files[1].dir);
        for file in &scan.files[..2] {
            assert_eq!(scan.dirs[file.dir as usize].parent, None);
        }
        // It needs the prelude, whose `split` it calls: the prelude runs
        // first.
        assert_eq!(scan.needs[0], [1]);
        assert_eq!(&scan.order[..2], [1, 0]);
    }

    #[test]
    fn split_gives_the_pieces_between_its_separator() {
        for (text, expected) in [
            ("= split(\"a/b/c\", \"/\")", "[\"a\", \"b\", \"c\"]"),
            ("= split(\"/a/b\", \"/\")", "[\"\", \"a\", \"b\"]"),
            ("= split(\"\", \"/\")", "[\"\"]"),
            ("= split(\"a//b\", \"/\")", "[\"a\", \"\", \"b\"]"),
            ("= split(\"a/\", \"/\")", "[\"a\", \"\"]"),
            ("= split(\"/\", \"/\")", "[\"\", \"\"]"),
            ("= split(\"abc\", \"/\")", "[\"abc\"]"),
            // A separator longer than one byte, and longer than the text.
            ("= split(\"a::b::\", \"::\")", "[\"a\", \"b\", \"\"]"),
            ("= split(\"a:::b\", \"::\")", "[\"a\", \":b\"]"),
            ("= split(\"ab\", \"abc\")", "[\"ab\"]"),
            // Text of several bytes a character, either side.
            (
                "= split(\"h\u{e9}llo w\u{f6}rld\", \"\u{f6}\")",
                "[\"h\u{e9}llo w\", \"rld\"]",
            ),
            (
                "= split(\"\u{65e5}\u{672c}\u{8a9e}\", \"\u{672c}\")",
                "[\"\u{65e5}\", \"\u{8a9e}\"]",
            ),
            ("= split(\"a\u{2192}b\", \"\u{2192}\")", "[\"a\", \"b\"]"),
            // Through a variable, by name.
            (
                "let f = split\n= f(sep = \",\", s = \"1,2\")",
                "[\"1\", \"2\"]",
            ),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // An empty separator is a runtime error at the call.
        let text = "x = 1\ny = split(\"abc\", \"\")";
        let error = lone_error(text);
        assert_eq!(error.message, "`split` needs a separator that is not empty");
        assert_eq!(error.span, Span::new(10, 26));
        assert_eq!(error.input, None);
        let error = lone_error("let sep = \"\"\nlet f = split\nx = f(\"a\", sep)");
        assert_eq!(error.message, "`split` needs a separator that is not empty");
    }

    #[test]
    fn join_starts_with_ends_with_and_trim_work_on_strings() {
        for (text, expected) in [
            ("= join([\"a\", \"b\", \"c\"], \"-\")", "a-b-c"),
            ("let none: List<String> = []\n= join(none, \",\")", ""),
            ("= join([], \",\")", ""),
            ("= join([\"a\"], \",\")", "a"),
            ("= join([\"\", \"\"], \"/\")", "/"),
            ("= join([\"a\", \"b\"], \"\")", "ab"),
            ("= join([\"x\", \"y\"], \" \u{2192} \")", "x \u{2192} y"),
            ("= join(split(\"/a//b/\", \"/\"), \"/\")", "/a//b/"),
            ("= starts_with(\"/usr\", \"/\")", "true"),
            ("= starts_with(\"usr\", \"/\")", "false"),
            ("= starts_with(\"\", \"\")", "true"),
            ("= starts_with(\"a\", \"\")", "true"),
            ("= starts_with(\"\", \"a\")", "false"),
            ("= starts_with(\"/\", \"/usr\")", "false"),
            ("= ends_with(\"x.tar.gz\", \".gz\")", "true"),
            ("= ends_with(\"gz\", \".gz\")", "false"),
            ("= ends_with(\"a\", \"\")", "true"),
            // Byte-wise: a precomposed `é` is not an `e` with a combining
            // accent, though they look alike.
            ("= starts_with(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "true"),
            ("= starts_with(\"e\\u{301}t\u{e9}\", \"\u{e9}\")", "false"),
            ("= ends_with(\"caf\u{e9}\", \"\u{e9}\")", "true"),
            ("= ends_with(\"cafe\\u{301}\", \"\u{e9}\")", "false"),
            ("= trim(\"  a b \\n\\t\")", "a b"),
            ("= trim(\"x\")", "x"),
            ("= trim(\"\")", ""),
            // Whitespace alone is nothing; Unicode's whitespace is trimmed
            // too, a no-break space and an ideographic space among it.
            ("= trim(\" \\n\\t\\r \")", ""),
            ("= trim(\"\\u{3000}x\\u{a0}\")", "x"),
            ("= trim(\"\\u{3000}\")", ""),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // Their values show as their types.
        for (text, expected) in [
            ("= split", "fn(s: String, sep: String): List<String>"),
            ("= join", "fn(parts: List<String>, sep: String): String"),
            ("= starts_with", "fn(s: String, prefix: String): Bool"),
            ("= ends_with", "fn(s: String, suffix: String): Bool"),
            ("= trim", "fn(s: String): String"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn path_handles_paths_lexically() {
        for (call, expected) in [
            // `normalize`, on the spec's examples and the edges.
            ("normalize(\"a/./b/../c/\")", "a/c"),
            ("normalize(\"\")", "."),
            ("normalize(\".\")", "."),
            ("normalize(\"./\")", "."),
            ("normalize(\"/\")", "/"),
            ("normalize(\"//\")", "/"),
            ("normalize(\"/../a\")", "/a"),
            ("normalize(\"../../a/..\")", "../.."),
            ("normalize(\"a/b/\")", "a/b"),
            ("normalize(\"a/b//\")", "a/b"),
            ("normalize(\"/a/b/\")", "/a/b"),
            ("normalize(\"a/../..\")", ".."),
            ("normalize(\"/a/../..\")", "/"),
            ("normalize(\"..\")", ".."),
            ("normalize(\"a//b\")", "a/b"),
            // `join`.
            ("join(\"/src/pkg\", \"../lib\")", "/src/lib"),
            ("join(\"/src/pkg\", \"/etc\")", "/etc"),
            ("join(\"/x\", \"\")", "/x"),
            ("join(\"\", \"a/b/\")", "a/b"),
            ("join(\"\", \"\")", "."),
            ("join(\"a\", \"b\")", "a/b"),
            ("join(\"a/\", \"../..\")", ".."),
            ("join(\"/\", \"..\")", "/"),
            ("join(\".\", \".\")", "."),
            // `dirname`.
            ("dirname(\"/src/pkg/a.c\")", "/src/pkg"),
            ("dirname(\"a\")", "."),
            ("dirname(\"/a\")", "/"),
            ("dirname(\"/\")", "/"),
            ("dirname(\"..\")", "."),
            ("dirname(\".\")", "."),
            ("dirname(\"\")", "."),
            ("dirname(\"//\")", "/"),
            ("dirname(\"a/../..\")", "."),
            ("dirname(\"/a/../..\")", "/"),
            ("dirname(\"a/b/\")", "a"),
            ("dirname(\"../../a\")", "../.."),
            // `basename`.
            ("basename(\"/src/pkg/a.c\")", "a.c"),
            ("basename(\"/\")", "/"),
            ("basename(\".\")", "."),
            ("basename(\"\")", "."),
            ("basename(\"//\")", "/"),
            ("basename(\"a/b/\")", "b"),
            ("basename(\"a/../..\")", ".."),
            ("basename(\"/a/../..\")", "/"),
            ("basename(\"a\")", "a"),
            // `is_absolute`.
            ("is_absolute(\"a\")", "false"),
            ("is_absolute(\"/\")", "true"),
            ("is_absolute(\"\")", "false"),
            ("is_absolute(\"//x\")", "true"),
        ] {
            let text = format!("import path\n= path.{call}");
            assert_eq!(lone(&text).unwrap(), expected, "{call}");
        }
        // In a workspace, as a member.
        let files = [
            ("workspace.gcsl", ""),
            (
                "main.gcsl",
                "import dirname from path\nd = dirname(\"a/b/c\")",
            ),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{d = \"a/b\"}");
    }

    #[test]
    fn path_is_not_loaded_when_nobody_imports_it() {
        let files = [
            ("workspace.gcsl", ""),
            ("lib/a.gcsl", "x = [1]\ny = len(x)"),
            ("main.gcsl", "import lib.a\nz = a.y + 1"),
        ];
        let (standard, checked) = checked_beside(&crate::STANDARD, &files);
        assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
        let mut workspace = checked.unwrap();
        assert!(
            workspace
                .scan
                .files
                .iter()
                .all(|f| f.path != "<stdlib>/path.gcsl")
        );
        // Its input numbers and slots are those of a run with no `path` at
        // all, beside the prelude alone.
        let (alone, bare) = checked_beside(&[prelude_entry()], &files);
        assert_eq!(alone, standard);
        let mut bare = bare.unwrap();
        let paths = |w: &Workspace| -> Vec<String> {
            w.scan.files.iter().map(|f| f.path.clone()).collect()
        };
        assert_eq!(paths(&workspace), paths(&bare));
        assert_eq!(workspace.scan.cells(), bare.scan.cells());
        assert_eq!(workspace.slot_names, bare.slot_names);
        let main = workspace.file("main.gcsl").unwrap();
        assert_eq!(main, 2);
        assert_eq!(workspace.dump_ir(main), bare.dump_ir(main));
        assert_eq!(
            workspace.run(main).unwrap().value,
            bare.run(main).unwrap().value
        );
        assert_eq!(workspace.frame.len(), bare.frame.len());
        // A lone file too: the prelude is input 0, the file the next.
        let source = SourceFile::new("lone.gcsl", "n = len([1])").unwrap();
        let (standard, checked) = check_lone_with_standard(&source, "/");
        assert_eq!(standard.len(), 1);
        assert_eq!(checked.unwrap().file("lone.gcsl"), Some(1));
    }

    #[test]
    fn path_is_loaded_by_any_form_of_import() {
        for text in [
            "import path\nv = path.join(\"a\", \"b\")",
            "import path as p\nv = p.join(\"a\", \"b\")",
            "import join from path\nv = join(\"a\", \"b\")",
            "import join as pj from path\nv = pj(\"a\", \"b\")",
            "import normalize, join from path\nv = join(\"a\", \"b\")",
            // `*` binds its `join`, to which the prelude's yields.
            "import * from path\nv = join(\"a\", \"b\")",
        ] {
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
            assert_eq!(standard, ["<stdlib>/path.gcsl", "<stdlib>/prelude.gcsl"]);
            assert_eq!(value, "{v = \"a/b\"}", "{text}");
            assert_eq!(lone(text).unwrap(), "{v = \"a/b\"}", "{text}");
        }
        // By any scanned file, even one the file run does not need: every
        // file is checked, and the standard files loaded run first.
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "v = 1"),
            ("other.gcsl", "import path\nw = path.normalize(\"a/.\")"),
        ];
        let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
        assert_eq!(standard.len(), 2);
        assert_eq!(value, "{v = 1}");
        let workspace = checked(&files);
        assert_eq!(workspace.file("main.gcsl"), Some(2));
        assert_eq!(workspace.scan.needed(2), [1, 0, 2]);
        // An explicit `*` from both is the conflict of any two.
        let files = [
            ("workspace.gcsl", ""),
            (
                "main.gcsl",
                "import * from path\nimport * from prelude\nv = 1",
            ),
        ];
        assert!(
            shown_error(&files).starts_with("field `join` is defined in two files at main.gcsl"),
            "{}",
            shown_error(&files)
        );
    }

    #[test]
    fn its_input_numbers_follow_the_standard_files_loaded() {
        // A syntax error is numbered after the standard files the files
        // that parsed need.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "import path"),
            ("b.gcsl", "y = ("),
        ];
        let (standard, checked) = checked_beside(&crate::STANDARD, &files);
        assert_eq!(standard.len(), 2);
        assert_eq!(checked.unwrap_err()[0].input, Some(3));
        // A file that fails to parse imports nothing.
        let files = [("workspace.gcsl", ""), ("a.gcsl", "import path\ny = (")];
        let (standard, checked) = checked_beside(&crate::STANDARD, &files);
        assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
        assert_eq!(checked.unwrap_err()[0].input, Some(1));
        // Run alone with `path`, the prelude is input 1, and the file's
        // diagnostics name no input.
        let error = lone_error("import path\nx = sh(\"exit 2\")");
        assert_eq!(error.input, Some(1));
        let source = SourceFile::new("lone.gcsl", "import path\nx = 1 + true").unwrap();
        assert_eq!(crate::run(&source).unwrap_err()[0].input, None);
    }

    #[test]
    fn a_standard_module_only_another_imports_is_loaded() {
        // A table of two modules, `a` importing `b`, beside the prelude.
        let table = [
            ("a", "import b\nx = b.y + 1"),
            ("b", "y = len([1, 2])"),
            prelude_entry(),
        ];
        let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
        let (standard, value) = value_beside(&table, &files, "main.gcsl");
        assert_eq!(
            standard,
            [
                "<stdlib>/a.gcsl",
                "<stdlib>/b.gcsl",
                "<stdlib>/prelude.gcsl"
            ]
        );
        assert_eq!(value, "{v = 3}");
        // `b` alone; and neither when nobody imports them.
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import y from b\nv = y"),
        ];
        let (standard, value) = value_beside(&table, &files, "main.gcsl");
        assert_eq!(standard, ["<stdlib>/b.gcsl", "<stdlib>/prelude.gcsl"]);
        assert_eq!(value, "{v = 2}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", "v = 1")];
        let (standard, _) = value_beside(&table, &files, "main.gcsl");
        assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
        // Standard modules importing each other are the usual error.
        let table = [
            ("a", "import b\nx = b.y"),
            ("b", "import a\ny = a.x"),
            prelude_entry(),
        ];
        let files = [("workspace.gcsl", ""), ("main.gcsl", "import a\nv = a.x")];
        let (_, checked) = checked_beside(&table, &files);
        let messages: Vec<String> = checked
            .unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(
            messages,
            ["`<stdlib>/a.gcsl` and `<stdlib>/b.gcsl` need each other"]
        );
    }

    #[test]
    fn standard_modules_are_scopes_of_their_own() {
        // Two may declare one top-level name: the prelude's string `join`
        // and `path`'s, and two of a test's table.
        let text = "import path\nv = [join([\"a\", \"b\"], \"-\"), path.join(\"a\", \"b\")]";
        assert_eq!(lone(text).unwrap(), "{v = [\"a-b\", \"a/b\"]}");
        let table = [("a", "x = 1"), ("b", "x = 2"), prelude_entry()];
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import a\nimport b\nv = a.x * 10 + b.x"),
        ];
        assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 12}");
        // One sees nothing of another it does not import, though both are
        // loaded; it sees the prelude's names.
        let table = [("a", "x = 1"), ("b", "y = x"), prelude_entry()];
        let files = [
            ("workspace.gcsl", ""),
            ("main.gcsl", "import a\nimport b\nv = b.y"),
        ];
        let (standard, checked) = checked_beside(&table, &files);
        assert_eq!(standard.len(), 3);
        let errors = checked.unwrap_err();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].message, "undefined variable `x`");
        assert_eq!(errors[0].input, Some(1));
        let table = [("a", "x = 1"), ("b", "y = half(2)"), prelude_entry()];
        let files = [("workspace.gcsl", ""), ("main.gcsl", "import b\nv = b.y")];
        assert_eq!(value_beside(&table, &files, "main.gcsl").1, "{v = 1.0}");
        // Nor anything of the workspace.
        let table = [("a", "y = shared"), prelude_entry()];
        let files = [
            ("workspace.gcsl", "shared = 1"),
            ("main.gcsl", "import a\nv = a.y"),
        ];
        let (_, checked) = checked_beside(&table, &files);
        assert_eq!(
            checked.unwrap_err()[0].message,
            "undefined variable `shared`"
        );
    }

    #[test]
    fn path_is_a_reserved_name() {
        for files in [
            &[("workspace.gcsl", ""), ("path.gcsl", "x = 1")][..],
            &[("path/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
        ] {
            // Reserved whether or not a file imports it.
            assert_eq!(
                error(files),
                (
                    "`path` is the name of a standard module".to_owned(),
                    Some(1),
                    vec![]
                ),
                "{files:?}"
            );
        }
        // Below the root, a name of a path of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/path.gcsl", "x = 1"),
            ("main.gcsl", "import lib.path\nn = path.x"),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{n = 1}");
    }

    #[test]
    fn a_file_s_own_join_shadows_the_prelude_s() {
        for (text, expected) in [
            (
                "join = \"mine\"\nv = join",
                "{join = \"mine\"; v = \"mine\"}",
            ),
            (
                "def join(a: Int, b: Int): Int { a + b }\nv = join(1, 2)",
                "{join = fn(a: Int, b: Int): Int; v = 3}",
            ),
            (
                "import path\njoin = 1\nv = path.join(\"a\", \"b\")",
                "{join = 1; v = \"a/b\"}",
            ),
            ("let trim = 2\nv = trim", "{v = 2}"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
    }

    #[test]
    fn sh_s_message_has_its_standard_error_trimmed() {
        let error = lone_error("x = sh(\"printf ' a\\\\n\\\\n  ' >&2; exit 1\")");
        assert_eq!(
            error.message,
            "`printf ' a\\n\\n  ' >&2; exit 1` exited with status 1: a"
        );
        // Its result is still the raw standard output.
        assert_eq!(lone("= sh(\"printf ' a\\\\n'\")").unwrap(), " a\n");
    }

    // ----- More string built-ins, and `compare_versions` -----

    /// The summary of slice 41.
    const MORE_STRINGS_SUMMARY: &str = "a = strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")  # \"6.6\"\n\
         b = strip_prefix(\"x\", \"y\")                                     # \"x\"\n\
         c = strip_suffix(\"a.tar.gz\", \".gz\")                            # \"a.tar\"\n\
         d = lines(\"a\\nb\\n\")                                            # [\"a\", \"b\"]\n\
         e = fields(\"  tcp\\t6  TCP \")                                   # [\"tcp\", \"6\", \"TCP\"]\n\
         f = compare_versions(\"6.10\", \"6.6\")                            # 1";

    /// `text` run as a lone file and as a file of a workspace, each giving
    /// `expected`.
    fn runs_to(text: &str, expected: &str) {
        assert_eq!(lone(text).unwrap(), expected, "{text}");
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
    }

    #[test]
    fn the_more_strings_summary_runs_in_a_workspace_and_alone() {
        let expected = "{a = \"6.6\"; b = \"x\"; c = \"a.tar\"; d = [\"a\", \"b\"]; \
                        e = [\"tcp\", \"6\", \"TCP\"]; f = 1}";
        runs_to(MORE_STRINGS_SUMMARY, expected);
        // Nothing but the prelude is loaded for them.
        let files = [("workspace.gcsl", ""), ("main.gcsl", MORE_STRINGS_SUMMARY)];
        let scan = ws(&files).unwrap();
        assert_eq!(scan.standard(), 1);
        assert_eq!(scan.files[0].path, "<stdlib>/prelude.gcsl");
    }

    #[test]
    fn strip_prefix_and_strip_suffix_remove_what_is_there_once() {
        for (text, expected) in [
            (
                "= strip_prefix(\"refs/tags/debian/6.6\", \"refs/tags/debian/\")",
                "6.6",
            ),
            ("= strip_prefix(\"x\", \"y\")", "x"),
            ("= strip_prefix(\"aab\", \"a\")", "ab"),
            ("= strip_prefix(\"abc\", \"abc\")", ""),
            ("= strip_prefix(\"ab\", \"abc\")", "ab"),
            ("= strip_prefix(\"abc\", \"\")", "abc"),
            ("= strip_prefix(\"\", \"\")", ""),
            ("= strip_prefix(\"\", \"a\")", ""),
            ("= strip_prefix(\"ba\", \"a\")", "ba"),
            ("= strip_suffix(\"a.tar.gz\", \".gz\")", "a.tar"),
            ("= strip_suffix(\"a.gz.gz\", \".gz\")", "a.gz"),
            ("= strip_suffix(\"a.tar\", \".gz\")", "a.tar"),
            ("= strip_suffix(\"abc\", \"\")", "abc"),
            ("= strip_suffix(\"\", \"\")", ""),
            ("= strip_suffix(\"gz\", \".gz\")", "gz"),
            ("= strip_suffix(\".gz\", \".gz\")", ""),
            // Byte-wise: a precomposed `é` is not an `e` with a combining
            // accent, and a prefix of `e` leaves the accent, a character of
            // its own, while it is not the start of a precomposed `é`.
            ("= strip_prefix(\"\u{e9}t\u{e9}\", \"\u{e9}\")", "t\u{e9}"),
            (
                "= strip_prefix(\"e\\u{301}t\u{e9}\", \"\u{e9}\")",
                "e\u{301}t\u{e9}",
            ),
            ("= strip_prefix(\"e\\u{301}x\", \"e\")", "\u{301}x"),
            ("= strip_prefix(\"\u{e9}t\u{e9}\", \"e\")", "\u{e9}t\u{e9}"),
            ("= strip_suffix(\"caf\u{e9}\", \"\u{e9}\")", "caf"),
            (
                "= strip_suffix(\"cafe\\u{301}\", \"\u{e9}\")",
                "cafe\u{301}",
            ),
            (
                "= strip_prefix(\"\u{65e5}\u{672c}\", \"\u{65e5}\")",
                "\u{672c}",
            ),
            // By name, and through a variable.
            ("= strip_prefix(prefix = \"v\", s = \"v1.2\")", "1.2"),
            ("let f = strip_suffix\n= f(\"x.c\", \".c\")", "x"),
        ] {
            runs_to(text, expected);
        }
    }

    #[test]
    fn lines_and_fields_split_strings() {
        for (text, expected) in [
            ("= lines(\"a\\nb\\n\")", "[\"a\", \"b\"]"),
            ("= lines(\"\")", "[]"),
            ("= lines(\"\\n\")", "[\"\"]"),
            ("= lines(\"\\n\\n\")", "[\"\", \"\"]"),
            ("= lines(\"a\")", "[\"a\"]"),
            ("= lines(\"a\\n\\nb\")", "[\"a\", \"\", \"b\"]"),
            ("= lines(\"a\\r\\nb\\r\\n\")", "[\"a\", \"b\"]"),
            ("= lines(\"a\\r\\n\\r\\nb\")", "[\"a\", \"\", \"b\"]"),
            // A `\r` alone ends no line, and stays, as Rust's `lines` has
            // it.
            ("= lines(\"a\\rb\")", "[\"a\\rb\"]"),
            ("= lines(\"a\\r\")", "[\"a\\r\"]"),
            ("= lines(\" a \\n\\tb\")", "[\" a \", \"\\tb\"]"),
            (
                "= lines(\"h\u{e9}\\n\u{65e5}\")",
                "[\"h\u{e9}\", \"\u{65e5}\"]",
            ),
            ("= fields(\"  tcp\\t6  TCP \")", "[\"tcp\", \"6\", \"TCP\"]"),
            ("= fields(\"\")", "[]"),
            ("= fields(\"  \")", "[]"),
            ("= fields(\" \\n\\t\\r \")", "[]"),
            ("= fields(\"a\")", "[\"a\"]"),
            (
                "= fields(\"a b\\nc\\r\\nd\")",
                "[\"a\", \"b\", \"c\", \"d\"]",
            ),
            // Unicode's whitespace, as `trim`'s: a no-break space and an
            // ideographic space separate fields too.
            ("= fields(\"a\\u{a0}b\")", "[\"a\", \"b\"]"),
            (
                "= fields(\"\\u{3000}x\\u{3000}\\u{3000}y\")",
                "[\"x\", \"y\"]",
            ),
            (
                "= fields(\"h\u{e9} \u{65e5}\")",
                "[\"h\u{e9}\", \"\u{65e5}\"]",
            ),
            // Together, as a table is read.
            (
                "= for line in lines(\"tcp 6 TCP\\nudp\\t17 UDP\\n\") { fields(line)[1] }",
                "[\"6\", \"17\"]",
            ),
            ("let f = fields\n= f(s = \"x y\")", "[\"x\", \"y\"]"),
        ] {
            runs_to(text, expected);
        }
    }

    #[test]
    fn compare_versions_orders_versions() {
        // Each pair in order, `a` before `b`: the other way round is `1`.
        let long = "1234567890123456789012345678901234567890";
        let longer = "12345678901234567890123456789012345678901";
        for (a, b) in [
            ("6.6", "6.10"),
            ("6.6", "6.6.1"),
            ("6.3", "6.3+nmu1"),
            ("6.3+nmu1", "6.4"),
            ("1.0~rc1", "1.0"),
            ("1.0~rc1", "1.0~rc2"),
            ("1.0~~", "1.0~"),
            ("1.0", "1.0a"),
            ("1.0", "1.0.0"),
            ("2", "10"),
            ("", "0"),
            ("", "a"),
            ("~", ""),
            ("1.01", "1.1"),
            ("01", "1"),
            ("a", "b"),
            ("Z", "a"),
            ("a", "."),
            ("a", "+"),
            ("1a", "1."),
            ("1.2.3", "1.2.10"),
            ("1:1.0", "1:1.1"),
            ("2.30-1", "2.30-2"),
            (long, longer),
            (
                "1.1234567890123456789012345678901234567890",
                "1.1234567890123456789012345678901234567891",
            ),
            // Bytes, not characters: a letter before any other byte, a
            // multi-byte character's bytes among them.
            ("1.a", "1.\u{e9}"),
            ("\u{e9}", "\u{f6}"),
        ] {
            let before = format!("= compare_versions(\"{a}\", \"{b}\")");
            runs_to(&before, "-1");
            let after = format!("= compare_versions(\"{b}\", \"{a}\")");
            runs_to(&after, "1");
        }
        // Equal strings, and only those, are `0`.
        for a in ["", "0", "6.6", "1.0~rc1", long, "\u{e9}"] {
            runs_to(&format!("= compare_versions(\"{a}\", \"{a}\")"), "0");
        }
        // By name, through `builtins`, and in a pure function.
        runs_to("= compare_versions(b = \"6.6\", a = \"6.10\")", "1");
        runs_to(
            "import builtins\n= builtins.compare_versions(\"1\", \"2\")",
            "-1",
        );
        runs_to(
            "fn newer(a: String, b: String): Bool { compare_versions(a, b) > 0 }\n\
             = [newer(\"6.10\", \"6.6\"), newer(\"1.0~rc1\", \"1.0\")]",
            "[true, false]",
        );
    }

    #[test]
    fn the_more_string_builtins_are_values_of_their_types() {
        for (text, expected) in [
            ("= strip_prefix", "fn(s: String, prefix: String): String"),
            ("= strip_suffix", "fn(s: String, suffix: String): String"),
            ("= lines", "fn(s: String): List<String>"),
            ("= fields", "fn(s: String): List<String>"),
            ("= compare_versions", "fn(a: String, b: String): Int"),
            // Through `builtins`.
            (
                "import builtins\n= builtins.fields(\" a  b \")",
                "[\"a\", \"b\"]",
            ),
            (
                "import builtins\n= builtins.lines",
                "fn(s: String): List<String>",
            ),
            ("import lines from builtins\n= lines(\"x\\n\")", "[\"x\"]"),
        ] {
            runs_to(text, expected);
        }
        // A file's own declaration of one of their names shadows the
        // prelude's silently, and loses the built-in there.
        for (text, expected) in [
            (
                "fields = \"mine\"\nv = fields",
                "{fields = \"mine\"; v = \"mine\"}",
            ),
            ("let lines = 2\nv = lines", "{v = 2}"),
            (
                "def compare_versions(a: Int): Int { a }\nv = compare_versions(3)",
                "{compare_versions = fn(a: Int): Int; v = 3}",
            ),
            (
                "fields.x = 1\nv = trim(\" a \")",
                "{fields = {x = 1}; v = \"a\"}",
            ),
        ] {
            runs_to(text, expected);
        }
    }

    // ----- The `fs` module -----

    /// A new empty directory for the `fs` test `name` to work in, in the
    /// system's temporary directory, never the repository's: its absolute
    /// path, canonical.
    fn fs_scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("gcsl-slice-42-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// The summary of slice 42, which copies `x.txt` from its own directory.
    const FS_SUMMARY: &str = "import fs\n\
                              import fileroot\n\
                              import path\n\
                              let dir = fs.temp_dir()\n\
                              fs.mkdir(path.join(dir, \"a/b\"))\n\
                              fs.copy(path.join(fileroot.this, \"x.txt\"), path.join(dir, \"a/b/x.txt\"))\n\
                              there = fs.exists(path.join(dir, \"a/b/x.txt\"))\n\
                              text = fs.read(path.join(dir, \"a/b/x.txt\"))";

    /// Runs `text` as the lone file `lone.gcsl` of the directory `dir`,
    /// returning its value shown, or its diagnostics, which name the file by
    /// no input, as [`crate::run`]'s do; the workspace it ran in, and its
    /// temporary directories, dropped before this returns.
    fn lone_in(dir: &str, text: &str) -> Result<String, Vec<Diagnostic>> {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let (loaded, checked) = check_lone_with_standard(&source, dir);
        let index = count(loaded.len());
        let being_run = |diagnostics| being_run(diagnostics, index);
        let mut workspace = checked.map_err(being_run)?;
        workspace
            .run(index)
            .map(|outcome| outcome.value.to_string())
            .map_err(being_run)
    }

    /// The one diagnostic of running `text` as a lone file of `dir`.
    fn lone_error_in(dir: &str, text: &str) -> Diagnostic {
        let mut errors = lone_in(dir, text).unwrap_err();
        assert_eq!(errors.len(), 1, "{text}: {errors:?}");
        errors.remove(0)
    }

    #[test]
    fn the_fs_summary_runs_in_a_workspace_and_alone() {
        let dir = fs_scratch("summary");
        std::fs::write(format!("{dir}/x.txt"), "hello, fs\n").unwrap();
        let expected = "{there = true; text = \"hello, fs\\n\"}";
        assert_eq!(lone_in(&dir, FS_SUMMARY).unwrap(), expected);
        // In a workspace, from a file of a directory of its own.
        std::fs::create_dir(format!("{dir}/app")).unwrap();
        std::fs::write(format!("{dir}/app/x.txt"), "hello, fs\n").unwrap();
        let sources = [
            SourceFile::new("workspace.gcsl", "").unwrap(),
            SourceFile::new("app/main.gcsl", FS_SUMMARY).unwrap(),
        ];
        let (standard, checked) = check_with_standard(&sources, &[], &dir);
        // `fs` and `path` are loaded, before the prelude, in path order.
        let names: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        assert_eq!(
            names,
            [
                "<stdlib>/fs.gcsl",
                "<stdlib>/path.gcsl",
                "<stdlib>/prelude.gcsl"
            ]
        );
        let mut workspace = checked.unwrap();
        let main = workspace.file("app/main.gcsl").unwrap();
        assert_eq!(main, 3);
        assert_eq!(workspace.run(main).unwrap().value.to_string(), expected);
        drop(workspace);
        // A relative path is an error at the call, before anything is
        // touched.
        let text = "import fs\ntext = fs.read(\"x.txt\")";
        let error = lone_error_in(&dir, text);
        assert_eq!(
            error.message,
            "`fs.read` needs an absolute path, found `x.txt`"
        );
        assert_eq!(error.span, Span::new(17, 33));
        assert_eq!(error.input, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn each_member_of_fs_does_what_it_says() {
        let dir = fs_scratch("members");
        std::fs::write(format!("{dir}/x.txt"), "h\u{e9}llo\n").unwrap();
        std::os::unix::fs::symlink(format!("{dir}/nothing"), format!("{dir}/dangling")).unwrap();
        std::os::unix::fs::symlink(format!("{dir}/x.txt"), format!("{dir}/link")).unwrap();
        for (text, expected) in [
            // `exists`: a file, a directory, nothing, links followed.
            (format!("= fs.exists(\"{dir}/x.txt\")"), "true"),
            (format!("= fs.exists(\"{dir}\")"), "true"),
            (format!("= fs.exists(\"{dir}/missing\")"), "false"),
            (format!("= fs.exists(\"{dir}/link\")"), "true"),
            (format!("= fs.exists(\"{dir}/dangling\")"), "false"),
            ("= fs.exists(\"/\")".to_owned(), "true"),
            // `read`: the text, UTF-8, through a link too.
            (format!("= fs.read(\"{dir}/x.txt\")"), "h\u{e9}llo\n"),
            (format!("= fs.read(p = \"{dir}/link\")"), "h\u{e9}llo\n"),
            // `mkdir`: nested, `{}`, and again, fine.
            (
                format!(
                    "fs.mkdir(\"{dir}/m/n/o\")\nfs.mkdir(\"{dir}/m/n/o\")\nfs.mkdir(\"{dir}/m\")\n\
                     = fs.exists(\"{dir}/m/n/o\")"
                ),
                "true",
            ),
            (format!("= fs.mkdir(\"{dir}/m2\")"), "{}"),
            // `copy`: a new file, then over it; `{}`.
            (
                format!("fs.copy(\"{dir}/x.txt\", \"{dir}/y.txt\")\n= fs.read(\"{dir}/y.txt\")"),
                "h\u{e9}llo\n",
            ),
            (
                format!("= fs.copy(to = \"{dir}/y.txt\", from = \"{dir}/link\")"),
                "{}",
            ),
        ] {
            assert_eq!(
                lone_in(&dir, &format!("import fs\n{text}")).unwrap(),
                expected,
                "{text}"
            );
        }
        // `copy` replaces a file's contents, and copies its permissions.
        std::fs::write(format!("{dir}/z.txt"), "old").unwrap();
        std::fs::write(format!("{dir}/w.txt"), "new").unwrap();
        let mode = std::os::unix::fs::PermissionsExt::from_mode(0o640);
        std::fs::set_permissions(format!("{dir}/w.txt"), mode).unwrap();
        let text = format!("import fs\n= fs.copy(\"{dir}/w.txt\", \"{dir}/z.txt\")");
        assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/z.txt")).unwrap(),
            "new"
        );
        let copied = std::fs::metadata(format!("{dir}/z.txt")).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&copied.permissions()) & 0o777,
            0o640
        );
        // Every form of import, and `builtins` itself.
        for text in [
            format!("import exists from fs\n= exists(\"{dir}\")"),
            format!("import exists as here from fs\n= here(\"{dir}\")"),
            format!("import fs as f\n= f.exists(\"{dir}\")"),
            format!("import * from fs\n= exists(\"{dir}\")"),
            format!("import builtins\n= builtins.fs_exists(\"{dir}\")"),
            format!("import fs\nlet e = fs.exists\n= e(\"{dir}\")"),
            format!("import fs\nfn f(p: String) -> Bool {{ fs.exists(p) }}\n= f(\"{dir}\")"),
            format!("import fs\n= await lazy fs.exists(\"{dir}\")"),
        ] {
            assert_eq!(lone_in(&dir, &text).unwrap(), "true", "{text}");
        }
        // The paths are used as given: not normalised, and so `..` is the
        // file system's own.
        let text = format!("import fs\n= fs.read(\"{dir}/m/../x.txt\")");
        assert_eq!(lone_in(&dir, &text).unwrap(), "h\u{e9}llo\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_path_given_to_fs_must_be_absolute() {
        let dir = fs_scratch("relative");
        // Named whatever it is imported as, at the call, the paths of each
        // member checked before anything is touched.
        let made = "gcsl-slice-42-never-made";
        for (text, message) in [
            (
                "import fs\n= fs.exists(\"a\")",
                "`fs.exists` needs an absolute path, found `a`",
            ),
            (
                "import fs\n= fs.read(\"x.txt\")",
                "`fs.read` needs an absolute path, found `x.txt`",
            ),
            (
                "import fs\n= fs.read(\"\")",
                "`fs.read` needs an absolute path, found ``",
            ),
            (
                "import fs\n= fs.mkdir(\"gcsl-slice-42-never-made/a\")",
                "`fs.mkdir` needs an absolute path, found `gcsl-slice-42-never-made/a`",
            ),
            (
                "import fs\n= fs.copy(\"./a\", \"/nowhere/b\")",
                "`fs.copy` needs an absolute path, found `./a`",
            ),
            (
                "import fs\n= fs.copy(\"/nowhere/a\", \"b\")",
                "`fs.copy` needs an absolute path, found `b`",
            ),
            (
                "import fs\n= fs.copy(\"a\", \"b\")",
                "`fs.copy` needs an absolute path, found `a`",
            ),
            (
                "import exists from fs\n= exists(\"x\")",
                "`fs.exists` needs an absolute path, found `x`",
            ),
            (
                "import fs as f\n= f.exists(\"x\")",
                "`fs.exists` needs an absolute path, found `x`",
            ),
            (
                "import read as slurp from fs\n= slurp(\"x\")",
                "`fs.read` needs an absolute path, found `x`",
            ),
            (
                "import builtins\n= builtins.fs_mkdir(\"x\")",
                "`fs.mkdir` needs an absolute path, found `x`",
            ),
        ] {
            let error = lone_error_in(&dir, text);
            assert_eq!(error.message, message, "{text}");
            // At the user's call, the whole of it, in the user's file.
            let call = &text[text.find("= ").unwrap() + 2..];
            assert_eq!(error.span.start as usize, text.len() - call.len(), "{text}");
            assert_eq!(error.span.end as usize, text.len(), "{text}");
            assert_eq!(error.input, None, "{text}");
            assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
        }
        assert!(!std::path::Path::new(made).exists());
        // In a workspace, the error names the file that made the call, not
        // `fs.gcsl`.
        let files = [
            ("workspace.gcsl", ""),
            (
                "lib/util.gcsl",
                "import fs\ndef load(p: String) -> String { fs.read(p) }",
            ),
            ("main.gcsl", "import lib.util\nv = util.load(\"x\")"),
        ];
        let error = run_file(&files, "main.gcsl").unwrap_err();
        let util = checked(&files).file("lib/util.gcsl");
        assert_eq!(
            error,
            [(
                "`fs.read` needs an absolute path, found `x`".to_owned(),
                util
            )]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_of_fs_names_the_member_the_paths_and_the_reason() {
        let dir = fs_scratch("failures");
        std::fs::write(format!("{dir}/x.txt"), "x").unwrap();
        std::fs::write(format!("{dir}/bad.bin"), [b'a', 0xff, 0xfe]).unwrap();
        let copy_dir = std::fs::copy(&dir, format!("{dir}/never"))
            .unwrap_err()
            .to_string();
        for (text, message) in [
            (
                format!("= fs.read(\"{dir}/missing\")"),
                format!(
                    "`fs.read` could not read `{dir}/missing`: No such file or directory (os error 2)"
                ),
            ),
            (
                format!("= fs.read(\"{dir}\")"),
                format!("`fs.read` could not read `{dir}`: Is a directory (os error 21)"),
            ),
            (
                format!("= fs.read(\"{dir}/bad.bin\")"),
                format!("`fs.read`: `{dir}/bad.bin` is not UTF-8 text"),
            ),
            (
                format!("= fs.mkdir(\"{dir}/x.txt/sub\")"),
                format!(
                    "`fs.mkdir` could not create `{dir}/x.txt/sub`: Not a directory (os error 20)"
                ),
            ),
            (
                format!("= fs.mkdir(\"{dir}/x.txt\")"),
                format!("`fs.mkdir` could not create `{dir}/x.txt`: File exists (os error 17)"),
            ),
            (
                format!("= fs.copy(\"{dir}\", \"{dir}/c\")"),
                format!("`fs.copy` could not copy `{dir}` to `{dir}/c`: {copy_dir}"),
            ),
            (
                format!("= fs.copy(\"{dir}/missing\", \"{dir}/c\")"),
                format!(
                    "`fs.copy` could not copy `{dir}/missing` to `{dir}/c`: No such file or \
                     directory (os error 2)"
                ),
            ),
            (
                format!("= fs.copy(\"{dir}/x.txt\", \"{dir}/none/x.txt\")"),
                format!(
                    "`fs.copy` could not copy `{dir}/x.txt` to `{dir}/none/x.txt`: No such file \
                     or directory (os error 2)"
                ),
            ),
        ] {
            let text = format!("import fs\n{text}");
            let error = lone_error_in(&dir, &text);
            assert_eq!(error.message, message, "{text}");
            assert_eq!(error.span.end as usize, text.len(), "{text}");
            assert_eq!(error.span.start, 12, "{text}");
            assert_eq!(error.input, None, "{text}");
        }
        assert!(!std::path::Path::new(&format!("{dir}/c")).exists());
        assert!(!std::path::Path::new(&format!("{dir}/never")).exists());
        // What cannot be checked: below a directory that cannot be searched,
        // unless the process may search anything.
        let locked = format!("{dir}/locked");
        std::fs::create_dir(&locked).unwrap();
        let mode = std::os::unix::fs::PermissionsExt::from_mode(0o000);
        std::fs::set_permissions(&locked, mode).unwrap();
        let text = format!("import fs\n= fs.exists(\"{locked}/x\")");
        let result = lone_in(&dir, &text).map_err(|errors| errors[0].message.clone());
        let expected = match std::path::Path::new(&format!("{locked}/x")).try_exists() {
            Ok(exists) => Ok(exists.to_string()),
            Err(error) => Err(format!("`fs.exists` could not check `{locked}/x`: {error}")),
        };
        let mode = std::os::unix::fs::PermissionsExt::from_mode(0o700);
        std::fs::set_permissions(&locked, mode).unwrap();
        assert_eq!(result, expected);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The temporary directories a run gave as its value, a list of their
    /// paths, once checked to be there, while the run's owner lives: each
    /// absolute, a directory, empty, of mode `0700`, and another than those
    /// before it, all children of one parent of this process's in the
    /// system's temporary directory.
    fn assert_fresh_temp_dirs(value: &Value) -> Vec<String> {
        let Value::List(list) = value else {
            panic!("not a list: {value}");
        };
        let dirs: Vec<String> = list
            .values()
            .iter()
            .map(|dir| match dir {
                Value::Str(dir) => dir.to_string(),
                other => panic!("not a string: {other}"),
            })
            .collect();
        let prefix = format!("gcsl-{}-", std::process::id());
        let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
        for (at, dir) in dirs.iter().enumerate() {
            let path = std::path::Path::new(dir);
            assert!(path.is_absolute(), "{dir}");
            assert!(path.is_dir(), "{dir}");
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 0, "{dir}");
            let mode = std::fs::metadata(path).unwrap().permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o700
            );
            assert_eq!(path.parent().unwrap(), parent, "{dir}");
            assert!(!dirs[..at].contains(dir), "{dir} twice");
        }
        let name = parent.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(&prefix), "{name}");
        assert_eq!(parent.parent().unwrap(), std::env::temp_dir());
        dirs
    }

    #[test]
    fn temporary_directories_live_as_long_as_the_workspace() {
        let text = "import fs\n\
                    fn fresh() -> String { fs.temp_dir() }\n\
                    let a = fs.temp_dir()\n\
                    let b = fresh()\n\
                    = [a, b, fresh(), await lazy fs.temp_dir()]";
        // A workspace's: made by its runs, there until it is dropped.
        let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check(&sources).unwrap();
        let main = workspace.file("main.gcsl").unwrap();
        let dirs = assert_fresh_temp_dirs(&workspace.run(main).unwrap().value);
        assert_eq!(dirs.len(), 4);
        let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
        assert_eq!(std::path::Path::new(&dirs[0]).file_name().unwrap(), "1");
        // Removed with what was put in them.
        std::fs::write(format!("{}/kept", dirs[1]), "x").unwrap();
        drop(workspace);
        assert!(!parent.exists(), "{}", parent.display());
        // A lone file's: gone as its run is.
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let (loaded, checked) = check_lone_with_standard(&source, "/");
        let mut workspace = checked.unwrap();
        let outcome = workspace.run(count(loaded.len())).unwrap();
        let dirs = assert_fresh_temp_dirs(&outcome.value);
        let parent = std::path::Path::new(&dirs[0]).parent().unwrap().to_owned();
        drop(workspace);
        assert!(!parent.exists(), "{}", parent.display());
        let Value::List(dirs) = crate::run(&source).unwrap().value else {
            panic!("not a list");
        };
        for dir in dirs.values() {
            assert!(!std::path::Path::new(&dir.to_string()).exists(), "{dir}");
        }
    }

    #[test]
    fn temporary_directories_are_removed_after_a_run_that_failed() {
        // With what the run put in them.
        let text = "import fs\n\
                    let d = fs.temp_dir()\n\
                    fs.mkdir(\"{d}/a/b\")\n\
                    = panic(d)";
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let (loaded, checked) = check_lone_with_standard(&source, "/");
        let mut workspace = checked.unwrap();
        let error = workspace.run(count(loaded.len())).unwrap_err().remove(0);
        let dir = std::path::Path::new(&error.message).to_owned();
        assert!(dir.join("a/b").is_dir());
        drop(workspace);
        assert!(!dir.exists());
        assert!(!dir.parent().unwrap().exists());
        let error = crate::run(&source).unwrap_err().remove(0);
        assert!(!std::path::Path::new(&error.message).exists());
        // A run that fails before making one: nothing to remove.
        let error = lone_error("import fs\n= fs.read(\"x\")");
        assert_eq!(error.message, "`fs.read` needs an absolute path, found `x`");
    }

    #[test]
    fn each_call_of_temp_dir_is_its_own() {
        // A memoised function makes one for each list of arguments; an
        // impure one, one each call.
        let text = "import fs\n\
                    fn once(n: Int) ->> String { fs.temp_dir() }\n\
                    fn each(n: Int) -> String { fs.temp_dir() }\n\
                    = [once(1), once(1), once(2), each(1), each(1)]";
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let (loaded, checked) = check_lone_with_standard(&source, "/");
        let mut workspace = checked.unwrap();
        let outcome = workspace.run(count(loaded.len())).unwrap();
        let Value::List(dirs) = outcome.value else {
            panic!("not a list");
        };
        let dirs: Vec<String> = dirs.values().iter().map(ToString::to_string).collect();
        assert_eq!(dirs[0], dirs[1]);
        let mut distinct = vec![&dirs[0], &dirs[2], &dirs[3], &dirs[4]];
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 4, "{dirs:?}");
    }

    #[test]
    fn temp_dir_in_a_run_without_an_owner_is_an_internal_error() {
        // The run of the file's IR alone, given no temporary directories,
        // once the standard files have run in the workspace.
        let source = SourceFile::new("lone.gcsl", "import fs\n= fs.temp_dir()").unwrap();
        let (loaded, checked) = check_lone_with_standard(&source, "/");
        let mut workspace = checked.unwrap();
        for file in 0..count(loaded.len()) {
            workspace.run(file).unwrap();
        }
        let ir = workspace.irs[loaded.len()].take().unwrap();
        let ir = Rc::new(ir);
        let size = ir.frame_size() as usize;
        workspace
            .frame
            .resize(size.max(workspace.frame.len()), Value::Empty);
        let context = (
            Some(&workspace.exec_result),
            &workspace.scan.dir_texts[..],
            None,
        );
        let error = crate::eval::eval_with(&ir, &mut workspace.frame, &mut workspace.memo, context)
            .unwrap_err();
        assert!(
            error.message.starts_with("internal error"),
            "{}",
            error.message
        );
    }

    #[test]
    fn fs_is_loaded_only_when_imported() {
        // Not imported: not loaded, and input numbers and slots are those of
        // a run beside the prelude alone.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/a.gcsl", "x = [1]\ny = len(x)"),
            ("main.gcsl", "import lib.a\nz = a.y + 1"),
        ];
        let (standard, checked) = checked_beside(&crate::STANDARD, &files);
        assert_eq!(standard, ["<stdlib>/prelude.gcsl"]);
        let mut workspace = checked.unwrap();
        let (_, bare) = checked_beside(&[prelude_entry()], &files);
        let mut bare = bare.unwrap();
        assert_eq!(workspace.scan.cells(), bare.scan.cells());
        assert_eq!(workspace.slot_names, bare.slot_names);
        let main = workspace.file("main.gcsl").unwrap();
        assert_eq!(main, 2);
        assert_eq!(workspace.dump_ir(main), bare.dump_ir(main));
        assert_eq!(
            workspace.run(main).unwrap().value,
            bare.run(main).unwrap().value
        );
        // Imported by any form, from any file scanned: loaded first.
        for text in [
            "import fs",
            "import fs as f",
            "import exists from fs",
            "import read as r from fs",
            "import * from fs",
        ] {
            let files = [
                ("workspace.gcsl", ""),
                ("main.gcsl", "v = 1"),
                ("other.gcsl", text),
            ];
            let (standard, value) = value_beside(&crate::STANDARD, &files, "main.gcsl");
            assert_eq!(standard, ["<stdlib>/fs.gcsl", "<stdlib>/prelude.gcsl"]);
            assert_eq!(value, "{v = 1}");
        }
        // A lone file too.
        let source = SourceFile::new("lone.gcsl", "import fs").unwrap();
        let (standard, checked) = check_lone_with_standard(&source, "/");
        assert_eq!(standard.len(), 2);
        assert_eq!(checked.unwrap().file("lone.gcsl"), Some(2));
    }

    #[test]
    fn fs_is_a_reserved_name() {
        for files in [
            &[("workspace.gcsl", ""), ("fs.gcsl", "x = 1")][..],
            &[("fs/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
        ] {
            // Reserved whether or not a file imports it.
            assert_eq!(
                error(files),
                (
                    "`fs` is the name of a standard module".to_owned(),
                    Some(1),
                    vec![]
                ),
                "{files:?}"
            );
        }
        // Below the root, a name of a path of its own.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/fs.gcsl", "x = 1"),
            ("main.gcsl", "import lib.fs\nn = fs.x"),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{n = 1}");
    }

    // ----- `fs.write`, `sort_versions` and `show` -----

    /// The summary of slice 43, its lock file written to `dir`.
    fn locks_summary(dir: &str) -> String {
        format!(
            "import fs\n\
             a = sort_versions([\"6.10\", \"6.6\", \"6.6.1\", \"1.0~rc1\", \"1.0\"])\n\
             b = show({{version = \"6.6\"; commit = \"3a8f\"}})\n\
             c = show(\"say \\\"hi\\\" {{{{x}}}}\\n\")\n\
             d = show([1, 2])\n\
             fs.write(\"{dir}/lock.gcsl\", \"locked = [{{b}}]\\n\")"
        )
    }

    /// The value of the summary of slice 43, shown.
    const LOCKS_SUMMARY_VALUE: &str = r#"{a = ["1.0~rc1", "1.0", "6.6", "6.6.1", "6.10"]; b = "{{version = \"6.6\"; commit = \"3a8f\"}}"; c = "\"say \\\"hi\\\" {{{{x}}}}\\n\""; d = "[1, 2]"}"#;

    /// The names of the entries of `dir` and of every directory below it
    /// that are temporary files of `fs.write`.
    fn temp_files_below(dir: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![std::path::PathBuf::from(dir)];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(".gcsl-tmp-") {
                    found.push(name);
                }
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                }
            }
        }
        found
    }

    /// The permission bits of the file at `path`.
    fn mode_of(path: &str) -> u32 {
        let permissions = std::fs::symlink_metadata(path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::mode(&permissions) & 0o7777
    }

    #[test]
    fn the_locks_summary_runs_in_a_workspace_and_alone() {
        let dir = fs_scratch("locks-summary");
        assert_eq!(
            lone_in(&dir, &locks_summary(&dir)).unwrap(),
            LOCKS_SUMMARY_VALUE
        );
        let written = std::fs::read_to_string(format!("{dir}/lock.gcsl")).unwrap();
        assert_eq!(
            written,
            "locked = [{version = \"6.6\"; commit = \"3a8f\"}]\n"
        );
        // The lock it wrote reads back as the record it showed.
        assert_eq!(
            lone(&written).unwrap(),
            "{locked = [{version = \"6.6\"; commit = \"3a8f\"}]}"
        );
        std::fs::remove_file(format!("{dir}/lock.gcsl")).unwrap();
        // In a workspace, the same.
        let sources = [
            SourceFile::new("workspace.gcsl", "").unwrap(),
            SourceFile::new("app/main.gcsl", locks_summary(&dir)).unwrap(),
        ];
        let (_, checked) = check_with_standard(&sources, &[], &dir);
        let mut workspace = checked.unwrap();
        let main = workspace.file("app/main.gcsl").unwrap();
        assert_eq!(
            workspace.run(main).unwrap().value.to_string(),
            LOCKS_SUMMARY_VALUE
        );
        drop(workspace);
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/lock.gcsl")).unwrap(),
            written
        );
        assert!(temp_files_below(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fs_write_makes_and_replaces_files_whole() {
        let dir = fs_scratch("write");
        for (text, expected) in [
            // A new file, its contents exactly as given.
            (
                format!(
                    "import fs\nfs.write(\"{dir}/new.txt\", \"h\u{e9}llo\\n\")\n\
                     = fs.read(\"{dir}/new.txt\")"
                ),
                "h\u{e9}llo\n",
            ),
            (
                format!("import fs\n= fs.write(\"{dir}/empty.txt\", \"\")"),
                "{}",
            ),
            (format!("import fs\n= fs.read(\"{dir}/empty.txt\")"), ""),
            // Over a file, whole: a shorter text leaves nothing of the
            // longer, and a run reads what it wrote.
            (
                format!(
                    "import fs\nfs.write(\"{dir}/new.txt\", \"a much longer text\")\n\
                     fs.write(\"{dir}/new.txt\", \"b\")\n= fs.read(\"{dir}/new.txt\")"
                ),
                "b",
            ),
            // By every road, and by its parameters' names.
            (
                format!("import write from fs\n= write(\"{dir}/w1\", \"1\")"),
                "{}",
            ),
            (
                format!("import write as put from fs\n= put(\"{dir}/w2\", \"2\")"),
                "{}",
            ),
            (
                format!("import fs as f\n= f.write(\"{dir}/w3\", \"3\")"),
                "{}",
            ),
            (
                format!("import builtins\n= builtins.fs_write(\"{dir}/w4\", \"4\")"),
                "{}",
            ),
            (
                format!("import fs\n= fs.write(contents = \"5\", p = \"{dir}/w5\")"),
                "{}",
            ),
            (
                format!(
                    "import fs\nfn put(p: String) -> {{}} {{ fs.write(p, \"6\") }}\n= put(\"{dir}/w6\")"
                ),
                "{}",
            ),
            (
                format!("import fs\n= await lazy fs.write(\"{dir}/w7\", \"7\")"),
                "{}",
            ),
        ] {
            assert_eq!(lone_in(&dir, &text).unwrap(), expected, "{text}");
        }
        for number in 1..=7 {
            assert_eq!(
                std::fs::read_to_string(format!("{dir}/w{number}")).unwrap(),
                number.to_string()
            );
        }
        // A file it replaces keeps neither its contents nor its permissions:
        // the new file has the mode any new file gets, never an executable
        // bit, whatever the umask.
        let kept = format!("{dir}/kept.sh");
        std::fs::write(&kept, "old").unwrap();
        let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&kept, mode).unwrap();
        let text = format!("import fs\n= fs.write(\"{kept}\", \"new\")");
        assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "new");
        let fresh = format!("{dir}/fresh");
        std::fs::File::create(&fresh).unwrap();
        assert_eq!(mode_of(&kept), mode_of(&fresh));
        assert_eq!(mode_of(&kept) & 0o111, 0);
        // A symbolic link is replaced by the file, not followed.
        let target = format!("{dir}/target.txt");
        std::fs::write(&target, "target").unwrap();
        std::os::unix::fs::symlink(&target, format!("{dir}/link")).unwrap();
        let text = format!("import fs\n= fs.write(\"{dir}/link\", \"over\")");
        assert_eq!(lone_in(&dir, &text).unwrap(), "{}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "target");
        let link = std::fs::symlink_metadata(format!("{dir}/link")).unwrap();
        assert!(link.file_type().is_file());
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/link")).unwrap(),
            "over"
        );
        // No temporary file is left behind.
        assert!(
            temp_files_below(&dir).is_empty(),
            "{:?}",
            temp_files_below(&dir)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_of_fs_write_names_the_path_and_leaves_nothing_behind() {
        let dir = fs_scratch("write-failures");
        std::fs::create_dir(format!("{dir}/full")).unwrap();
        std::fs::write(format!("{dir}/full/inner"), "inner").unwrap();
        std::fs::create_dir(format!("{dir}/empty")).unwrap();
        std::fs::write(format!("{dir}/x.txt"), "x").unwrap();
        for (text, message) in [
            // A relative path, before anything is touched, whatever name it
            // is called by.
            (
                "import fs\n= fs.write(\"x.txt\", \"a\")".to_owned(),
                "`fs.write` needs an absolute path, found `x.txt`".to_owned(),
            ),
            (
                "import fs\n= fs.write(\"\", \"a\")".to_owned(),
                "`fs.write` needs an absolute path, found ``".to_owned(),
            ),
            (
                "import write as put from fs\n= put(\"gcsl-slice-43-never/x\", \"a\")".to_owned(),
                "`fs.write` needs an absolute path, found `gcsl-slice-43-never/x`".to_owned(),
            ),
            // A directory that does not exist, or is a file.
            (
                format!("import fs\n= fs.write(\"{dir}/none/x.txt\", \"a\")"),
                format!(
                    "`fs.write` could not write `{dir}/none/x.txt`: No such file or directory \
                     (os error 2)"
                ),
            ),
            (
                format!("import fs\n= fs.write(\"{dir}/x.txt/y\", \"a\")"),
                format!(
                    "`fs.write` could not write `{dir}/x.txt/y`: Not a directory (os error 20)"
                ),
            ),
            // A directory, full or empty.
            (
                format!("import fs\n= fs.write(\"{dir}/full\", \"a\")"),
                format!("`fs.write` could not write `{dir}/full`: Is a directory (os error 21)"),
            ),
            (
                format!("import fs\n= fs.write(\"{dir}/empty\", \"a\")"),
                format!("`fs.write` could not write `{dir}/empty`: Is a directory (os error 21)"),
            ),
            // A path that names no file at all.
            (
                "import fs\n= fs.write(\"/\", \"a\")".to_owned(),
                "`fs.write` could not write `/`: the path names no file".to_owned(),
            ),
        ] {
            let error = lone_error_in(&dir, &text);
            assert_eq!(error.message, message, "{text}");
            // At the user's call, the whole of it.
            let call = &text[text.find("= ").unwrap() + 2..];
            assert_eq!(error.span.start as usize, text.len() - call.len(), "{text}");
            assert_eq!(error.span.end as usize, text.len(), "{text}");
            assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
        }
        assert!(!std::path::Path::new("gcsl-slice-43-never").exists());
        // Nothing was changed, and no temporary file is left behind.
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/full/inner")).unwrap(),
            "inner"
        );
        assert!(std::path::Path::new(&format!("{dir}/empty")).is_dir());
        assert_eq!(
            std::fs::read_to_string(format!("{dir}/x.txt")).unwrap(),
            "x"
        );
        assert!(!std::path::Path::new(&format!("{dir}/none")).exists());
        assert!(
            temp_files_below(&dir).is_empty(),
            "{:?}",
            temp_files_below(&dir)
        );
        // From a pure function, the purity error, before anything runs.
        let text = format!("import fs\nfn f(): {{}} {{ fs.write(\"{dir}/pure\", \"a\") }}\n= f()");
        assert_eq!(
            lone_error_in(&dir, &text).message,
            "cannot call an impure function from a pure function"
        );
        assert!(!std::path::Path::new(&format!("{dir}/pure")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sort_versions_sorts_in_version_order() {
        for (text, expected) in [
            (
                "= sort_versions([\"6.10\", \"6.6\", \"6.6.1\", \"1.0~rc1\", \"1.0\"])",
                "[\"1.0~rc1\", \"1.0\", \"6.6\", \"6.6.1\", \"6.10\"]",
            ),
            // A string there twice is there twice, side by side; strings
            // that differ but whose runs are equal are apart, by their bytes.
            (
                "= sort_versions([\"1.0\", \"0.9\", \"1.00\", \"1.0\", \"0.9\"])",
                "[\"0.9\", \"0.9\", \"1.0\", \"1.0\", \"1.00\"]",
            ),
            ("= sort_versions([])", "[]"),
            ("let none: List<String> = []\n= sort_versions(none)", "[]"),
            ("= sort_versions([\"6.6\"])", "[\"6.6\"]"),
            (
                "= sort_versions([\"1\", \"2\", \"10\"])",
                "[\"1\", \"2\", \"10\"]",
            ),
            (
                "= sort_versions([\"10\", \"2\", \"1\"])",
                "[\"1\", \"2\", \"10\"]",
            ),
            (
                "= sort_versions([\"1.0\", \"1.0~rc2\", \"1.0~rc1\", \"1.0a\"])",
                "[\"1.0~rc1\", \"1.0~rc2\", \"1.0\", \"1.0a\"]",
            ),
            // The list sorted is not changed.
            (
                "let vs = [\"2\", \"1\"]\nlet sorted = sort_versions(vs)\n= [vs, sorted]",
                "[[\"2\", \"1\"], [\"1\", \"2\"]]",
            ),
            // By name, through `builtins` and a variable, and in a pure
            // function.
            ("= sort_versions(vs = [\"b\", \"a\"])", "[\"a\", \"b\"]"),
            (
                "import builtins\n= builtins.sort_versions([\"2\", \"1\"])",
                "[\"1\", \"2\"]",
            ),
            (
                "let s = sort_versions\n= s([\"2\", \"1\"])",
                "[\"1\", \"2\"]",
            ),
            (
                "fn newest(vs: List<String>): String {\n\
                 let sorted = sort_versions(vs)\n\
                 sorted[len(sorted) - 1]\n\
                 }\n\
                 = newest([\"6.6\", \"6.10\", \"6.9\"])",
                "6.10",
            ),
            // As `git.update` uses it.
            (
                "let tags = [{version = \"6.6\"}, {version = \"6.10\"}, {version = \"5.0\"}]\n\
                 = sort_versions(for t in tags if compare_versions(t.version, \"6.0\") >= 0 { t.version })",
                "[\"6.6\", \"6.10\"]",
            ),
            ("= sort_versions", "fn(vs: List<String>): List<String>"),
            // A file's own declaration shadows the prelude's.
            (
                "sort_versions = 1\nv = sort_versions",
                "{sort_versions = 1; v = 1}",
            ),
        ] {
            runs_to(text, expected);
        }
        // Agrees with `compare_versions` on every pair of what it sorts.
        let versions = [
            "1.0", "1.0~rc1", "1.0a", "1.0.0", "", "0", "01", "1.01", "1.1", "a", ".", "~", "6.10",
            "6.6", "6.3+nmu1", "2:1", "10", "9",
        ];
        let list = versions.map(|v| format!("\"{v}\"")).join(", ");
        let text = format!(
            "let sorted = sort_versions([{list}])\n\
             = for i in 1..len(sorted) {{ compare_versions(sorted[i - 1], sorted[i]) }}"
        );
        // All different, so each strictly before the next.
        let expected = format!("[{}]", vec!["-1"; versions.len() - 1].join(", "));
        assert_eq!(lone(&text).unwrap(), expected);
    }

    #[test]
    fn show_gives_the_source_of_a_value() {
        for (text, expected) in [
            // Every kind of value.
            ("= show(1)", "1"),
            ("= show(-5)", "-5"),
            ("= show(9223372036854775807)", "9223372036854775807"),
            ("= show(1.5)", "1.5"),
            ("= show(3.0)", "3.0"),
            ("= show(-0.0)", "-0.0"),
            ("= show(1e300)", "1e300"),
            ("= show(0.000000015)", "1.5e-8"),
            ("= show(true)", "true"),
            ("= show(false)", "false"),
            ("= show(\"a\")", "\"a\""),
            ("= show(\"\")", "\"\""),
            ("= show({})", "{}"),
            ("= show([1, 2])", "[1, 2]"),
            ("let none: List<Int> = []\n= show(none)", "[]"),
            // The lock entry's shape.
            (
                "= show({version = \"6.6\"; commit = \"3a8f\"})",
                "{version = \"6.6\"; commit = \"3a8f\"}",
            ),
            // Nested records and lists, fields in their order.
            (
                "= show({z = {b = [{c = [1.5, 2.0]}]}; a = [[\"x\"], [\"y\", \"z\"]]; e = {}})",
                "{z = {b = [{c = [1.5, 2.0]}]}; a = [[\"x\"], [\"y\", \"z\"]]; e = {}}",
            ),
            (
                "let xs: List<List<Int>> = [[1], []]\n= show(xs)",
                "[[1], []]",
            ),
            // Field names quoted unless identifiers.
            (
                r#"= show({"a b" = 1; "if" = 2; "a.b" = 3; "{{x}}" = 4; "" = 5; ok-name = 6; "\n" = 7})"#,
                r#"{"a b" = 1; "if" = 2; "a.b" = 3; "{{x}}" = 4; "" = 5; ok-name = 6; "\n" = 7}"#,
            ),
            // Every escape, and braces, which are doubled; any other
            // character as it is, a control character too.
            (
                r#"= show("q\"b\\n\nt\tr\r{{x}} }}{{")"#,
                r#""q\"b\\n\nt\tr\r{{x}} }}{{""#,
            ),
            (
                "= show(\"\\u{0}\\u{1b}\\u{7f}\\u{85}\\u{2028}\u{e9}\u{1f600}\")",
                "\"\u{0}\u{1b}\u{7f}\u{85}\u{2028}\u{e9}\u{1f600}\"",
            ),
            // A list of several kinds, and a union.
            (
                "= show([1, \"x\", 2.5, true, {}])",
                "[1, \"x\", 2.5, true, {}]",
            ),
            ("let u: Int | String = \"s\"\n= show(u)", "\"s\""),
            // By name, through `builtins`, an alias, a member import.
            ("= show(value = 1)", "1"),
            ("import builtins\n= builtins.show(true)", "true"),
            ("let s = show\n= s(1)", "1"),
            ("s = show\nv = s([2])", "{s = builtin show; v = \"[2]\"}"),
            ("import show as render from prelude\n= render([1])", "[1]"),
            // A future argument, or one holding futures, awaited.
            ("fn f() -> Int { 3 }\n= show(f())", "3"),
            ("fn f() -> {a: Int} { {a = 1} }\n= show(f())", "{a = 1}"),
            ("= show(lazy {a = lazy 1})", "{a = 1}"),
            ("= show([lazy 1, lazy 2])", "[1, 2]"),
            (
                "let r: {a: Future<Int>} = {a = lazy 7}\n= show(r)",
                "{a = 7}",
            ),
            // In a pure function, of a present value.
            (
                "fn entry(p: {version: String, commit: String}): String { \"    {show(p)},\" }\n\
                 = entry({version = \"1\"; commit = \"c\"})",
                "    {version = \"1\"; commit = \"c\"},",
            ),
            // A file's own declaration shadows the prelude's.
            (
                "def show(n: Int): Int { n }\nv = show(3)",
                "{show = fn(n: Int): Int; v = 3}",
            ),
        ] {
            runs_to(text, expected);
        }
        // An alias of it crosses the boundary of a file, as `len`'s does.
        let files = [
            ("workspace.gcsl", ""),
            (
                "lib/render.gcsl",
                "import show as r from prelude\nlet s = r\nas_text = s",
            ),
            (
                "main.gcsl",
                "import lib.render\nv = render.as_text({n = [1]})",
            ),
        ];
        assert_eq!(value(&files, "main.gcsl"), "{v = \"{{n = [1]}}\"}");
        // `Int`'s least value has no literal: its source does not read back.
        runs_to("= show(-9223372036854775807 - 1)", "-9223372036854775808");
        assert_eq!(
            lone("= -9223372036854775808").unwrap_err(),
            ["integer literal out of range for Int"]
        );
    }

    #[test]
    fn show_cannot_render_nan_or_an_infinity() {
        for (text, shown) in [
            ("= show(0.0 / 0.0)", "nan"),
            ("= show(1.0 / 0.0)", "inf"),
            ("= show(-1.0 / 0.0)", "-inf"),
            ("= show({a = [1.0, 1.0 / 0.0]})", "inf"),
            // The first in the order it is shown.
            ("= show([[2.0], [0.0 / 0.0, 1.0 / 0.0]])", "nan"),
            ("= show({b = -1.0 / 0.0; a = 0.0 / 0.0})", "-inf"),
        ] {
            let error = lone_error(text);
            assert_eq!(
                error.message,
                format!("`show` cannot render `{shown}` as source"),
                "{text}"
            );
            assert_eq!(error.span, Span::new(2, u32::try_from(text.len()).unwrap()));
            assert!(error.labels.is_empty() && error.notes.is_empty(), "{text}");
        }
    }

    /// A deterministic generator of pseudo-random numbers, xorshift64*, for
    /// the round-trip test of `show`.
    struct Rng(u64);

    impl Rng {
        /// The next number.
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        /// A number below `n`.
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
        }

        /// One of `choices`.
        fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
            choices[self.below(choices.len())]
        }
    }

    /// The type of a value the round-trip test of `show` generates.
    enum Shape {
        /// `Int`.
        Int,
        /// `Float`.
        Float,
        /// `Bool`.
        Bool,
        /// `String`.
        Str,
        /// A record type of these fields, in order.
        Record(Vec<(String, Shape)>),
        /// A list type of this element type.
        List(Box<Shape>),
    }

    impl Shape {
        /// A random type, nested at most `depth` deep.
        fn random(rng: &mut Rng, depth: u32) -> Shape {
            match rng.below(if depth == 0 { 4 } else { 6 }) {
                0 => Shape::Int,
                1 => Shape::Float,
                2 => Shape::Bool,
                3 => Shape::Str,
                4 => {
                    let mut fields: Vec<(String, Shape)> = Vec::new();
                    for _ in 0..rng.below(4) {
                        let name = random_name(rng);
                        if fields.iter().all(|(other, _)| *other != name) {
                            fields.push((name, Shape::random(rng, depth - 1)));
                        }
                    }
                    Shape::Record(fields)
                }
                _ => Shape::List(Box::new(Shape::random(rng, depth - 1))),
            }
        }

        /// The type as GCSL writes it, field names as [`FieldName`] shows
        /// them.
        fn text(&self) -> String {
            match self {
                Shape::Int => "Int".to_owned(),
                Shape::Float => "Float".to_owned(),
                Shape::Bool => "Bool".to_owned(),
                Shape::Str => "String".to_owned(),
                Shape::Record(fields) => {
                    let fields: Vec<String> = fields
                        .iter()
                        .map(|(name, shape)| {
                            format!("{}: {}", crate::strings::FieldName(name), shape.text())
                        })
                        .collect();
                    format!("{{{}}}", fields.join(", "))
                }
                Shape::List(element) => format!("List<{}>", element.text()),
            }
        }

        /// A random value of the type as a GCSL literal, written another way
        /// than `show` writes it: every character of a string or a field
        /// name as a `\u{...}` escape, every `Float` in exponent form. Also
        /// whether it holds an empty list, which has no element type to
        /// infer where no type is expected.
        fn literal(&self, rng: &mut Rng) -> (String, bool) {
            match self {
                Shape::Int => {
                    let value = match rng.below(3) {
                        0 => rng.pick(&[0, 1, -1, i64::MAX, i64::MIN + 1, 42, -7]),
                        1 => i64::try_from(rng.below(1000)).unwrap() - 500,
                        // Any but `i64::MIN`, which has no literal.
                        _ => i64::from_ne_bytes(rng.next().to_ne_bytes()).max(i64::MIN + 1),
                    };
                    (value.to_string(), false)
                }
                Shape::Float => {
                    let value = match rng.below(2) {
                        0 => rng.pick(&[
                            0.0,
                            -0.0,
                            1.0,
                            -1.5,
                            0.1 + 0.2,
                            1e300,
                            1e16,
                            1e15,
                            1.5e-7,
                            5e-324,
                            f64::MAX,
                            f64::MIN,
                            f64::MIN_POSITIVE,
                            f64::EPSILON,
                        ]),
                        _ => loop {
                            let value = f64::from_bits(rng.next());
                            if value.is_finite() {
                                break value;
                            }
                        },
                    };
                    let sign = if value.is_sign_negative() { "-" } else { "" };
                    (format!("{sign}{:e}", value.abs()), false)
                }
                Shape::Bool => (rng.pick(&["true", "false"]).to_owned(), false),
                Shape::Str => (escaped(&random_text(rng)), false),
                Shape::Record(fields) => {
                    let mut empty = false;
                    let fields: Vec<String> = fields
                        .iter()
                        .map(|(name, shape)| {
                            let (value, holds) = shape.literal(rng);
                            empty |= holds;
                            format!("{} = {value}", escaped(name))
                        })
                        .collect();
                    (format!("{{{}}}", fields.join("; ")), empty)
                }
                Shape::List(element) => {
                    let count = rng.below(4);
                    let mut empty = count == 0;
                    let elements: Vec<String> = (0..count)
                        .map(|_| {
                            let (value, holds) = element.literal(rng);
                            empty |= holds;
                            value
                        })
                        .collect();
                    (format!("[{}]", elements.join(", ")), empty)
                }
            }
        }
    }

    /// The characters the round-trip test of `show` builds strings of: what
    /// is escaped, braces, control characters, and others.
    const TEXT_CHARS: &str = "aZ0 \"\\{}\n\t\r\0\u{1}\u{1b}\u{7f}\u{80}\u{85}\u{a0}\u{e9}\u{2028}\
                              \u{feff}\u{1f600}#$'-";

    /// A random string, of up to seven characters.
    fn random_text(rng: &mut Rng) -> String {
        let chars: Vec<char> = TEXT_CHARS.chars().collect();
        (0..rng.below(8))
            .map(|_| {
                if rng.below(8) == 0 {
                    // Any character at all.
                    loop {
                        let code = u32::try_from(rng.below(0x11_0000)).unwrap();
                        if let Some(ch) = char::from_u32(code) {
                            break ch;
                        }
                    }
                } else {
                    rng.pick(&chars)
                }
            })
            .collect()
    }

    /// A random field name: an identifier, a keyword, or any text.
    fn random_name(rng: &mut Rng) -> String {
        match rng.below(3) {
            0 => rng
                .pick(&["a", "b", "x1", "_", "ok-name", "version", "commit"])
                .to_owned(),
            1 => rng
                .pick(&["if", "let", "true", "fn", "a b", "", "1x", "a.b", "a-"])
                .to_owned(),
            _ => random_text(rng),
        }
    }

    /// `text` as a GCSL string literal of `\u{...}` escapes alone.
    fn escaped(text: &str) -> String {
        let mut literal = String::from("\"");
        for ch in text.chars() {
            std::fmt::Write::write_fmt(&mut literal, format_args!("\\u{{{:x}}}", u32::from(ch)))
                .unwrap();
        }
        literal.push('"');
        literal
    }

    /// The fields of the record that `text`, run as a lone file, evaluates
    /// to, by name.
    fn lone_fields(text: &str) -> Vec<(String, Value)> {
        let source = SourceFile::new("lone.gcsl", text).unwrap();
        let outcome = crate::run(&source).unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
        let Value::Record(record) = outcome.value else {
            panic!("{text}: not a record");
        };
        record
            .layout
            .texts
            .iter()
            .map(ToString::to_string)
            .zip(record.values.iter().cloned())
            .collect()
    }

    /// The text of the string `value`.
    fn text_of(value: &Value) -> String {
        let Value::Str(text) = value else {
            panic!("not a string: {value}");
        };
        text.to_string()
    }

    #[test]
    fn what_show_gives_reads_back_as_an_equal_value() {
        let mut rng = Rng(0x5eed_0043_1ab5_c0de);
        // How many values held a record, a string, and no empty list: that
        // the generator is not degenerate.
        let (mut records, mut strings, mut bare) = (0, 0, 0);
        for _ in 0..300 {
            let shape = Shape::random(&mut rng, 3);
            let ty = shape.text();
            let (literal, holds_empty) = shape.literal(&mut rng);
            records += usize::from(ty.contains('{'));
            strings += usize::from(literal.contains("\\u{"));
            bare += usize::from(!holds_empty);
            // The value, written without `show`'s help, and its source.
            let fields = lone_fields(&format!("v: {ty} = {literal}\ns = show(v)"));
            let source = text_of(&fields[1].1);
            // Read back where its type is expected, it is an equal value,
            // whose source is the same text, `-0.0` and every digit of a
            // `Float` included.
            let text = format!(
                "a: {ty} = {literal}\n\
                 b: {ty} = {source}\n\
                 same = a == b\n\
                 again = show(b)"
            );
            let back = lone_fields(&text);
            assert_eq!(back[2].1, Value::Bool(true), "{text}");
            assert_eq!(text_of(&back[3].1), source, "{text}");
            // And where none is, unless an empty list's element type is left
            // to infer.
            if !holds_empty {
                let text = format!("a: {ty} = {literal}\nc = {source}\nsame = a == c");
                let back = lone_fields(&text);
                assert_eq!(back[2].1, Value::Bool(true), "{text}");
            }
        }
        assert!(
            records > 50 && strings > 50 && bare > 100,
            "{records} {strings} {bare}"
        );
    }

    /// The files of the summary of slice 31, in path order.
    const DEFAULTS: [(&str, &str); 3] = [
        (
            "_lib/defaults.gcsl",
            "#[shadowable]\narch = \"x86_64\"\n#[shadowable] jobs = 4",
        ),
        (
            "pkg/build.gcsl",
            "import * from _lib.defaults\narch = \"aarch64\"          # shadows the default\n\
             j = jobs                   # 4",
        ),
        ("workspace.gcsl", ""),
    ];

    #[test]
    fn the_shadowable_summary_runs() {
        assert_eq!(
            value(&DEFAULTS, "pkg/build.gcsl"),
            "{arch = \"aarch64\"; j = 4}"
        );
        assert_eq!(
            value(&DEFAULTS, "_lib/defaults.gcsl"),
            "{arch = \"x86_64\"; jobs = 4}"
        );
        // The implicit prelude is the file's too, and a name of it is
        // shadowed by a declaration as before.
        let mut files = DEFAULTS.to_vec();
        files[1].1 = "import * from _lib.defaults\narch = \"aarch64\"\nn = len([jobs])\n\
                      trim = 3";
        assert_eq!(
            value(&files, "pkg/build.gcsl"),
            "{arch = \"aarch64\"; n = 1; trim = 3}"
        );
        // Without its mark, the import's `arch` conflicts, as it always has.
        files[0].1 = "arch = \"x86_64\"\n#[shadowable] jobs = 4";
        assert_eq!(
            shown_error(&files),
            "field `arch` is defined in two files at pkg/build.gcsl (imported here); \
             _lib/defaults.gcsl: defined here; pkg/build.gcsl: defined here"
        );
    }

    #[test]
    fn within_one_file_the_last_declaration_wins_for_the_whole_file() {
        for (text, expected) in [
            // A read above it sees it too.
            (
                "x = a\n#[shadowable] a = 1\na = 2\ny = a",
                "{x = 2; a = 2; y = 2}",
            ),
            // The last of several shadowable ones.
            (
                "#[shadowable] a = 1\n#[shadowable] a = 2\nx = a\n#[shadowable] a = 3",
                "{x = 3; a = 3}",
            ),
            // A `def` by a field, and a field by a `def`: the winner's type.
            (
                "#[shadowable] def f(): Int { 1 }\nf = 2\nx = f + 1",
                "{f = 2; x = 3}",
            ),
            (
                "#[shadowable] f = 2\ndef f(n: Int): Int { n + 1 }\nx = f(1)",
                "{f = fn(n: Int): Int; x = 2}",
            ),
            // A function reads the winner too.
            (
                "fn get(): Int { a }\n#[shadowable] a = 1\na = 5\nx = get()",
                "{a = 5; x = 5}",
            ),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // A shadowing declaration cannot read the value it shadows.
        assert_eq!(
            lone("#[shadowable] a = 1\na = a + 1").unwrap_err(),
            ["`a` depends on itself"]
        );
        // A plain one is shadowed by nothing, and a plain one followed by a
        // shadowable one is the conflict it always was.
        for text in ["a = 1\na = 2", "a = 1\n#[shadowable] a = 2"] {
            assert_eq!(
                lone(text).unwrap_err(),
                ["field `a` is defined twice"],
                "{text}"
            );
        }
    }

    #[test]
    fn a_shadowed_declaration_is_checked_but_never_run() {
        // Its own errors are reported.
        assert_eq!(
            lone("#[shadowable] a = 1 + \"x\"\na = 2").unwrap_err(),
            ["cannot add Int and String"]
        );
        // It runs nothing.
        assert_eq!(
            lone("#[shadowable] a = panic(\"never\")\na = 2").unwrap(),
            "{a = 2}"
        );
        // It needs nothing: here it alone reads `lib/x.gcsl`, which is not
        // run, and whose cell it sees as erroneous, without an error.
        let files = [
            (
                "a.gcsl",
                "import * from lib.x\n#[shadowable] arch = thing + 1\narch = 2",
            ),
            ("lib/x.gcsl", "thing = panic(\"never run\")"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a.gcsl"), "{arch = 2}");
        let scan = ws(&files).unwrap();
        let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
        assert!(scan.needs[a].is_empty(), "{:?}", scan.needs[a]);
        // Nor is it in a cycle.
        assert_eq!(
            lone("#[shadowable] a = b\nb = a\na = 1").unwrap(),
            "{b = 1; a = 1}"
        );
    }

    #[test]
    fn across_the_files_of_a_directory_in_path_order() {
        let files = [
            ("a.gcsl", "#[shadowable] arch = \"x86_64\"\nmine = arch"),
            ("b.gcsl", "arch = \"aarch64\""),
            ("c.gcsl", "got = arch"),
            ("workspace.gcsl", ""),
        ];
        // Every file of the directory agrees, the shadowed one included,
        // whose record no longer has it.
        assert_eq!(value(&files, "c.gcsl"), "{got = \"aarch64\"}");
        assert_eq!(value(&files, "a.gcsl"), "{mine = \"aarch64\"}");
        assert_eq!(value(&files, "b.gcsl"), "{arch = \"aarch64\"}");
        // The reverse conflicts.
        let files = [
            ("a.gcsl", "arch = \"x86_64\""),
            ("b.gcsl", "#[shadowable] arch = \"aarch64\""),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            shown_error(&files),
            "field `arch` is defined in two files at b.gcsl (defined here); \
             a.gcsl: defined here"
        );
    }

    #[test]
    fn an_inner_directory_shadows_an_outer_one() {
        let files = [
            ("other.gcsl", "got = arch"),
            ("sub/s.gcsl", "arch = \"aarch64\"\nmine = arch"),
            ("sub/t.gcsl", "seen = arch"),
            ("workspace.gcsl", "#[shadowable] arch = \"x86_64\""),
        ];
        assert_eq!(value(&files, "other.gcsl"), "{got = \"x86_64\"}");
        assert_eq!(
            value(&files, "sub/s.gcsl"),
            "{arch = \"aarch64\"; mine = \"aarch64\"}"
        );
        assert_eq!(value(&files, "sub/t.gcsl"), "{seen = \"aarch64\"}");
        // The reverse conflicts, with its note, reported as ever at the
        // later of the two in the order of conflicts (the file's own
        // directory first, then upward).
        let files = [
            ("sub/s.gcsl", "#[shadowable] arch = \"aarch64\""),
            ("workspace.gcsl", "arch = \"x86_64\""),
        ];
        assert_eq!(
            shown_error(&files),
            "field `arch` is defined in two files at workspace.gcsl (defined here); \
             sub/s.gcsl: defined here; note: one is in a directory above"
        );
    }

    #[test]
    fn an_import_and_a_field_shadow_in_their_order() {
        // A field after an import of a shadowable leaf: the field wins.
        let mut files = DEFAULTS.to_vec();
        files[1].1 = "import arch from _lib.defaults\narch = \"own\"\ngot = arch";
        assert_eq!(
            value(&files, "pkg/build.gcsl"),
            "{arch = \"own\"; got = \"own\"}"
        );
        // An import after a shadowable field: the import wins in this
        // file's view, which is its own; the field is still its
        // directory's, which the other files of the directory see.
        let files = [
            ("lib/defs.gcsl", "arch = \"lib\""),
            (
                "pkg/a.gcsl",
                "#[shadowable] arch = \"own\"\nimport arch from lib.defs\ngot = arch",
            ),
            ("pkg/b.gcsl", "seen = arch"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "pkg/a.gcsl"),
            "{arch = \"own\"; got = \"lib\"}"
        );
        assert_eq!(value(&files, "pkg/b.gcsl"), "{seen = \"own\"}");
        // A plain field before the import conflicts, as it always has.
        let files = [
            ("lib/defs.gcsl", "arch = \"lib\""),
            ("pkg/a.gcsl", "arch = \"own\"\nimport arch from lib.defs"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            shown_error(&files),
            "field `arch` is defined in two files at pkg/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; pkg/a.gcsl: defined here"
        );
    }

    #[test]
    fn a_marked_import_brings_shadowable_leaves() {
        let files = [
            (
                "a/a.gcsl",
                "#[shadowable] import * from lib.defs\narch = \"own\"\nj = jobs",
            ),
            (
                "b/b.gcsl",
                "#[shadowable] import arch, jobs as n from lib.defs\narch = 1\nn = 3",
            ),
            ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"own\"; j = 2}");
        assert_eq!(value(&files, "b/b.gcsl"), "{arch = 1; n = 3}");
        // Unmarked, the same is the conflict it always was.
        let files = [
            ("a/a.gcsl", "import * from lib.defs\narch = \"own\""),
            ("lib/defs.gcsl", "arch = \"lib\""),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            shown_error(&files),
            "field `arch` is defined in two files at a/a.gcsl (imported here); \
             lib/defs.gcsl: defined here; a/a.gcsl: defined here"
        );
        // A `let` or a `fn` of its name shadows it silently, from its item
        // on, or in the whole file.
        let files = [
            ("lib/defs.gcsl", "arch = \"lib\"\njobs = 2"),
            (
                "pkg/a.gcsl",
                "#[shadowable] import * from lib.defs\nbefore = arch\nlet arch = 1\n\
                 after = arch\nfn jobs(): Int { 9 }\nj = jobs()",
            ),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "pkg/a.gcsl"),
            "{before = \"lib\"; after = 1; j = 9}"
        );
    }

    #[test]
    fn shadowability_travels_with_a_leaf() {
        // Through `*` from a file, a member, a directory's merged tree and
        // a directory above.
        let files = [
            ("_lib/defaults.gcsl", "#[shadowable] arch = \"x86_64\""),
            ("_lib/more.gcsl", "jobs = 4"),
            ("a/a.gcsl", "import * from _lib.defaults\narch = \"a\""),
            ("b/b.gcsl", "import arch from _lib.defaults\narch = \"b\""),
            ("c/c.gcsl", "import * from _lib\narch = \"c\"\nj = jobs"),
            ("d/d.gcsl", "import _lib\narch = \"d\"\nx = _lib.arch"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a/a.gcsl"), "{arch = \"a\"}");
        assert_eq!(value(&files, "b/b.gcsl"), "{arch = \"b\"}");
        assert_eq!(value(&files, "c/c.gcsl"), "{arch = \"c\"; j = 4}");
        assert_eq!(value(&files, "d/d.gcsl"), "{arch = \"d\"; x = \"x86_64\"}");
        // And to the directories below its own, which see it in the
        // directory's merged tree.
        let files = [
            ("e/e.gcsl", "#[shadowable] arch = \"e\"\ny = arch"),
            ("e/f/f.gcsl", "arch = \"f\"\nz = arch"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "e/f/f.gcsl"), "{arch = \"f\"; z = \"f\"}");
        assert_eq!(value(&files, "e/e.gcsl"), "{arch = \"e\"; y = \"e\"}");
    }

    #[test]
    fn dotted_leaves_are_shadowed_leaf_by_leaf() {
        // Two files of one directory: the later leaf wins, the others of
        // the node merge as before.
        let files = [
            ("a.gcsl", "#[shadowable] com.x.port = 1\ncom.x.host = \"h\""),
            ("b.gcsl", "com.x.port = 2"),
            ("c.gcsl", "all = com"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "c.gcsl"),
            "{all = {x = {host = \"h\"; port = 2}}}"
        );
        // Through an import.
        let files = [
            (
                "lib/defs.gcsl",
                "#[shadowable] com.x.port = 1\ncom.x.host = \"h\"",
            ),
            (
                "pkg/a.gcsl",
                "import * from lib.defs\ncom.x.port = 2\nall = com",
            ),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(
            value(&files, "pkg/a.gcsl"),
            "{com = {x = {port = 2}}; all = {x = {port = 2; host = \"h\"}}}"
        );
    }

    #[test]
    fn a_declaration_overlapping_shadowable_ones_shadows_them_all() {
        for (text, expected) in [
            // At, above and below its path.
            ("#[shadowable] a.x = 1\na = {z = 3}", "{a = {z = 3}}"),
            ("#[shadowable] d = {j = 4; k = 5}\nd.j = 8", "{d = {j = 8}}"),
            (
                "#[shadowable] a.x.y = 1\n#[shadowable] a.w = 2\na.x = 3",
                "{a = {w = 2; x = 3}}",
            ),
            // Not overlapping: they merge as before.
            ("#[shadowable] a.x = 1\na.y = 2", "{a = {x = 1; y = 2}}"),
        ] {
            assert_eq!(lone(text).unwrap(), expected, "{text}");
            let files = [("workspace.gcsl", ""), ("main.gcsl", text)];
            assert_eq!(value(&files, "main.gcsl"), expected, "{text}");
        }
        // If one it overlaps is not shadowable, the conflict as today.
        assert_eq!(
            lone("#[shadowable] a.x = 1\na.y = 2\na = {}").unwrap_err(),
            ["field `a` is defined twice"]
        );
        assert_eq!(
            lone("d = {j = 4}\nd.j = 8").unwrap_err(),
            ["field `d` is already defined"]
        );
        // Across files and directories, as within one.
        let files = [
            ("a.gcsl", "#[shadowable] d = {j = 4; k = 5}"),
            ("b.gcsl", "d.j = 8"),
            ("c.gcsl", "got = d"),
            ("sub/s.gcsl", "a = 1\nseen = d"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "c.gcsl"), "{got = {j = 8}}");
        assert_eq!(value(&files, "sub/s.gcsl"), "{a = 1; seen = {j = 8}}");
        let files = [
            ("sub/s.gcsl", "a = {z = 3}\ngot = a"),
            ("workspace.gcsl", "#[shadowable] a.x = 1\nseen = a"),
        ];
        assert_eq!(value(&files, "sub/s.gcsl"), "{a = {z = 3}; got = {z = 3}}");
        assert_eq!(
            value(&files, "workspace.gcsl"),
            "{a = {x = 1}; seen = {x = 1}}"
        );
    }

    #[test]
    fn a_dotted_root_shadows_the_prelude_s_name_as_before() {
        // The five cases the old rule's drop by root name shadowed: each
        // is its result before this slice.
        assert_eq!(
            lone("len.x = 1\ny = len").unwrap(),
            "{len = {x = 1}; y = {x = 1}}"
        );
        let files = [
            ("app/main.gcsl", "import * from lib.a\ny = len"),
            ("lib/a.gcsl", "len.x = 1"),
            ("workspace.gcsl", "x = 1"),
        ];
        assert_eq!(value(&files, "app/main.gcsl"), "{y = {x = 1}}");
        let files = [
            ("app/main.gcsl", "s = join\nn = len([1, 2])"),
            ("workspace.gcsl", "join.sep = \",\""),
        ];
        assert_eq!(value(&files, "app/main.gcsl"), "{s = {sep = \",\"}; n = 2}");
        let files = [
            ("a.gcsl", "len.x = 1"),
            ("b.gcsl", "y = len\nparts = split(\"a,b\", \",\")"),
            ("workspace.gcsl", "x = 1"),
        ];
        assert_eq!(
            value(&files, "b.gcsl"),
            "{y = {x = 1}; parts = [\"a\", \"b\"]}"
        );
        // (The REPL's is `crate::session`'s.) An explicit import is plain,
        // and still conflicts.
        assert_eq!(
            lone("import * from prelude\nlen.x = 1").unwrap_err(),
            ["field `len` is already defined"]
        );
    }

    #[test]
    fn no_prelude_turns_the_implicit_import_off() {
        assert_eq!(
            lone("#![no_prelude]\nn = len([1])").unwrap_err(),
            ["undefined variable `len`"]
        );
        assert_eq!(
            lone("# a comment\n#![no_prelude]\nimport len from builtins\nn = len([1])").unwrap(),
            "{n = 1}"
        );
        let files = [
            ("main.gcsl", "#![no_prelude]\nn = trim(\" a \")"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(messages(&files, "main.gcsl"), ["undefined variable `trim`"]);
        // The prelude has it, and so no import of its own; every other
        // file, the standard ones included, has the implicit one.
        let scan = ws(&[("workspace.gcsl", ""), ("main.gcsl", "import path")]).unwrap();
        let prelude = scan.standard_file(PRELUDE).unwrap();
        let path = scan.standard_file("path").unwrap();
        let files = &scan.files;
        assert!(files[prelude as usize].ast.no_prelude().is_some());
        assert!(files[prelude as usize].ast.implicit_import().is_none());
        assert!(files[prelude as usize].aliases.iter().all(|a| !a.implicit));
        for file in [path, 2, 3] {
            let file = &files[file as usize];
            assert!(file.ast.implicit_import().is_some(), "{}", file.path);
            assert!(file.aliases.iter().any(|a| a.implicit), "{}", file.path);
        }
    }

    #[test]
    fn the_clash_family_ignores_shadowable_leaves() {
        // A `let`, a `fn`, a `def`, a field or a whole import of the name
        // of a shadowable field of a directory above shadows it silently.
        let files = [
            ("a/a.gcsl", "let port = 5\nn = port"),
            ("b/b.gcsl", "fn port(): Int { 7 }\nn = port()"),
            ("c/c.gcsl", "def port(): Int { 8 }\nn = port()"),
            ("d/d.gcsl", "import lib.port\nn = port.x"),
            ("e/e.gcsl", "port = 6\nn = port"),
            ("lib/port.gcsl", "x = 9"),
            ("workspace.gcsl", "#[shadowable] port = 1"),
        ];
        assert_eq!(value(&files, "a/a.gcsl"), "{n = 5}");
        assert_eq!(value(&files, "b/b.gcsl"), "{n = 7}");
        assert_eq!(value(&files, "c/c.gcsl"), "{port = fn(): Int; n = 8}");
        assert_eq!(value(&files, "d/d.gcsl"), "{n = 9}");
        assert_eq!(value(&files, "e/e.gcsl"), "{port = 6; n = 6}");
        // Within one file, a `let` from its item on and a `fn` in the whole
        // file; the field is still the file's.
        assert_eq!(
            lone("#[shadowable] a = 1\nx = a\nlet a = 2\ny = a").unwrap(),
            "{a = 1; x = 1; y = 2}"
        );
        assert_eq!(
            lone("x = a()\n#[shadowable] a = 1\nfn a(): Int { 3 }").unwrap(),
            "{x = 3; a = 1}"
        );
        // Plain, they clash as they always have.
        let files = [
            ("sub/a.gcsl", "let port = 5"),
            ("workspace.gcsl", "port = 1"),
        ];
        assert_eq!(
            shown_checked_error(&files),
            "`port` is a field of this directory"
        );
        assert_eq!(
            lone("a = 1\nlet a = 2").unwrap_err(),
            ["`a` is a field of this file"]
        );
    }

    #[test]
    fn a_declaration_a_later_file_shadows_is_still_checked_and_its_file_s() {
        // Its own error, against its own file.
        let sources: Vec<SourceFile> = [
            ("a.gcsl", "#[shadowable] arch = 1 + \"x\""),
            ("b.gcsl", "arch = 2"),
            ("workspace.gcsl", ""),
        ]
        .iter()
        .map(|&(path, text)| SourceFile::new(path, text).unwrap())
        .collect();
        let errors = check(&sources).unwrap_err();
        let scan = ws(&[("a.gcsl", ""), ("workspace.gcsl", "")]).unwrap();
        let a = scan.files.iter().position(|f| f.path == "a.gcsl").unwrap();
        let shown: Vec<(&str, Option<u32>)> = errors
            .iter()
            .map(|e| (e.message.as_str(), e.input))
            .collect();
        assert_eq!(shown, [("cannot add Int and String", Some(count(a)))]);
        // A `let` that shadows a file's own shadowable field is the file's
        // alone: the directory has the field.
        let files = [
            ("a.gcsl", "#[shadowable] a = 1\nlet a = 2\nmine = a"),
            ("b.gcsl", "x = a"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "b.gcsl"), "{x = 1}");
        assert_eq!(value(&files, "a.gcsl"), "{a = 1; mine = 2}");
        // A `fn` shadows a dotted shadowable root of its file in the whole
        // file; the root is still its field, and its directory's.
        let files = [
            (
                "a.gcsl",
                "#[shadowable] a.x = 1\nfn a(): Int { 3 }\nb = a()",
            ),
            ("c.gcsl", "y = a.x"),
            ("workspace.gcsl", ""),
        ];
        assert_eq!(value(&files, "a.gcsl"), "{a = {x = 1}; b = 3}");
        assert_eq!(value(&files, "c.gcsl"), "{y = 1}");
    }

    /// The messages of the errors of checking `files`, joined.
    fn shown_checked_error(files: &[(&str, &str)]) -> String {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let errors = check(&sources).unwrap_err();
        let shown: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        shown.join("; ")
    }

    #[test]
    fn an_input_cannot_turn_the_prelude_off() {
        let started = start("/").unwrap();
        let (mut names, mut types) = (started.names, started.types);
        let source = SourceFile::new("input", "#![no_prelude]\nx = 1").unwrap();
        let earlier = Earlier {
            imports: &[],
            types: &[],
            store: &mut TypeStore::new(),
            def_types: &[],
        };
        let errors = check_input(
            &started.standard,
            (&source, earlier),
            (&mut names, &mut types),
            Scope::of_slots(started.slot_names),
            2,
        )
        .unwrap_err();
        let shown: Vec<(String, Span, Option<u32>)> = errors
            .into_iter()
            .map(|e| (e.message, e.span, e.input))
            .collect();
        assert_eq!(
            shown,
            [(
                "`#![no_prelude]` applies only to a file".to_owned(),
                Span::new(0, 14),
                None
            )]
        );
    }

    /// The root the `fileroot` tests' workspaces are located at: a path in
    /// the scratch directory, which nothing reads, since the library reads
    /// no file.
    const SCRATCH: &str = "/scratch/fileroot";

    /// The library of the `fileroot` summary, `_lib/ppk.gcsl`, with what the
    /// other tests call besides: `commit`, `wrapped` (a helper), `twice`
    /// (a helper of a helper), `here` (`this` in a `def`), `caller` (the
    /// caller of a `def` of the library), `mid` (`helper()` between two
    /// calls), closures made in the library, and a step made in it.
    const PPK: &str = "import path\nimport fileroot\n\
        def commit(content: String, output: String) -> String {\n\
            let content = path.join(fileroot.caller, content)\n\
            let output = path.join(fileroot.workspace, output)\n\
            \"{content} {output}\"\n\
        }\n\
        def wrapped(content: String) -> String {\n\
            fileroot.helper()\n\
            commit(content, \"dist\")\n\
        }\n\
        def twice(content: String) -> String {\n\
            fileroot.helper()\n\
            wrapped(content)\n\
        }\n\
        def here(): String { fileroot.this }\n\
        def caller(): String { fileroot.caller }\n\
        def mid(): {before: String, after: String} {\n\
            let first = caller()\n\
            fileroot.helper()\n\
            let second = caller()\n\
            { before = first; after = second }\n\
        }\n\
        def reader(): fn(): String { fn(): String { fileroot.caller } }\n\
        def calling(): fn(): String { fn(): String { caller() } }\n\
        def call(f: fn(): String): String { f() }\n\
        def step(): {s: Future<String>} { s = lazy { fileroot.caller } }\n\
        def force(f: Future<String>) -> String { await f }";

    /// The workspace of `files`, each a path and a text, located at
    /// [`SCRATCH`], checked.
    fn rooted(files: &[(&str, &str)]) -> Result<Workspace, Vec<Diagnostic>> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        check_with_standard(&sources, &[], SCRATCH).1
    }

    /// The value of the file at `path` of the workspace of `files` located
    /// at [`SCRATCH`].
    fn rooted_value(files: &[(&str, &str)], path: &str) -> Value {
        let mut workspace = rooted(files).unwrap_or_else(|errors| panic!("{path}: {errors:?}"));
        assert_imports_stored(&workspace);
        assert_assemblies_stored(&workspace);
        let index = workspace.file(path).unwrap();
        match workspace.run(index) {
            Ok(outcome) => outcome.value,
            Err(errors) => panic!("{path}: {errors:?}"),
        }
    }

    /// [`rooted_value`] shown, [`SCRATCH`] shown as `<root>`.
    fn rooted_text(files: &[(&str, &str)], path: &str) -> String {
        rooted_value(files, path)
            .to_string()
            .replace(SCRATCH, "<root>")
    }

    /// The messages of the diagnostics of checking the workspace of `files`
    /// located at [`SCRATCH`].
    fn rooted_messages(files: &[(&str, &str)]) -> Vec<String> {
        match rooted(files) {
            Ok(_) => panic!("{files:?}: no error"),
            Err(errors) => errors.into_iter().map(|error| error.message).collect(),
        }
    }

    #[test]
    fn the_fileroot_summary_runs_in_a_workspace() {
        let files = [
            ("workspace.gcsl", ""),
            ("_lib/ppk.gcsl", PPK),
            (
                "pkg/test.gcsl",
                "import _lib.ppk\nimport fileroot\n\
                 a = ppk.commit(\"usr\", \"dist\")\n\
                 b = ppk.wrapped(\"usr\")\n\
                 here = fileroot.this",
            ),
        ];
        // The paths are absolute, from the root the driver was given.
        assert_eq!(
            rooted_value(&files, "pkg/test.gcsl").to_string(),
            "{a = \"/scratch/fileroot/pkg/usr /scratch/fileroot/dist\"; \
             b = \"/scratch/fileroot/pkg/usr /scratch/fileroot/dist\"; \
             here = \"/scratch/fileroot/pkg\"}"
        );
        // `this` and `workspace` are constants of the file, `caller` and
        // `helper()` nodes of their own, and each file's code carries its
        // directory, by its index in the run's table, the root's first.
        let mut workspace = rooted(&files).unwrap();
        let (test, lib) = (
            workspace.file("pkg/test.gcsl").unwrap(),
            workspace.file("_lib/ppk.gcsl").unwrap(),
        );
        let test_ir = workspace.dump_ir(test).unwrap();
        assert!(
            test_ir.contains("(store here#") && test_ir.contains("(str \"/scratch/fileroot/pkg\""),
            "{test_ir}"
        );
        let lib_ir = workspace.dump_ir(lib).unwrap();
        assert!(lib_ir.contains("(caller @"), "{lib_ir}");
        assert!(lib_ir.contains("(helper @"), "{lib_ir}");
        assert!(lib_ir.contains("(str \"/scratch/fileroot\" @"), "{lib_ir}");
        let dirs: Vec<&str> = workspace.scan.dir_texts.iter().map(|d| &**d).collect();
        assert_eq!(dirs[0], SCRATCH);
        for (file, dir) in [
            (test, "/scratch/fileroot/pkg"),
            (lib, "/scratch/fileroot/_lib"),
        ] {
            let ir = workspace.irs[file as usize].as_ref().unwrap();
            assert_eq!(dirs[ir.dir() as usize], dir);
        }
        // The same, run from a file of the root itself.
        let files = [
            ("workspace.gcsl", ""),
            ("_lib/ppk.gcsl", PPK),
            (
                "top.gcsl",
                "import _lib.ppk\na = ppk.commit(\"usr\", \"dist\")",
            ),
        ];
        assert_eq!(
            rooted_text(&files, "top.gcsl"),
            "{a = \"<root>/usr <root>/dist\"}"
        );
    }

    #[test]
    fn this_and_caller_through_calls_and_helpers() {
        let files = [
            ("workspace.gcsl", ""),
            ("_lib/ppk.gcsl", PPK),
            (
                "pkg/test.gcsl",
                "import _lib.ppk\nimport fileroot\n\
                 top = fileroot.caller\n\
                 this = fileroot.this\n\
                 lib = ppk.here()\n\
                 deep = ppk.caller()\n\
                 one = ppk.wrapped(\"usr\")\n\
                 two = ppk.twice(\"usr\")\n\
                 mid = ppk.mid()\n\
                 fn mine(): String { fileroot.caller }\n\
                 own = mine()",
            ),
        ];
        assert_eq!(
            rooted_text(&files, "pkg/test.gcsl"),
            "{top = \"<root>/pkg\"; this = \"<root>/pkg\"; lib = \"<root>/_lib\"; \
             deep = \"<root>/pkg\"; one = \"<root>/pkg/usr <root>/dist\"; \
             two = \"<root>/pkg/usr <root>/dist\"; \
             mid = {before = \"<root>/_lib\"; after = \"<root>/pkg\"}; own = \"<root>/pkg\"}"
        );
    }

    #[test]
    fn closures_and_lazies_carry_their_files() {
        let files = [
            ("workspace.gcsl", ""),
            ("_lib/ppk.gcsl", PPK),
            (
                "pkg/make.gcsl",
                "import _lib.ppk\nimport fileroot\n\
                 fn mine(): String { fileroot.caller }\n\
                 reads = ppk.reader()()\n\
                 calls = ppk.calling()()\n\
                 passed = ppk.call(mine)\n\
                 made = ppk.step()\n\
                 written = lazy { ppk.caller() }",
            ),
            (
                "other/await.gcsl",
                "import pkg.make\nimport _lib.ppk\n\
                 made = await make.made.s\n\
                 written = await make.written\n\
                 forced = ppk.force(ppk.step().s)",
            ),
        ];
        // A closure of the library called from `pkg` has `pkg` as its
        // caller, and calls with `_lib`, its code's; a closure of `pkg`
        // called by the library has `_lib`.
        let made = rooted_text(&files, "pkg/make.gcsl");
        assert!(
            made.starts_with(
                "{reads = \"<root>/pkg\"; calls = \"<root>/_lib\"; passed = \"<root>/_lib\""
            ),
            "{made}"
        );
        // A `lazy` runs as if where it was written, with the caller of the
        // frame that made it, whoever awaits it.
        assert_eq!(
            rooted_text(&files, "other/await.gcsl"),
            "{made = \"<root>/pkg\"; written = \"<root>/pkg\"; forced = \"<root>/other\"}"
        );
    }

    #[test]
    fn the_workspace_is_the_root_wherever_the_file_is() {
        let files = [
            ("workspace.gcsl", ""),
            (
                "a/b/c.gcsl",
                "import fileroot\nws = fileroot.workspace\nhere = fileroot.this",
            ),
        ];
        assert_eq!(
            rooted_value(&files, "a/b/c.gcsl").to_string(),
            "{ws = \"/scratch/fileroot\"; here = \"/scratch/fileroot/a/b\"}"
        );
        // A lone file is the workspace of its own directory, as the driver
        // was told it.
        let source = SourceFile::new(
            "lone.gcsl",
            "import fileroot as fr\nws = fr.workspace\nhere = fr.this\ntop = fr.caller",
        )
        .unwrap();
        let (_, checked) = check_lone_with_standard(&source, "/scratch/alone");
        let mut workspace = checked.unwrap();
        let index = workspace.file("lone.gcsl").unwrap();
        assert_eq!(
            workspace.run(index).unwrap().value.to_string(),
            "{ws = \"/scratch/alone\"; here = \"/scratch/alone\"; top = \"/scratch/alone\"}"
        );
        // A root of `/` takes no second `/`.
        let source =
            SourceFile::new("lone.gcsl", "import fileroot\nws = fileroot.workspace").unwrap();
        let (_, checked) = check_lone_with_standard(&source, "/");
        let mut workspace = checked.unwrap();
        let index = workspace.file("lone.gcsl").unwrap();
        assert_eq!(
            workspace.run(index).unwrap().value.to_string(),
            "{ws = \"/\"}"
        );
        let files = [
            ("workspace.gcsl", ""),
            ("sub/a.gcsl", "import fileroot\nh = fileroot.this"),
        ];
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let mut workspace = check_with_standard(&sources, &[], "/").1.unwrap();
        let index = workspace.file("sub/a.gcsl").unwrap();
        assert_eq!(
            workspace.run(index).unwrap().value.to_string(),
            "{h = \"/sub\"}"
        );
    }

    #[test]
    fn this_in_a_standard_file_is_the_standard_directory() {
        let table = [
            (
                "here",
                "import fileroot\nwhere = fileroot.this\ndef call(): String { fileroot.caller }",
            ),
            prelude_entry(),
        ];
        let files = [
            ("workspace.gcsl", ""),
            (
                "app/main.gcsl",
                "import here\nw = here.where\nc = here.call()",
            ),
        ];
        let (_, value) = value_beside(&table, &files, "app/main.gcsl");
        assert_eq!(value, "{w = \"<stdlib>\"; c = \"/app\"}");
    }

    #[test]
    fn fileroot_is_read_only_as_its_members() {
        let with = |text: &str| rooted_messages(&[("workspace.gcsl", ""), ("a.gcsl", text)]);
        let read_as =
            |member: &str| format!("`fileroot`'s members are read as `fileroot.{member}`");
        assert_eq!(with("import caller from fileroot"), [read_as("caller")]);
        assert_eq!(with("import this as t from fileroot"), [read_as("this")]);
        assert_eq!(with("import * from fileroot"), [read_as("caller")]);
        assert_eq!(
            with("import nope from fileroot"),
            ["`fileroot` has no `nope`"]
        );
        assert_eq!(with("import fileroot\nx = fileroot"), [read_as("caller")]);
        assert_eq!(
            with("import fileroot as fr\nx = fr"),
            ["`fr`'s members are read as `fr.caller`"]
        );
        assert_eq!(
            with("import fileroot\nx = (fileroot).this"),
            [read_as("caller")]
        );
        assert_eq!(
            with("import fileroot\nx = fileroot.helper"),
            ["`fileroot.helper` is not a value"]
        );
        assert_eq!(
            with("import fileroot\nx = (fileroot.helper)()"),
            ["`fileroot.helper` is not a value"]
        );
        assert_eq!(
            with("import fileroot\nx = fileroot.helper(1)"),
            ["expected 0 arguments, found 1"]
        );
        assert_eq!(
            with("import fileroot\nx = fileroot.this(1)"),
            ["cannot call a value of type String"]
        );
        assert_eq!(
            with("import fileroot\nx = fileroot.nope"),
            ["`fileroot` has no `nope`"]
        );
        assert_eq!(
            with("import fileroot\nx = fileroot.caller.x"),
            ["cannot access field `x` of String"]
        );
        // Not imported, it is no name; a nearer binding shadows it.
        assert_eq!(with("x = fileroot.this"), ["undefined variable `fileroot`"]);
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "import fileroot\nfn f(fileroot: {this: Int}): Int { fileroot.this }\nx = f({ this = 1 })",
            ),
        ];
        assert_eq!(rooted_text(&files, "a.gcsl"), "{x = 1}");
        // `helper()` is pure, and does nothing at the top level.
        let files = [
            ("workspace.gcsl", ""),
            ("_lib/ppk.gcsl", PPK),
            (
                "a.gcsl",
                "import fileroot\nimport _lib.ppk\nfileroot.helper()\nx = ppk.caller()\n\
                 fn pure(): String { fileroot.helper()\nfileroot.caller }\ny = pure()",
            ),
        ];
        assert_eq!(
            rooted_text(&files, "a.gcsl"),
            "{x = \"<root>\"; y = \"<root>\"}"
        );
    }

    #[test]
    fn fileroot_is_a_reserved_name() {
        for files in [
            &[("workspace.gcsl", ""), ("fileroot.gcsl", "x = 1")][..],
            &[("fileroot/a.gcsl", "x = 1"), ("workspace.gcsl", "")][..],
        ] {
            assert_eq!(
                error(files),
                (
                    "`fileroot` is the name of a built-in module".to_owned(),
                    Some(1),
                    vec![]
                ),
                "{files:?}"
            );
        }
    }

    /// The library of the memoisation tests: `reads`, which reads its
    /// caller; `plain`, which does not; `outer`, a helper whose callee reads
    /// the caller it passes on; `outer_hit`, a helper whose memoised callee,
    /// `reads`, may be found; `awaits`, which reads its caller in a `lazy`
    /// it awaits while it runs; `again`, which calls itself with its own
    /// arguments, from its own file, once it has read a caller of another;
    /// and four that return a step: `step`, whose `lazy` reads its caller,
    /// `plain_step`, whose does not, `helped`, whose is made after
    /// `fileroot.helper()` and calls `inner`, `nested`, whose makes a `lazy`
    /// that reads it, and `fn_step`, whose makes a function that reads its
    /// own.
    const MEMO_LIB: &str = "import fileroot\n\
        def reads(x: Int) ->> {c: String} { c = fileroot.caller }\n\
        def plain(x: Int) ->> {v: Int} { v = x }\n\
        def inner(): String { fileroot.caller }\n\
        def outer(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            c = inner()\n\
        }\n\
        def outer_hit(x: Int) ->> {c: String} {\n\
            fileroot.helper()\n\
            reads(x)\n\
        }\n\
        def awaits(x: Int) ->> {c: String} {\n\
            let made = lazy { fileroot.caller }\n\
            c = await made\n\
        }\n\
        def again(x: Int) ->> String {\n\
            let c = fileroot.caller\n\
            if c == fileroot.this { c } else { again(x) + \" \" + c }\n\
        }\n\
        def step(x: Int) ->> {s: Future<String>} { s = lazy { fileroot.caller } }\n\
        def plain_step(x: Int) ->> {s: Future<Int>} { s = lazy { x + 1 } }\n\
        def helped(x: Int) ->> {s: Future<String>} {\n\
            fileroot.helper()\n\
            s = lazy { inner() }\n\
        }\n\
        def nested(x: Int) ->> {s: Future<String>} { s = lazy { await lazy { fileroot.caller } } }\n\
        def fn_step(x: Int) ->> {s: Future<fn(): String>} { s = lazy { fn(): String { fileroot.caller } } }";

    /// Whether the values of the fields of `record` at `a` and `b` are one
    /// record: one run of the memoised function that made both.
    fn one_run(record: &Value, (a, b): (usize, usize)) -> bool {
        let Value::Record(record) = record else {
            panic!("expected a record, got {record}");
        };
        match (record.values.get(a), record.values.get(b)) {
            (Some(Value::Record(a)), Some(Value::Record(b))) => Rc::ptr_eq(a, b),
            other => panic!("expected records, got {other:?}"),
        }
    }

    #[test]
    fn a_memoised_call_depends_on_the_caller_it_reads() {
        let files = [
            ("workspace.gcsl", ""),
            ("lib/m.gcsl", MEMO_LIB),
            (
                "p/a.gcsl",
                "import lib.m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
            ),
            (
                "p/b.gcsl",
                "import lib.m\nrb = m.reads(1)\nvb = m.plain(1)\nob = m.outer(1)\nwb = m.awaits(1)",
            ),
            (
                "q/c.gcsl",
                "import lib.m\nr = m.reads(1)\nv = m.plain(1)\no = m.outer(1)\nw = m.awaits(1)",
            ),
            (
                "z/all.gcsl",
                "import p.a\nimport p.b\nimport q.c\n\
                 ra = a.r\nrb = b.rb\nrc = c.r\n\
                 va = a.v\nvb = b.vb\nvc = c.v\n\
                 oa = a.o\nob = b.ob\noc = c.o\n\
                 wa = a.w\nwb = b.wb\nwc = c.w",
            ),
        ];
        let all = rooted_value(&files, "z/all.gcsl");
        assert_eq!(
            all.to_string().replace(SCRATCH, "<root>"),
            "{ra = {c = \"<root>/p\"}; rb = {c = \"<root>/p\"}; rc = {c = \"<root>/q\"}; \
             va = {v = 1}; vb = {v = 1}; vc = {v = 1}; \
             oa = {c = \"<root>/p\"}; ob = {c = \"<root>/p\"}; oc = {c = \"<root>/q\"}; \
             wa = {c = \"<root>/p\"}; wb = {c = \"<root>/p\"}; wc = {c = \"<root>/q\"}}"
        );
        // A `lazy` it awaits while it runs reads its caller as it would.
        assert!(one_run(&all, (9, 10)));
        assert!(!one_run(&all, (9, 11)));
        // Reading the caller: one run for two files of one directory, whose
        // callers are equal, and another for another directory.
        assert!(one_run(&all, (0, 1)));
        assert!(!one_run(&all, (0, 2)));
        // Not reading it: one run from every directory.
        assert!(one_run(&all, (3, 4)));
        assert!(one_run(&all, (3, 5)));
        // A helper whose callee reads the caller it passed on depends on it.
        assert!(one_run(&all, (6, 7)));
        assert!(!one_run(&all, (6, 8)));
    }

    #[test]
    fn a_memoised_call_found_passes_its_dependencies_on() {
        // In `p`, `outer_hit(1)` finds `reads(1)`, which read `p`, as `first`
        // ran it: `outer_hit`'s entry depends on the caller through the hit,
        // so `q` runs it again rather than finding `p`'s.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/m.gcsl", MEMO_LIB),
            (
                "p/a.gcsl",
                "import lib.m\nfirst = m.reads(1)\nhit = m.outer_hit(len([first]))",
            ),
            ("q/c.gcsl", "import lib.m\nhit = m.outer_hit(1)"),
            (
                "z/all.gcsl",
                "import p.a\nimport q.c\nfirst = a.first\nhit = a.hit\nother = c.hit",
            ),
        ];
        let all = rooted_value(&files, "z/all.gcsl");
        assert_eq!(
            all.to_string().replace(SCRATCH, "<root>"),
            "{first = {c = \"<root>/p\"}; hit = {c = \"<root>/p\"}; other = {c = \"<root>/q\"}}"
        );
        // `p`'s `outer_hit` found the entry `first` made.
        assert!(one_run(&all, (0, 1)));
        assert!(!one_run(&all, (1, 2)));
    }

    #[test]
    fn a_step_that_can_read_its_caller_is_a_dependency_of_its_maker() {
        // Each step is awaited after the memoised call that made it has
        // returned, in a third directory.
        let made = "import lib.m\nst = m.step(1)\nps = m.plain_step(1)\n\
                    hs = m.helped(1)\nns = m.nested(1)\nfs = m.fn_step(1)";
        let files = [
            ("workspace.gcsl", ""),
            ("lib/m.gcsl", MEMO_LIB),
            ("p/a.gcsl", made),
            ("q/c.gcsl", made),
            (
                "z/all.gcsl",
                "import p.a\nimport q.c\n\
                 sa = await a.st.s\nsc = await c.st.s\n\
                 pa = a.ps\npc = c.ps\n\
                 ha = await a.hs.s\nhc = await c.hs.s\n\
                 na = await a.ns.s\nnc = await c.ns.s\n\
                 fa = a.fs\nfc = c.fs",
            ),
        ];
        let all = rooted_value(&files, "z/all.gcsl");
        let Value::Record(record) = &all else {
            panic!("expected a record, got {all}");
        };
        let shown: Vec<String> = record
            .values
            .iter()
            .map(|value| value.to_string().replace(SCRATCH, "<root>"))
            .collect();
        // A step reading its caller: one run of its maker for each
        // directory, each step reading its own, as the maker's caller.
        assert_eq!(shown[0], "<root>/p");
        assert_eq!(shown[1], "<root>/q");
        // A step that cannot read it: one run from every directory.
        assert!(one_run(&all, (2, 3)));
        // Made after `fileroot.helper()`, its calls pass the caller on.
        assert_eq!(shown[4], "<root>/p");
        assert_eq!(shown[5], "<root>/q");
        // A step making a step that reads it.
        assert_eq!(shown[6], "<root>/p");
        assert_eq!(shown[7], "<root>/q");
        // A function a step makes reads its own caller, not the step's: one
        // run from every directory.
        assert!(one_run(&all, (8, 9)));
    }

    #[test]
    fn a_running_memoised_call_is_found_as_a_lookup_finds_it() {
        // `again(1)` from `p` has read `p` when it calls `again(1)` from
        // `lib`: the running entry depends on `p`, so the call from `lib` is
        // not the call depending on its own result, and runs.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/m.gcsl", MEMO_LIB),
            ("p/a.gcsl", "import lib.m\nr = m.again(1)"),
        ];
        assert_eq!(
            rooted_text(&files, "p/a.gcsl"),
            "{r = \"<root>/lib <root>/p\"}"
        );
    }

    /// Every diagnostic of checking the workspace of `files`, in order,
    /// each shown as `MESSAGE at FILE (LABEL); FILE: LABEL; note: NOTE`,
    /// its file and each label's by path.
    fn check_errors(files: &[(&str, &str)]) -> Vec<String> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let (standard, checked) = check_with_standard(&sources, &[], "/");
        let Err(errors) = checked else {
            panic!("{files:?}: checked without an error");
        };
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        all.extend(paths);
        let path = |input: Option<u32>| {
            input
                .and_then(|i| all.get(i as usize))
                .copied()
                .unwrap_or("?")
        };
        errors
            .iter()
            .map(|error| {
                let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
                if let Some(label) = &error.label {
                    parts[0] = format!("{} ({label})", parts[0]);
                }
                for label in &error.labels {
                    let file = path(label.input.or(error.input));
                    parts.push(format!("{file}: {}", label.text));
                }
                for note in &error.notes {
                    parts.push(format!("note: {note}"));
                }
                parts.join("; ")
            })
            .collect()
    }

    /// The files of the slice 33 summary: named record types, one built
    /// from another by spread, an annotated field, a qualified name and a
    /// file below reading a type it imports.
    const TYPES: [(&str, &str); 3] = [
        (
            "_lib/types.gcsl",
            "type PackageDef = {\n\
             \x20   version: String,\n\
             \x20   arch: String,\n\
             \x20   content: String,\n\
             }\n\
             type Package = { ...PackageDef, name: String }",
        ),
        (
            "workspace.gcsl",
            "import _lib.types as t\n\
             type Steps = List<fn() -> String>\n\
             def make(name: String, f: fn(): t.PackageDef): { pkg: t.Package } {\n\
             \x20   pkg: t.Package = { ...f(); name = name }\n\
             }\n\
             built = make(\"hello\", fn(): t.PackageDef { version = \"1\"; arch = \"x\"; content = \"usr\" })",
        ),
        (
            "pkg/app.gcsl",
            "import Package from _lib.types\n\
             p: Package = { version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\" }\n\
             steps: Steps = []\n\
             n = built.pkg.name",
        ),
    ];

    #[test]
    fn the_types_summary_runs() {
        assert_eq!(
            value(&TYPES, "pkg/app.gcsl"),
            "{p = {version = \"1\"; arch = \"x\"; content = \"usr\"; name = \"n\"}; \
             steps = []; n = \"hello\"}"
        );
        // A type is shown as what it stands for, never by its name.
        assert_eq!(
            value(&TYPES, "workspace.gcsl"),
            "{make = fn(name: String, f: fn(): {arch: String, content: String, version: \
             String}): {pkg: {arch: String, content: String, name: String, version: String}}; \
             built = {pkg = {arch = \"x\"; content = \"usr\"; version = \"1\"; name = \"hello\"}}}"
        );
    }

    #[test]
    fn jacks_workspace_checks_with_spread_for_intersection() {
        // `/home/jack/projects/peios/newpkgs/workspace.gcsl`, with `&`
        // written as a spread and `#[def_type]` removed, and `ppk.commit`
        // stubbed by a module of the test's own.
        let files = [
            (
                "workspace.gcsl",
                "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
                 import _lib.ppk\n\
                 import fileroot\n\
                 \n\
                 type PackageDef = {\n\
                 \x20 version: String,\n\
                 \x20 arch: String,\n\
                 \x20 content: String,\n\
                 }\n\
                 \n\
                 type Package = { ...PackageDef, name: String }\n\
                 \n\
                 \n\
                 def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
                 \x20   fileroot.helper()\n\
                 \x20   pkg: Package = { ...func(); name = name }\n\
                 \n\
                 \x20   manifest = ppk.commit(\n\
                 \x20       name = pkg.name,\n\
                 \x20       version = pkg.version,\n\
                 \x20       arch = pkg.arch,\n\
                 \x20       content = pkg.content,\n\
                 \x20   )\n\
                 }\n",
            ),
            (
                "_lib/ppk.gcsl",
                "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
            ),
            (
                "dev.peios.testpkg/test.gcsl",
                "built = package(\"dev.peios.testpkg\", fn(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 })",
            ),
        ];
        checked(&files);
        assert_eq!(
            value(&files, "dev.peios.testpkg/test.gcsl"),
            "{built = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
        );
    }

    /// The files of the slice 34 summary as it is written: `hello` in
    /// `a.gcsl` reads `name` of `b.gcsl`, which reads `hello`.
    const DOTTED_DEFS: [(&str, &str); 3] = [
        ("workspace.gcsl", ""),
        (
            "pkgs/a.gcsl",
            "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.count(n: Int): Int {\n\
             \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
             }",
        ),
        (
            "pkgs/b.gcsl",
            "def dev.peios.name(): String { \"world\" }\n\
             dev.peios.version = \"1\"\n\
             greeting = dev.peios.hello()     # \"hello world\"\n\
             all = dev.peios                  # record of hello, count, name, version",
        ),
    ];

    #[test]
    fn the_dotted_defs_summary_is_two_files_that_need_each_other() {
        // As written, the two files need each other, as they do with every
        // `def` named by one name: a file runs as a whole before a file
        // that needs it.
        let (message, input, labels) = error(&DOTTED_DEFS);
        assert_eq!(message, "`pkgs/a.gcsl` and `pkgs/b.gcsl` need each other");
        assert_eq!((input, labels), (Some(1), vec![Some(2)]));
        let plain = [
            ("workspace.gcsl", ""),
            ("pkgs/a.gcsl", "def hello(): String { \"hello {name()}\" }"),
            (
                "pkgs/b.gcsl",
                "def name(): String { \"world\" }\ngreeting = hello()",
            ),
        ];
        assert_eq!(error(&plain).0, message);
        // With `name` beside `hello`, it runs: each file to its own value,
        // and `all` the record of the directory's leaves under the prefix,
        // the functions among them.
        let files = [
            ("workspace.gcsl", ""),
            (
                "pkgs/a.gcsl",
                "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
                 def dev.peios.count(n: Int): Int {\n\
                 \x20   if n == 0 { 0 } else { 1 + dev.peios.count(n - 1) }\n\
                 }\n\
                 def dev.peios.name(): String { \"world\" }",
            ),
            (
                "pkgs/b.gcsl",
                "dev.peios.version = \"1\"\n\
                 greeting = dev.peios.hello()\n\
                 all = dev.peios\n\
                 counted = dev.peios.count(3)",
            ),
        ];
        assert_eq!(
            value(&files, "pkgs/b.gcsl"),
            "{dev = {peios = {version = \"1\"}}; greeting = \"hello world\"; \
             all = {hello = fn(): String; count = fn(n: Int): Int; name = fn(): String; \
             version = \"1\"}; counted = 3}"
        );
        assert_eq!(
            value(&files, "pkgs/a.gcsl"),
            "{dev = {peios = {hello = fn(): String; count = fn(n: Int): Int; \
             name = fn(): String}}}"
        );
    }

    #[test]
    fn dotted_defs_across_the_files_of_a_directory() {
        // Called by path from another file of the directory, and from a
        // directory below.
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }\n\
                 def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
            ),
            ("b.gcsl", "x = dev.even(10)"),
            ("sub/c.gcsl", "y = dev.odd(3)\nall = dev"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "{x = true}");
        assert_eq!(
            value(&files, "sub/c.gcsl"),
            "{y = true; all = {odd = fn(n: Int): Bool; even = fn(n: Int): Bool}}"
        );
        // Two in two files that call each other need each other, as two
        // `def`s of one name do.
        let mutual = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "def dev.even(n: Int): Bool { if n == 0 { true } else { dev.odd(n - 1) } }",
            ),
            (
                "b.gcsl",
                "def dev.odd(n: Int): Bool { if n == 0 { false } else { dev.even(n - 1) } }",
            ),
        ];
        let plain = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "def even(n: Int): Bool { if n == 0 { true } else { odd(n - 1) } }",
            ),
            (
                "b.gcsl",
                "def odd(n: Int): Bool { if n == 0 { false } else { even(n - 1) } }",
            ),
        ];
        assert_eq!(error(&mutual), error(&plain));
        assert_eq!(error(&mutual).0, "`a.gcsl` and `b.gcsl` need each other");
        // A leaf conflict across files, as any leaf's.
        let conflict = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "def dev.x(): Int { 1 }"),
            ("b.gcsl", "dev.x = 2"),
        ];
        assert_eq!(error(&conflict).0, "field `dev.x` is defined in two files");
        // Shadowable across the files of a directory, in path order.
        let shadowed = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "#[shadowable] def dev.x(): Int { 1 }"),
            ("b.gcsl", "def dev.x(): Int { 2 }\ny = dev.x()"),
            ("c.gcsl", "z = dev.x()"),
        ];
        assert_eq!(value(&shadowed, "c.gcsl"), "{z = 2}");
        // A call from another file is of a known callee, as a `def` of one
        // name is: the call-site form of the implicit-await error.
        let known = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "def dev.inc(n: Int): Int { n + 1 }\ndef inc(n: Int): Int { n + 1 }",
            ),
            (
                "b.gcsl",
                "fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { inc(f) }",
            ),
        ];
        assert_eq!(
            messages(&known, "b.gcsl"),
            [
                "argument `f` would be awaited inside `dev.inc`, which is pure",
                "argument `f` would be awaited inside `inc`, which is pure",
            ]
        );
    }

    #[test]
    fn dotted_defs_through_each_form_of_import() {
        let lib = (
            "pkgs/a.gcsl",
            "def dev.peios.hello(): String { \"hello {dev.peios.name()}\" }\n\
             def dev.peios.name(): String { \"world\" }",
        );
        for (text, expected) in [
            (
                "import dev from pkgs.a\nh = dev.peios.hello()",
                "{h = \"hello world\"}",
            ),
            (
                "import dev as d from pkgs.a\nh = d.peios.name()",
                "{h = \"world\"}",
            ),
            (
                "import * from pkgs.a\ndef dev.own(): String { dev.peios.name() }\n\
                 h = dev.peios.hello()\no = dev.own()\nall = dev",
                "{dev = {own = fn(): String}; h = \"hello world\"; o = \"world\"; \
                 all = {own = fn(): String; peios = {hello = fn(): String; \
                 name = fn(): String}}}",
            ),
            (
                "import pkgs.a\nh = a.dev.peios.hello()\nw = a",
                "{h = \"hello world\"; w = {dev = {peios = {hello = fn(): String; \
                 name = fn(): String}}}}",
            ),
            ("import pkgs\nh = pkgs.dev.peios.name()", "{h = \"world\"}"),
        ] {
            let files = [("workspace.gcsl", ""), lib, ("app/main.gcsl", text)];
            assert_eq!(value(&files, "app/main.gcsl"), expected, "{text}");
        }
        // Through an import, a call is of a known callee.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/x.gcsl", "def dev.inc(n: Int): Int { n + 1 }"),
            (
                "app/main.gcsl",
                "import dev from lib.x\nimport lib\n\
                 fn p(f: Future<Int>): Int { dev.inc(f) }\n\
                 fn q(f: Future<Int>): Int { lib.dev.inc(f) }",
            ),
        ];
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            [
                "argument `f` would be awaited inside `dev.inc`, which is pure",
                "argument `f` would be awaited inside `lib.dev.inc`, which is pure",
            ]
        );
    }

    #[test]
    fn a_view_read_whole_has_its_dotted_defs_resolved() {
        // `dev` merges the root's, the import's and the file's own leaves:
        // read whole above the `def`, it holds the type its body gives it,
        // in either order of the file's items.
        let resolved = "{base: Int, lib: Int, own: {get: fn(n: Int | Future<Int>) -> Int}}";
        for text in [
            "import * from lib\nwhole = dev\ndef dev.own.get(n: Int) -> Int { n + 1 }\n\
             got = whole.own.get(1)",
            "import * from lib\ndef dev.own.get(n: Int) -> Int { n + 1 }\nwhole = dev\n\
             got = whole.own.get(1)",
        ] {
            let files = [
                ("workspace.gcsl", "dev.base = 1"),
                ("lib/l.gcsl", "dev.lib = 2"),
                ("app/main.gcsl", text),
            ];
            let mut workspace = checked(&files);
            let index = workspace.file("app/main.gcsl").unwrap();
            let dump = workspace.dump_types(index).unwrap();
            assert!(
                dump.contains(&format!("(name dev : {resolved}")),
                "{text}\n{dump}"
            );
            assert!(
                value(&files, "app/main.gcsl").ends_with("got = 2}"),
                "{text}"
            );
        }
        // And one that cannot be had there yet is the error at the read.
        let files = [
            ("workspace.gcsl", "dev.base = 1"),
            ("lib/l.gcsl", "dev.lib = 2"),
            (
                "app/main.gcsl",
                "import * from lib\nwhole = dev\nlet k = 1\ndef dev.own(): Int { k }",
            ),
        ];
        assert_eq!(
            messages(&files, "app/main.gcsl"),
            ["`dev.own` cannot be used here yet"]
        );
    }

    #[test]
    fn jacks_package_is_a_dotted_def_given_to_package() {
        // `/home/jack/projects/peios/newpkgs`, with `&` written as a spread,
        // `#[def_type]` removed and `ppk.commit` stubbed as in the slice 33
        // test, and the package written as a `def` named by its dotted path
        // passed to `package`.
        let files = [
            (
                "workspace.gcsl",
                "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
                 import _lib.ppk\n\
                 import fileroot\n\
                 \n\
                 type PackageDef = {\n\
                 \x20 version: String,\n\
                 \x20 arch: String,\n\
                 \x20 content: String,\n\
                 }\n\
                 \n\
                 type Package = { ...PackageDef, name: String }\n\
                 \n\
                 \n\
                 def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
                 \x20   fileroot.helper()\n\
                 \x20   pkg: Package = { ...func(); name = name }\n\
                 \n\
                 \x20   manifest = ppk.commit(\n\
                 \x20       name = pkg.name,\n\
                 \x20       version = pkg.version,\n\
                 \x20       arch = pkg.arch,\n\
                 \x20       content = pkg.content,\n\
                 \x20   )\n\
                 }\n",
            ),
            (
                "_lib/ppk.gcsl",
                "def commit(name: String, version: String, arch: String, content: String) -> String {\n\
                 \x20   \"{name} {version} {arch} {content}\"\n\
                 }",
            ),
            (
                "dev.peios.testpkg/test.gcsl",
                "def dev.peios.testpkg(): PackageDef {\n\
                 \x20   version = \"0.1.0\"\n\
                 \x20   arch = \"x86_64\"\n\
                 \x20   content = \"usr\"\n\
                 }\n\
                 \n\
                 result = package(\"dev.peios.testpkg\", dev.peios.testpkg)",
            ),
        ];
        checked(&files);
        assert_eq!(
            value(&files, "dev.peios.testpkg/test.gcsl"),
            "{dev = {peios = {testpkg = fn(): {arch: String, content: String, \
             version: String}}}; result = {pkg = {arch = \"x86_64\"; content = \"usr\"; \
             version = \"0.1.0\"; name = \"dev.peios.testpkg\"}; \
             manifest = \"dev.peios.testpkg 0.1.0 x86_64 usr\"}}"
        );
    }

    #[test]
    fn a_type_is_used_above_its_declaration_and_across_files() {
        // In its own file above its line, from another file of its
        // directory, and from a directory below, through the chain.
        let root = "a: Point = { x = 1; y = 2 }\ntype Point = {x: Int, y: Int}";
        let files = [
            ("workspace.gcsl", root),
            ("other.gcsl", "b: Point = { x = 3; y = 4 }"),
            ("sub/deep.gcsl", "c: Point = { x = 5; y = 6 }"),
        ];
        assert_eq!(value(&files, "workspace.gcsl"), "{a = {x = 1; y = 2}}");
        assert_eq!(value(&files, "other.gcsl"), "{b = {x = 3; y = 4}}");
        assert_eq!(value(&files, "sub/deep.gcsl"), "{c = {x = 5; y = 6}}");
        // A sibling directory does not see them.
        let hidden = [
            ("workspace.gcsl", ""),
            ("a/types.gcsl", "type Point = {x: Int}"),
            ("b/use.gcsl", "p: Point = { x = 1 }"),
        ];
        assert_eq!(messages(&hidden, "b/use.gcsl"), ["unknown type `Point`"]);
    }

    /// A library of types for the import tests: `lib/types.gcsl` with a
    /// record type and a type also a value's name, and `lib/more.gcsl`.
    const TYPE_LIB: [(&str, &str); 3] = [
        ("workspace.gcsl", ""),
        (
            "lib/types.gcsl",
            "type P = {x: Int}\ntype Both = Int\nBoth = 7",
        ),
        ("lib/more.gcsl", "type Q = {...P, y: String}"),
    ];

    #[test]
    fn types_are_reached_through_every_import_form() {
        let with = |main: &str| {
            let mut files = TYPE_LIB.to_vec();
            files.push(("app/main.gcsl", main));
            value(&files, "app/main.gcsl")
        };
        // Whole, by its path's last segment and `as`.
        assert_eq!(
            with("import lib.types\nv: types.P = { x = 1 }"),
            "{v = {x = 1}}"
        );
        assert_eq!(
            with("import lib.types as t\nv: t.P = { x = 1 }"),
            "{v = {x = 1}}"
        );
        // A directory's merged tree has its files' types.
        assert_eq!(
            with("import lib\nv: lib.Q = { x = 1; y = \"a\" }"),
            "{v = {x = 1; y = \"a\"}}"
        );
        // A member, bare or renamed.
        assert_eq!(
            with("import P from lib.types\nv: P = { x = 2 }"),
            "{v = {x = 2}}"
        );
        assert_eq!(
            with("import P as R from lib.types\nv: R = { x = 2 }"),
            "{v = {x = 2}}"
        );
        // `*`, with the fields.
        assert_eq!(
            with("import * from lib.types\nv: P = { x = Both }"),
            "{v = {x = 7}}"
        );
        // A member that is both a type and a value binds both.
        assert_eq!(
            with("import Both from lib.types\nv: Both = Both + 1"),
            "{v = 8}"
        );
    }

    #[test]
    fn qualified_names_go_two_deep() {
        // A deep module path, and a type whose definition names another
        // module's type qualified, used qualified from a third.
        let files = [
            ("workspace.gcsl", ""),
            ("a/b/inner.gcsl", "type Leaf = {v: Int}"),
            (
                "a/b/types.gcsl",
                "import a.b.inner as i\ntype Node = {leaf: i.Leaf, name: String}",
            ),
            (
                "app/main.gcsl",
                "import a.b.types as t\nn: t.Node = { leaf = { v = 1 }; name = \"n\" }\nw = n.leaf.v",
            ),
        ];
        assert_eq!(
            value(&files, "app/main.gcsl"),
            "{n = {leaf = {v = 1}; name = \"n\"}; w = 1}"
        );
    }

    #[test]
    fn a_type_and_a_value_of_one_name_coexist() {
        let files = [(
            "workspace.gcsl",
            "type Foo = {a: Int}\nFoo = 3\nlet x: Foo = { a = Foo }\nfn bar(Foo: Foo): Int { Foo.a }\ny = bar(x)",
        )];
        assert_eq!(value(&files, "workspace.gcsl"), "{Foo = 3; y = 3}");
        // `type` is still a name, as it was, for a variable and a field.
        let files = [(
            "workspace.gcsl",
            "type = 1\nlet t = type\nx = { type = t }.type\ny: Int = type",
        )];
        assert_eq!(value(&files, "workspace.gcsl"), "{type = 1; x = 1; y = 1}");
    }

    #[test]
    fn spread_in_a_record_type_overrides_as_value_spread_does() {
        let run = |types: &str, value_text: &str| {
            let prefix = "type A = {a: Int, b: Int}\ntype B = {b: String}";
            let text = format!("{prefix}\n{types}\n{value_text}");
            value(&[("workspace.gcsl", &text)], "workspace.gcsl")
        };
        // A later field overrides a spread's, its type differing.
        assert_eq!(
            run("type T = {...A, b: String}", "v: T = { a = 1; b = \"x\" }"),
            "{v = {a = 1; b = \"x\"}}"
        );
        // A later spread overrides an earlier one's.
        assert_eq!(
            run("type T = {...A, ...B}", "v: T = { a = 1; b = \"y\" }"),
            "{v = {a = 1; b = \"y\"}}"
        );
        assert_eq!(
            run("type T = {...B, ...A}", "v: T = { a = 1; b = 2 }"),
            "{v = {a = 1; b = 2}}"
        );
        // The same spread twice is idempotent.
        assert_eq!(
            run("type T = {...A, ...A}", "v: T = { a = 1; b = 2 }"),
            "{v = {a = 1; b = 2}}"
        );
        // A spread of a type built by a spread.
        assert_eq!(
            run(
                "type T = {...A, c: Bool}\ntype U = {...T, d: Int}",
                "v: U = { a = 1; b = 2; c = true; d = 4 }"
            ),
            "{v = {a = 1; b = 2; c = true; d = 4}}"
        );
        // A record type written in place spread.
        assert_eq!(
            run(
                "type T = {...{z: Int}, ...A}",
                "v: T = { z = 0; a = 1; b = 2 }"
            ),
            "{v = {z = 0; a = 1; b = 2}}"
        );
    }

    #[test]
    fn spread_problems_in_record_types() {
        let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
        let twice = "field `b` is defined twice at workspace.gcsl (`b` is a field of this, of \
                     type Int); workspace.gcsl: first defined here; note: a spread cannot \
                     replace a field defined before it; put the spread first";
        let text = "type A = {b: Int}\ntype T = {b: String, ...A}";
        assert_eq!(one(text), [twice]);
        let text = "type A = {b: Int, c: Int}\nx: {b: String, c: Bool, ...A} = { b = \"x\" }";
        let others = format!("{twice} (it also has 1 other field defined before it)");
        assert_eq!(one(text), [others]);
        assert_eq!(
            one("type T = {...Int, a: Int}"),
            ["cannot spread type Int: it is not a record type at workspace.gcsl"]
        );
        assert_eq!(
            one("type N = List<Int>\ntype T = {...N}"),
            ["cannot spread type `List<Int>`: it is not a record type at workspace.gcsl"]
        );
        // Only a record type, written so or named, is spread: the type is
        // shown as what it stands for.
        assert_eq!(
            one("x: {...Future<{a: Int}>} = { a = 1 }"),
            ["cannot spread type `Future<{a: Int}>`: it is not a record type at workspace.gcsl"]
        );
        let union = "cannot spread type `{a: Int} | {b: Int}`: it is not a record type at \
                     workspace.gcsl";
        assert_eq!(
            one("type A = {a: Int}\ntype B = {b: Int}\nx: {...A | B} = { a = 1 }"),
            [union]
        );
        // A record type written in place with a name twice, spread, is the
        // repeated-name error it is anywhere else, at the same place, in a
        // `type` and in a field's annotation alike.
        let repeated = |text: &str| {
            let source = SourceFile::new("workspace.gcsl", text).unwrap();
            let errors = check(std::slice::from_ref(&source)).unwrap_err();
            assert_eq!(errors.len(), 1, "{text}: {errors:?}");
            let at = u32::try_from(text.rfind("a: String").unwrap()).unwrap();
            assert_eq!(errors[0].span, Span::new(at, at + 1), "{text}");
            errors[0].message.clone()
        };
        let twice = "field `a` appears twice in this type";
        for text in [
            "type R = { a: Int, a: String }",
            "type R = { ...{ a: Int, a: String } }",
            "x: { a: Int, a: String } = { a = 1 }",
            "x: { ...{ a: Int, a: String } } = { a = 1 }",
            "type B = {b: Int}\nx: { ...B, ...{ a: Int, a: String } } = { a = 1; b = 2 }",
        ] {
            assert_eq!(repeated(text), twice, "{text}");
        }
        // And a type so defined is erroneous wherever it is used, reported
        // once, where it is written.
        assert_eq!(
            check_errors(&[
                ("workspace.gcsl", "type R = { ...{ a: Int, a: String } }"),
                ("use.gcsl", "x: R = { a = 1 }"),
            ]),
            [format!("{twice} at workspace.gcsl")]
        );
        // An erroneous type spread is reported where it is written, once.
        assert_eq!(
            one("type A = {a: Strng}\ntype T = {...A}\nx: T = { a = 1 }"),
            ["unknown type `Strng` at workspace.gcsl"]
        );
    }

    #[test]
    fn type_declaration_errors() {
        let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
        assert_eq!(
            one("type Int = String"),
            ["`Int` is a built-in type at workspace.gcsl"]
        );
        assert_eq!(
            one("type List = {a: Int}\ntype Future = Int"),
            [
                "`List` is a built-in type at workspace.gcsl",
                "`Future` is a built-in type at workspace.gcsl"
            ]
        );
        assert_eq!(
            one("type A = Int\nx = 1\ntype A = String"),
            ["type `A` is declared twice at workspace.gcsl; workspace.gcsl: first declared here"]
        );
        assert_eq!(
            one("type A = {a: Nope}\nx: A = { a = 1 }\ny: A = { a = 2 }"),
            ["unknown type `Nope` at workspace.gcsl"]
        );
        assert_eq!(
            one("type A = Int\nx: A<Int> = 1"),
            ["type `A` takes no type argument at workspace.gcsl"]
        );
        let mismatch = "mismatched types at workspace.gcsl (expected Int, found String); \
                        workspace.gcsl: expected because of this annotation";
        assert_eq!(one("type A = Int\nx: A = \"s\""), [mismatch]);
    }

    #[test]
    fn an_erroneous_type_is_reported_once_whichever_file_is_checked_first() {
        // `a.gcsl` is checked before `z.gcsl`, whose type it uses; and
        // `n.gcsl`, which needs `z.gcsl`, after it.
        let files = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x: Bad = { b = 1 }"),
            ("n.gcsl", "y = zed + 1"),
            ("z.gcsl", "type Bad = {b: Strng}\nzed = 1"),
        ];
        assert_eq!(check_errors(&files), ["unknown type `Strng` at z.gcsl"]);
    }

    #[test]
    fn type_conflicts_are_the_conflicts_of_fields() {
        let errors = |files: &[(&str, &str)]| check_errors(files);
        // Two files of one directory.
        let two = "type `Foo` is declared in two files at b.gcsl (declared here); \
                   a.gcsl: declared here";
        assert_eq!(
            errors(&[
                ("workspace.gcsl", ""),
                ("a.gcsl", "type Foo = Int"),
                ("b.gcsl", "type Foo = String"),
            ]),
            [two]
        );
        // An inner directory's and an outer one's.
        let above = "type `Foo` is declared in two files at workspace.gcsl (declared here); \
                     sub/a.gcsl: declared here; note: one is in a directory above";
        assert_eq!(
            errors(&[
                ("workspace.gcsl", "type Foo = Int"),
                ("sub/a.gcsl", "type Foo = String"),
            ]),
            [above]
        );
        // An imported one and an own one.
        let imported = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                        lib/t.gcsl: declared here; app/m.gcsl: declared here";
        assert_eq!(
            errors(&[
                ("workspace.gcsl", ""),
                ("lib/t.gcsl", "type Foo = Int"),
                ("app/m.gcsl", "import Foo from lib.t\ntype Foo = String"),
            ]),
            [imported]
        );
        // A `#[shadowable]` import brings types as plain ones, which an own
        // type of their name conflicts with.
        let star = "type `Foo` is declared in two files at app/m.gcsl (imported here); \
                    lib/t.gcsl: declared here; app/m.gcsl: declared here";
        assert_eq!(
            errors(&[
                ("workspace.gcsl", ""),
                ("lib/t.gcsl", "type Foo = Int"),
                (
                    "app/m.gcsl",
                    "#[shadowable] import * from lib.t\ntype Foo = String"
                ),
            ]),
            [star]
        );
        // One type reached twice is one type.
        let files = [
            ("workspace.gcsl", ""),
            ("lib/t.gcsl", "type Foo = Int"),
            (
                "lib/m.gcsl",
                "import * from lib\nimport Foo from lib.t\nx: Foo = 1",
            ),
        ];
        assert_eq!(value(&files, "lib/m.gcsl"), "{x = 1}");
    }

    #[test]
    fn qualified_names_that_name_no_type() {
        let with = |main: &str| {
            let mut files = TYPE_LIB.to_vec();
            files.push(("app/main.gcsl", main));
            check_errors(&files)
        };
        assert_eq!(
            with("import lib.types as t\nx: t.Nope = 1"),
            ["`t` has no type `Nope` at app/main.gcsl; note: `t` has the types `P`, `Both`"]
        );
        assert_eq!(
            with("import lib.more as m\nimport builtins\nx: builtins.Int = 1"),
            ["`builtins` has no type `Int` at app/main.gcsl; note: `builtins` has no types"]
        );
        assert_eq!(
            with("import lib.types as t\nx: t = 1"),
            ["`t` is a module, not a type at app/main.gcsl"]
        );
        assert_eq!(
            with("import lib.types as t\nx: t.P.x = 1"),
            ["`t.P` is a type, not a module at app/main.gcsl"]
        );
        assert_eq!(
            with("x: nothing.P = 1"),
            ["unknown type `nothing.P` at app/main.gcsl"]
        );
    }

    #[test]
    fn cycles_of_types() {
        let one = |text: &str| check_errors(&[("workspace.gcsl", text)]);
        assert_eq!(
            one("type A = {a: A}"),
            ["type `A` refers to itself at workspace.gcsl; workspace.gcsl: `A` uses `A` here"]
        );
        assert_eq!(
            one("type A = B\ntype B = {a: A}"),
            ["type `A` refers to itself at workspace.gcsl; \
              workspace.gcsl: `A` uses `B` here; workspace.gcsl: `B` uses `A` here"]
        );
        // Across files, at the first in path then source order.
        let three = "type `A` refers to itself at a.gcsl; a.gcsl: `A` uses `B` here; \
                     b.gcsl: `B` uses `C` here; x.gcsl: `C` uses `A` here";
        assert_eq!(
            check_errors(&[
                ("workspace.gcsl", ""),
                ("x.gcsl", "type C = {c: A}"),
                ("a.gcsl", "type A = List<B>"),
                ("b.gcsl", "type B = {...C}"),
            ]),
            [three]
        );
    }

    #[test]
    fn annotated_fields_have_their_annotations_types() {
        // Top-level, in an inner block, dotted and quoted, read from another
        // file, whose readers see the annotation's type, not the value's.
        let files = [
            ("workspace.gcsl", ""),
            (
                "a.gcsl",
                "x: Int | String = 1\n\
                 conf.port: Int | String = 80\n\
                 \"a b\": Float = 2\n\
                 inner = { y: Int | String = \"s\"; z = y }",
            ),
            (
                "b.gcsl",
                "import a as m\n\
                 kind = if x is Int { \"int\" } else { \"string\" }\n\
                 port = conf.port\n\
                 f = m.\"a b\" + 0.5",
            ),
        ];
        assert_eq!(
            value(&files, "a.gcsl"),
            "{x = 1; conf = {port = 80}; \"a b\" = 2.0; inner = {y = \"s\"; z = \"s\"}}"
        );
        assert_eq!(
            value(&files, "b.gcsl"),
            "{kind = \"int\"; port = 80; f = 2.5}"
        );
        let wide = [
            ("workspace.gcsl", ""),
            ("a.gcsl", "x: Int | String = 1"),
            ("b.gcsl", "y = x + 1"),
        ];
        let add = "cannot add `Int | String` and Int at b.gcsl; b.gcsl: Int | String; \
                   b.gcsl: Int; note: tell them apart first, as in `if x is Int { ... }`";
        assert_eq!(check_errors(&wide), [add]);
    }

    #[test]
    fn annotated_fields_check_as_annotated_lets_do() {
        let one = |text: &str| value(&[("workspace.gcsl", text)], "workspace.gcsl");
        let errors = |text: &str| check_errors(&[("workspace.gcsl", text)]);
        // A value that fits once awaited is awaited, a literal adapts, and a
        // pure function's parameter the annotation needs present is
        // measured, present.
        assert_eq!(
            one("x: Int = lazy 1\ny: Float = 2\nf = fn(n: Int): {v: Int} { v: Int = n }"),
            "{x = 1; y = 2.0; f = fn(n: Int): {v: Int}}"
        );
        // In a record block checked against an expected record type.
        assert_eq!(
            one("let r: {a: Int | String} = { a: Int = 1 }\nv = r.a"),
            "{v = 1}"
        );
        let mismatch = "mismatched types at workspace.gcsl (expected Int, found Int | String); \
                        workspace.gcsl: expected because of this annotation";
        assert_eq!(
            errors("let r: {a: Int} = { a: Int | String = 1 }"),
            [mismatch]
        );
        // TEMPORARY (alias rule): an annotated field is no alias of a
        // built-in function, as an annotated `let` is none.
        let value_yet = "`len` is a built-in function and cannot be used as a value yet at \
                         workspace.gcsl; note: a built-in function with a type for any list \
                         cannot be written as a type yet; call it, or bind it to a name";
        assert_eq!(errors("l: fn(xs: List<Int>): Int = len"), [value_yet]);
    }

    #[test]
    fn an_alias_is_bare_where_it_is_used() {
        // As if `Int` were written at each use: the either shape in an
        // impure function's bare parameter, inferred in a pure one's.
        let files = [(
            "workspace.gcsl",
            "type N = Int\n\
             type F = Future<Int>\n\
             def impure(x: N) -> N { x }\n\
             def pure(x: N): Int { x }\n\
             def either(x: Int | F): Int { 1 }",
        )];
        assert_eq!(
            value(&files, "workspace.gcsl"),
            "{impure = fn(x: Int | Future<Int>) -> Int | Future<Int>; \
              pure = fn(x: Int): Int; either = fn(x: Int | Future<Int>): Int}"
        );
        assert_eq!(
            check_errors(&[(
                "workspace.gcsl",
                "type F = Future<Int>\nx: Int | F | String = 1"
            )]),
            ["a union with a `Future` member must be `T | Future<T>` at workspace.gcsl"]
        );
    }

    #[test]
    fn a_type_expanded_too_deep_is_an_error_where_it_is_written() {
        // Each type nests the one before it a level deeper, so the chain
        // passes `MAX_DEPTH` though no type is written deeply: `T10000` is
        // one level too deep where it is written, and `T10001`, which uses
        // it, is erroneous wherever it is used.
        let chain: Vec<String> = (1..=10_001)
            .map(|at| format!("type T{at} = {{a: T{}}}", at - 1))
            .collect();
        let text = format!("type T0 = Int\n{}\nx: T10001 = 1", chain.join("\n"));
        let errors = std::thread::Builder::new()
            .stack_size(crate::STACK_SIZE)
            .spawn(move || check_errors(&[("workspace.gcsl", &text)]))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(errors, ["expression nested too deeply at workspace.gcsl"]);
    }

    #[test]
    fn a_value_file_declares_types() {
        let files = [
            ("workspace.gcsl", ""),
            ("lib/v.gcsl", "type T = {a: Int}\n= 5"),
            (
                "app.gcsl",
                "import lib.v\nimport T from lib.v\nx: v.T = { a = v }\ny: T = x",
            ),
        ];
        assert_eq!(value(&files, "app.gcsl"), "{x = {a = 5}; y = {a = 5}}");
    }

    #[test]
    fn dump_types_shows_each_type_with_its_expansion() {
        let mut workspace = checked(&TYPES);
        let index = workspace.file("_lib/types.gcsl").unwrap();
        assert_eq!(
            workspace.dump_types(index).unwrap(),
            "(file\n\
             \x20 (type PackageDef {version: String, arch: String, content: String} = \
             {arch: String, content: String, version: String} @0..81)\n\
             \x20 (type Package {...PackageDef, name: String} = {arch: String, content: String, \
             name: String, version: String} @82..128))"
        );
        // A use prints as written.
        let index = workspace.file("pkg/app.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(dump.contains("(field p : Package @"), "{dump}");
        let index = workspace.file("workspace.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(dump.contains("(f fn(): t.PackageDef)"), "{dump}");
        assert!(
            dump.contains(
                "(type Steps List<fn() -> String> = List<fn() -> String | Future<String>> @"
            ),
            "{dump}"
        );
        // A type defined as another prints as written, and dumps as what
        // both stand for.
        let mut workspace = checked(&[("workspace.gcsl", "type A = Int\ntype B = A")]);
        let index = workspace.file("workspace.gcsl").unwrap();
        assert_eq!(
            workspace.dump_types(index).unwrap(),
            "(file\n  (type A Int = Int @0..12)\n  (type B A = Int @13..23))"
        );
    }

    /// Jack's `/home/jack/projects/peios/newpkgs` as it is written, with
    /// `&` written as a spread and the commas of the package's block as
    /// newlines, the bottom line `result = package dev.peios.testpkg()`,
    /// and `ppk.commit` stubbed by a module of the test's own, which joins
    /// the content to its caller's directory as the real one does.
    const JACKS_PACKAGE: [(&str, &str); 3] = [
        (
            "workspace.gcsl",
            "# Marks newpkgs/ as a GCSL workspace: imports are absolute from here.\n\
             import _lib.ppk\n\
             import fileroot\n\
             \n\
             type PackageDef = {\n\
             \x20 version: String,\n\
             \x20 arch: String,\n\
             \x20 content: String,\n\
             }\n\
             \n\
             type Package = { ...PackageDef, name: String }\n\
             \n\
             \n\
             #[def_type]\n\
             def package(name: String, func: fn(): PackageDef) -> { pkg: Package, manifest: String } {\n\
             \x20   fileroot.helper()\n\
             \x20   pkg: Package = { ...func(); name = name }\n\
             \n\
             \x20   manifest = ppk.commit(\n\
             \x20       name = pkg.name,\n\
             \x20       version = pkg.version,\n\
             \x20       arch = pkg.arch,\n\
             \x20       content = pkg.content,\n\
             \x20   )\n\
             }\n",
        ),
        (
            "_lib/ppk.gcsl",
            "import path\n\
             import fileroot\n\
             def commit(name: String, version: String, arch: String, content: String) -> String {\n\
             \x20   let content = path.join(fileroot.caller, content)\n\
             \x20   \"{name} {version} {arch} {content}\"\n\
             }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "def package dev.peios.testpkg() {\n\
             \x20   version = \"0.1.0\"\n\
             \x20   arch = \"x86_64\"\n\
             \x20   content = \"usr\"\n\
             }\n\
             \n\
             \n\
             result = package dev.peios.testpkg()",
        ),
    ];

    #[test]
    fn jacks_package_is_defined_and_called_with_its_def_type() {
        // The stub is given the package's directory as the content's base,
        // through the def-type's `fileroot.helper()`; the definition binds
        // nothing of the file's record.
        assert_eq!(
            rooted_text(&JACKS_PACKAGE, "dev.peios.testpkg/test.gcsl"),
            "{result = {pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}}"
        );
    }

    /// A def-type of the root of a workspace, for the def-type tests:
    /// `build`, whose definitions take a target and give a string, and
    /// which takes a mode when it is invoked.
    const BUILD: (&str, &str) = (
        "workspace.gcsl",
        "#[def_type]\n\
         def build(name: String, f: fn(target: String): String, mode: String): String {\n\
         \x20   \"{name}: {f(mode)}\"\n\
         }",
    );

    #[test]
    fn a_definition_takes_its_types_and_its_invocation_its_arguments() {
        // A definition with a parameter, used above it; invocation
        // parameters; a reference stored and called later; `def x()` beside
        // `def build x()`, a bare `x` seeing only the former.
        let files = [
            BUILD,
            (
                "app.gcsl",
                "y = build x(\"release\")\n\
                 def build x(target) { \"built {target}\" }\n\
                 def x(): Int { 7 }\n\
                 let later = build x\n\
                 z = later(mode = \"debug\")\n\
                 plain = x()",
            ),
        ];
        assert_eq!(
            value(&files, "app.gcsl"),
            "{y = \"x: built release\"; x = fn(): Int; z = \"x: built debug\"; plain = 7}"
        );
        // From another file of the directory and from a directory below.
        let files = [
            BUILD,
            ("a.gcsl", "def build lib.core(t) { \"core-{t}\" }"),
            ("b.gcsl", "b = build lib.core(\"b\")"),
            ("sub/c.gcsl", "c = build lib.core(\"c\")"),
        ];
        assert_eq!(value(&files, "b.gcsl"), "{b = \"lib.core: core-b\"}");
        assert_eq!(value(&files, "sub/c.gcsl"), "{c = \"lib.core: core-c\"}");
        assert_eq!(value(&files, "a.gcsl"), "{}");
    }

    #[test]
    fn a_definition_is_a_leaf_under_its_namespace_s_key() {
        // The scan names the namespace, makes the definition a top-level
        // function and a leaf under its key, and a reference above it
        // mentions it, as a read of its path.
        let files = [
            BUILD,
            ("app.gcsl", "y = build x(\"m\")\ndef build x(t) { t }"),
        ];
        let scan = ws(&files).unwrap();
        let app = scan.files.iter().find(|f| f.path == "app.gcsl").unwrap();
        let workspace = scan
            .files
            .iter()
            .position(|f| f.path == "workspace.gcsl")
            .unwrap();
        let tree = &app.mentions.tree;
        // The roots: `y`, then the namespace, whose child `x` is the leaf.
        let root = &tree.nodes[tree.roots[1] as usize];
        // The `def`'s item, the second the parser made, after its body's.
        assert_eq!(scan.names.get(root.name), format!("<build#{workspace}.1>"));
        assert_eq!(root.children, [2]);
        assert_eq!(tree.path(2).len(), 2);
        assert_eq!(app.mentions.functions.len(), 1);
        assert_eq!(app.mentions.functions[0].name, None);
        assert_eq!(app.mentions.items[0], [0]);
        assert_eq!(app.mentions.fields[0], [2]);
        // The definition's item mentions its def-type, another file's `def`:
        // its node and its function.
        assert_eq!(app.mentions.foreign[1].len(), 2);
    }

    #[test]
    fn each_def_type_has_a_namespace_of_its_own() {
        // Two def-types, each with a definition of one path, and a field of
        // that path too: none meets another.
        let files = [
            BUILD,
            (
                "app.gcsl",
                "#[def_type]\n\
                 def test(name: String, f: fn(target: String): String): String { f(name) }\n\
                 def build lib.core(t) { \"built {t}\" }\n\
                 def test lib.core(t) { \"tested {t}\" }\n\
                 lib.core = 1\n\
                 a = build lib.core(\"r\")\n\
                 b = test lib.core()",
            ),
        ];
        assert_eq!(
            value(&files, "app.gcsl"),
            "{test = fn(name: String, f: fn(target: String): String): String; \
             lib = {core = 1}; a = \"lib.core: built r\"; b = \"tested lib.core\"}"
        );
    }

    #[test]
    fn definitions_conflict_and_shadow_as_dotted_defs_do() {
        let conflict = [
            BUILD,
            ("a.gcsl", "def build x(t) { t }"),
            ("b.gcsl", "def build x(t) { t }"),
        ];
        assert_eq!(
            error(&conflict).0,
            "field `build x` is defined in two files"
        );
        let under = [
            BUILD,
            ("a.gcsl", "def build x(t) { t }"),
            ("b.gcsl", "def build x.y(t) { t }"),
        ];
        assert_eq!(error(&under).0, "field `build x` is already defined");
        // In one file, as the dotted fields of a block.
        let files = [
            BUILD,
            (
                "a.gcsl",
                "def build x(t) { t }\ndef build x(t) { t }\ny = build x(\"m\")",
            ),
        ];
        assert_eq!(
            messages(&files, "a.gcsl"),
            ["field `build x` is defined twice"]
        );
        // Shadowable, across files in path order and within one file.
        let shadowed = [
            BUILD,
            ("a.gcsl", "#[shadowable] def build x(t) { \"a {t}\" }"),
            (
                "b.gcsl",
                "#[shadowable] def build x(t) { \"b {t}\" }\ndef build x(t) { \"c {t}\" }",
            ),
            ("d.gcsl", "y = build x(\"m\")"),
        ];
        assert_eq!(value(&shadowed, "d.gcsl"), "{y = \"x: c m\"}");
        let reversed = [
            BUILD,
            ("a.gcsl", "def build x(t) { t }"),
            ("b.gcsl", "#[shadowable] def build x(t) { t }"),
        ];
        assert_eq!(
            error(&reversed).0,
            "field `build x` is defined in two files"
        );
    }

    #[test]
    fn a_def_type_is_reached_through_its_imports() {
        let lib = (
            "lib/types.gcsl",
            "#[def_type]\n\
             def build(name: String, f: fn(): String): String { \"{name} {f()}\" }",
        );
        // Through a renamed member, defining and invoking; through a whole
        // import, qualified; and a definition through one alias is in the
        // def-type's namespace whatever another file calls it.
        let files = [
            ("workspace.gcsl", ""),
            lib,
            (
                "app/a.gcsl",
                "import build as b from lib.types\n\
                 def b x() { \"via b\" }\n\
                 one = b x()",
            ),
            (
                "app/c.gcsl",
                "import lib.types as t\n\
                 two = t.build x()",
            ),
        ];
        assert_eq!(value(&files, "app/a.gcsl"), "{one = \"x via b\"}");
        assert_eq!(value(&files, "app/c.gcsl"), "{two = \"x via b\"}");
        // `*` brings a file's definitions; a whole import of it does not.
        let defs = (
            "lib/defs.gcsl",
            "import * from lib.types\ndef build x() { \"x\" }",
        );
        let files = [
            ("workspace.gcsl", ""),
            lib,
            defs,
            (
                "app/a.gcsl",
                "import * from lib.defs\nimport * from lib.types\na = build x()",
            ),
            ("app/c.gcsl", "import lib.defs\nc = defs"),
        ];
        assert_eq!(value(&files, "app/a.gcsl"), "{a = \"x x\"}");
        // Read whole, a file holds no namespace.
        assert_eq!(value(&files, "app/c.gcsl"), "{c = {}}");
        let files = [
            ("workspace.gcsl", ""),
            lib,
            defs,
            (
                "app/b.gcsl",
                "import lib.defs\nimport * from lib.types\nb = build x()",
            ),
        ];
        assert_eq!(
            messages(&files, "app/b.gcsl"),
            ["`build` has no definition `x`"]
        );
    }

    #[test]
    fn a_def_type_s_caller_is_where_its_reference_is_called() {
        // The reference is made in `a/`, called in `b/`: the def-type runs
        // for `b/`, and the definition for the def-type's file, but through
        // its helper.
        let files = [
            (
                "workspace.gcsl",
                "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn() -> String) -> { called: String, defined: String } {\n\
                 \x20   called = fileroot.caller\n\
                 \x20   defined = f()\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn() -> String) -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   f()\n\
                 }",
            ),
            (
                "a/a.gcsl",
                "import fileroot\n\
                 def where x() { fileroot.caller }\n\
                 def helped x() { fileroot.caller }\n\
                 def made(): fn() -> { called: String, defined: String } { where x }\n\
                 def made_helped(): fn() -> String { helped x }",
            ),
            (
                "b/b.gcsl",
                "import a.a\n\
                 there = a.made()()\n\
                 through = a.made_helped()()",
            ),
        ];
        assert_eq!(
            rooted_text(&files, "b/b.gcsl"),
            "{there = {called = \"<root>/b\"; defined = \"<root>\"}; through = \"<root>/b\"}"
        );
    }

    #[test]
    fn a_pure_definition_cannot_call_an_impure_function() {
        let files = [
            BUILD,
            (
                "app.gcsl",
                "fn noisy() -> String { \"n\" }\n\
                 def build x(t) { noisy() }",
            ),
        ];
        assert_eq!(
            messages(&files, "app.gcsl"),
            ["cannot call an impure function from a pure function"]
        );
        // And an impure def-type's reference from a pure function.
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\ndef run(name: String, f: fn(): String) -> String { f() }",
            ),
            (
                "app.gcsl",
                "def run x() { \"x\" }\nfn p(): String { run x() }",
            ),
        ];
        assert_eq!(
            messages(&files, "app.gcsl"),
            ["cannot call an impure function from a pure function"]
        );
    }

    #[test]
    fn def_type_errors() {
        // Each error with its labels and notes, joined by `; `, the errors
        // by newlines.
        let errors = |text: &str| {
            let files = [BUILD, ("app.gcsl", text)];
            check_errors(&files).join("\n")
        };
        let signature = "a def-type takes the name as a `String` and then the definition as a \
                         function, as in `def bad(name: String, func: fn(): T)` at app.gcsl";
        assert_eq!(
            errors("#[def_type]\ndef bad(name: Int, f: fn(): Int): Int { 1 }"),
            signature
        );
        assert_eq!(errors("#[def_type]\ndef bad(): Int { 1 }"), signature);
        let not_one = "note: a def-type is a `def` marked `#[def_type]`";
        assert_eq!(
            errors("def plain(): Int { 1 }\ndef plain x() { 1 }"),
            format!("`plain` is not a def-type at app.gcsl; {not_one}")
        );
        assert_eq!(
            errors("def build x() { \"x\" }"),
            "`build` defines functions of 1 parameter, `target` at app.gcsl"
        );
        assert_eq!(
            errors(
                "#[def_type]\ndef two(name: String, f: fn(a: Int, b: Int): Int): Int { 1 }\n\
                 def two x(a) { a }"
            ),
            "`two` defines functions of 2 parameters, `a` and `b` at app.gcsl"
        );
        assert_eq!(
            errors(
                "#[def_type]\ndef none(name: String, f: fn(): Int): Int { 1 }\n\
                 def none x(a) { a }"
            ),
            "`none` defines functions of no parameters at app.gcsl"
        );
        assert_eq!(
            errors("def build a.b(t) { t }\nx = build a.c\ny = build a\nz = build q"),
            "`build` has no definition `a.c` at app.gcsl; \
             note: the only definition of `build` here is `a.b`\n\
             `a` is not a `build` definition at app.gcsl\n\
             `build` has no definition `q` at app.gcsl; \
             note: the only definition of `build` here is `a.b`"
        );
        assert_eq!(
            errors("def build a(t) { t }\ndef build b.c(t) { t }\nx = build d"),
            "`build` has no definition `d` at app.gcsl; \
             note: the definitions of `build` here are `a` and `b.c`"
        );
        assert_eq!(
            errors("x = build d"),
            "`build` has no definition `d` at app.gcsl; note: `build` has no definitions here"
        );
        // Two names in a row, the first no def-type, or undefined.
        let juxtaposed = "note: two names in a row are read as a def-type reference, as in \
                          `package dev.x()`; is an operator or a line break missing?";
        assert_eq!(
            errors("let a = 1\nx = a b"),
            format!("`a` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
        );
        assert!(
            errors("x = nope b").starts_with("undefined variable `nope` at app.gcsl")
                && errors("x = nope b").ends_with(juxtaposed),
            "{}",
            errors("x = nope b")
        );
        // A parameter of the def-type's name is no def-type.
        assert_eq!(
            errors("def build x(t) { t }\nfn f(build: Int): Int { build x() }"),
            format!("`build` is not a def-type at app.gcsl; {not_one}; {juxtaposed}")
        );
    }

    #[test]
    fn no_source_can_write_a_namespace_s_key() {
        // A field of the key's form is no field at all, so it cannot reach
        // a namespace's definitions.
        let files = [
            BUILD,
            ("app.gcsl", "def build x(t) { t }\n\"<build#1.0>\" = 1"),
        ];
        assert_eq!(
            check_errors(&files),
            [
                "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
            ]
        );
        let files = [BUILD, ("app.gcsl", "x = {}.\"<build#1.0>\"")];
        assert_eq!(
            check_errors(&files),
            [
                "a field cannot be named `\"<build#1.0>\"`: names of that form are def-types' \
                 namespaces at app.gcsl"
            ]
        );
    }

    #[test]
    fn dump_ast_shows_definitions_and_references() {
        let mut workspace = checked(&[
            BUILD,
            ("app.gcsl", "def build x(t) { t }\ny = build x(\"m\")"),
        ]);
        let index = workspace.file("app.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        assert!(dump.contains("(def-type-def build x @0..20\n"), "{dump}");
        assert!(dump.contains("(params t)\n"), "{dump}");
        assert!(
            dump.contains("(def-type-ref build x : fn(mode: String): String @25..32)"),
            "{dump}"
        );
    }

    /// Jack's `/home/jack/projects/peios/newpkgs` as it is written, but for
    /// `&` written as a spread and the commas of the package's block as
    /// newlines, as in [`JACKS_PACKAGE`], with no line invoking the package:
    /// `ppk.commit` stubbed by a module of the test's own, which returns its
    /// arguments, the content joined to its caller's directory as the real
    /// one joins it.
    const NEWPKGS: [(&str, &str); 3] = [
        JACKS_PACKAGE[0],
        (
            "_lib/ppk.gcsl",
            "import path\n\
             import fileroot\n\
             def commit(name: String, version: String, arch: String, content: String) -> String {\n\
             \x20   let content = path.join(fileroot.caller, content)\n\
             \x20   \"{name} {version} {arch} {content}\"\n\
             }",
        ),
        (
            "dev.peios.testpkg/test.gcsl",
            "def package dev.peios.testpkg() {\n\
             \x20   version = \"0.1.0\"\n\
             \x20   arch = \"x86_64\"\n\
             \x20   content = \"usr\"\n\
             }\n",
        ),
    ];

    /// Checks the workspace of `files`, located at [`SCRATCH`], and invokes
    /// the definition `path` of the def-type `def_type` in it, with the
    /// arguments `args`.
    fn invoke_with(
        files: &[(&str, &str)],
        (def_type, path): (&str, &str),
        args: &[&str],
    ) -> Result<Value, InvokeError> {
        let mut workspace = rooted(files).unwrap_or_else(|errors| panic!("{errors:?}"));
        let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
        workspace
            .invoke(def_type, path, &args)
            .map(|outcome| outcome.value)
    }

    /// [`invoke_with`]'s value with no arguments shown, [`SCRATCH`] shown as
    /// `<root>`.
    fn invoked_text(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
        invoked_text_with(files, (def_type, path), &[])
    }

    /// [`invoke_with`]'s value shown, [`SCRATCH`] shown as `<root>`.
    fn invoked_text_with(
        files: &[(&str, &str)],
        (def_type, path): (&str, &str),
        args: &[&str],
    ) -> String {
        match invoke_with(files, (def_type, path), args) {
            Ok(value) => value.to_string().replace(SCRATCH, "<root>"),
            Err(error) => panic!("{def_type} {path} {args:?}: {error:?}"),
        }
    }

    /// The request error of invoking `path` of `def_type` in the workspace
    /// of `files`, with no arguments: its message and its notes, joined by
    /// `; `.
    fn request_error(files: &[(&str, &str)], def_type: &str, path: &str) -> String {
        request_error_with(files, (def_type, path), &[])
    }

    /// [`request_error`] with the arguments `args`.
    fn request_error_with(
        files: &[(&str, &str)],
        (def_type, path): (&str, &str),
        args: &[&str],
    ) -> String {
        match invoke_with(files, (def_type, path), args) {
            Err(InvokeError::Request(error)) => {
                let mut parts = vec![error.message()];
                parts.extend(
                    error
                        .notes()
                        .into_iter()
                        .map(|note| format!("note: {note}")),
                );
                assert_eq!(error.to_string(), error.message());
                parts.join("; ")
            }
            other => panic!("{def_type} {path}: {other:?}"),
        }
    }

    /// The diagnostics invoking `path` of `def_type` in the workspace of
    /// `files` fails with, each shown as [`check_errors`] shows them.
    fn invoke_errors(files: &[(&str, &str)], def_type: &str, path: &str) -> Vec<String> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
        let errors = match checked.unwrap().invoke(def_type, path, &[]) {
            Err(InvokeError::Diagnostics(errors)) => errors,
            other => panic!("{def_type} {path}: {other:?}"),
        };
        let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        all.extend(paths);
        shown_errors(&errors, &all)
    }

    /// `errors`, each shown with its labels and notes, joined by `; `, its
    /// input and each label's by its path in `all`, by input number.
    fn shown_errors(errors: &[Diagnostic], all: &[&str]) -> Vec<String> {
        let path = |input: Option<u32>| {
            input
                .and_then(|i| all.get(i as usize))
                .copied()
                .unwrap_or("?")
        };
        errors
            .iter()
            .map(|error| {
                let mut parts = vec![format!("{} at {}", error.message, path(error.input))];
                if let Some(label) = &error.label {
                    parts[0] = format!("{} ({label})", parts[0]);
                }
                for label in &error.labels {
                    let at = path(label.input.or(error.input));
                    parts.push(format!("{at}: {}", label.text));
                }
                parts.extend(error.notes.iter().map(|note| format!("note: {note}")));
                parts.join("; ")
            })
            .collect()
    }

    #[test]
    fn jacks_package_is_invoked_by_name() {
        // The record the def-type returns; the stub given the package's
        // directory as the content's base, through `fileroot.helper()`.
        assert_eq!(
            invoked_text(&NEWPKGS, "package", "dev.peios.testpkg"),
            "{pkg = {arch = \"x86_64\"; content = \"usr\"; version = \"0.1.0\"; \
             name = \"dev.peios.testpkg\"}; manifest = \"dev.peios.testpkg 0.1.0 x86_64 \
             <root>/dev.peios.testpkg/usr\"}"
        );
        // Listed: the def-type, its file and the span of its name, and the
        // definition, its file and the span of its path.
        let workspace = rooted(&NEWPKGS).unwrap();
        let listed = workspace.def_types().unwrap();
        let root = workspace.file("workspace.gcsl").unwrap();
        let package = workspace.file("dev.peios.testpkg/test.gcsl").unwrap();
        let name = count(NEWPKGS[0].1.find("def package(").unwrap() + 4);
        assert_eq!(
            listed,
            [DefType {
                path: "package".to_owned(),
                file: root,
                span: Span::new(name, name + 7),
                parameters: Vec::new(),
                definitions: vec![Definition {
                    path: "dev.peios.testpkg".to_owned(),
                    file: package,
                    span: Span::new(12, 29),
                }],
            }]
        );
    }

    /// The paths of the def-types `workspace` lists, each with the paths
    /// and files of its definitions: `package: a.one in a/one.gcsl`.
    fn listed(workspace: &Workspace, files: &[(&str, &str)]) -> Vec<String> {
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        let standard = workspace.scan.standard as usize;
        let path = |file: u32| paths[file as usize - standard];
        workspace
            .def_types()
            .unwrap()
            .iter()
            .map(|def_type| {
                let definitions: Vec<String> = def_type
                    .definitions
                    .iter()
                    .map(|definition| format!("{} in {}", definition.path, path(definition.file)))
                    .collect();
                format!(
                    "{} in {}: {}",
                    def_type.path,
                    path(def_type.file),
                    definitions.join(", ")
                )
            })
            .collect()
    }

    #[test]
    fn every_definition_is_listed_under_the_def_type_it_names() {
        // Definitions in two sibling directories, which the root sees
        // neither of; one through an alias of its def-type, listed by its
        // path as written; a def-type with none; two def-types of the root
        // with definitions; and a dotted def-type.
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def build(name: String, f: fn(): String): String { \"built {f()}\" }\n\
                 #[def_type]\n\
                 def test(name: String, f: fn(): String): String { \"tested {f()}\" }",
            ),
            (
                "lib/types.gcsl",
                "#[def_type]\n\
                 def package(name: String, f: fn(): String) -> String { \"{name}: {f()}\" }\n\
                 #[def_type]\n\
                 def unused(name: String, f: fn(): Int): Int { f() }\n\
                 #[def_type]\n\
                 def kinds.tool(name: String, f: fn(): String): String { f() }",
            ),
            (
                "a/one.gcsl",
                "import * from lib.types\n\
                 def package a.one() { \"one\" }\n\
                 def build a.one() { \"a\" }\n\
                 def kinds.tool hammer() { \"hit\" }",
            ),
            (
                "b/two.gcsl",
                "import package as pkg from lib.types\n\
                 def pkg b.two() { \"two\" }\n\
                 def test b.two() { \"b\" }",
            ),
        ];
        let workspace = rooted(&files).unwrap();
        assert_eq!(
            listed(&workspace, &files),
            [
                "package in lib/types.gcsl: a.one in a/one.gcsl, b.two in b/two.gcsl",
                "unused in lib/types.gcsl: ",
                "kinds.tool in lib/types.gcsl: hammer in a/one.gcsl",
                "build in workspace.gcsl: a.one in a/one.gcsl",
                "test in workspace.gcsl: b.two in b/two.gcsl",
            ]
        );
        // The dotted def-type's name is its whole path.
        let tool = &workspace.def_types().unwrap()[2];
        let text = files[1].1;
        let at = count(text.find("kinds.tool").unwrap());
        assert_eq!(tool.span, Span::new(at, at + 10));
        // Each is invoked by its def-type's path and its own.
        assert_eq!(invoked_text(&files, "package", "a.one"), "a.one: one");
        assert_eq!(invoked_text(&files, "package", "b.two"), "b.two: two");
        assert_eq!(invoked_text(&files, "build", "a.one"), "built a");
        assert_eq!(invoked_text(&files, "test", "b.two"), "tested b");
        assert_eq!(invoked_text(&files, "kinds.tool", "hammer"), "hit");
    }

    /// The paths of the files of the workspace of `files`, by input number,
    /// the standard files `standard` first.
    fn all_paths<'a>(standard: &'a [SourceFile], files: &[(&'a str, &str)]) -> Vec<&'a str> {
        let mut paths: Vec<&str> = files.iter().map(|&(path, _)| path).collect();
        paths.sort_unstable();
        let mut all: Vec<&str> = standard.iter().map(SourceFile::name).collect();
        all.extend(paths);
        all
    }

    /// The errors [`Workspace::def_types`] fails with in the workspace of
    /// `files`, each shown as [`shown_errors`] shows it.
    fn listing_errors(files: &[(&str, &str)]) -> Vec<String> {
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|&(path, text)| SourceFile::new(path, text).unwrap())
            .collect();
        let (standard, checked) = check_with_standard(&sources, &[], SCRATCH);
        let errors = checked.unwrap().def_types().unwrap_err();
        shown_errors(&errors, &all_paths(&standard, files))
    }

    /// A def-type of the root, for the tests of duplicates.
    const PACKAGE: (&str, &str) = (
        "workspace.gcsl",
        "#[def_type]\ndef package(name: String, f: fn(): String): String { \"{name} {f()}\" }",
    );

    #[test]
    fn two_definitions_of_one_path_fail_every_invocation() {
        let defined = "note: a definition invoked by name must be the only one of its path \
                       in the workspace";
        // In two sibling directories, which see neither the other: invoking
        // another definition, unique, fails, and so does the listing.
        let siblings = [
            PACKAGE,
            ("a/x.gcsl", "def package dup() { \"a\" }"),
            ("b/y.gcsl", "def package dup() { \"b\" }"),
            (
                "c/z.gcsl",
                "def package other() { \"c\" }\nz = package other()",
            ),
        ];
        let twice = format!(
            "`package dup` is defined 2 times in the workspace at a/x.gcsl; \
             b/y.gcsl: also defined here; {defined}"
        );
        assert_eq!(
            invoke_errors(&siblings, "package", "other"),
            [twice.as_str()]
        );
        assert_eq!(listing_errors(&siblings), [twice.as_str()]);
        // An ordinary run allows them, and runs.
        assert_eq!(rooted_text(&siblings, "c/z.gcsl"), "{z = \"other c\"}");
        assert_eq!(rooted_text(&siblings, "a/x.gcsl"), "{}");
        // Three, each place labelled.
        let mut thrice = siblings.to_vec();
        thrice.push(("d/w.gcsl", "def package dup() { \"d\" }"));
        assert_eq!(
            invoke_errors(&thrice, "package", "dup"),
            [format!(
                "`package dup` is defined 3 times in the workspace at a/x.gcsl; \
                 b/y.gcsl: also defined here; d/w.gcsl: also defined here; {defined}"
            )]
        );
        // A `#[shadowable]` one of the root, shadowed in a directory below:
        // both are found, the first in path order first.
        let shadowed = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
                 #[shadowable] def package dup() { \"root\" }",
            ),
            (
                "sub/s.gcsl",
                "def package dup() { \"sub\" }\ny = package dup()",
            ),
        ];
        assert_eq!(
            invoke_errors(&shadowed, "package", "dup"),
            [format!(
                "`package dup` is defined 2 times in the workspace at sub/s.gcsl; \
                 workspace.gcsl: also defined here; {defined}"
            )]
        );
        assert_eq!(rooted_text(&shadowed, "sub/s.gcsl"), "{y = \"dup sub\"}");
        // And one shadowed in its own file, which is in no tree.
        let own = [
            PACKAGE,
            (
                "a.gcsl",
                "#[shadowable] def package dup() { \"one\" }\n\
                 def package dup() { \"two\" }\n\
                 y = package dup()",
            ),
        ];
        assert_eq!(
            invoke_errors(&own, "package", "dup"),
            [format!(
                "`package dup` is defined 2 times in the workspace at a.gcsl; \
                 a.gcsl: also defined here; {defined}"
            )]
        );
        assert_eq!(rooted_text(&own, "a.gcsl"), "{y = \"dup two\"}");
    }

    #[test]
    fn two_def_types_of_one_path_fail_every_invocation() {
        let declared = "note: a def-type invoked by name must be the only one of its name \
                        in the workspace";
        let def_type = "#[def_type]\n\
                        def package(name: String, f: fn(): String): String { f() }";
        // In two sibling directories.
        let siblings = [
            ("workspace.gcsl", ""),
            ("a/x.gcsl", def_type),
            ("b/y.gcsl", def_type),
            (
                "c/z.gcsl",
                "#[def_type]\ndef other(name: String, f: fn(): Int): Int { f() }",
            ),
            ("c/w.gcsl", "def other one() { 1 }\nw = other one()"),
        ];
        let twice = format!(
            "def-type `package` is declared 2 times in the workspace at a/x.gcsl; \
             b/y.gcsl: also declared here; {declared}"
        );
        assert_eq!(invoke_errors(&siblings, "other", "one"), [twice.as_str()]);
        assert_eq!(listing_errors(&siblings), [twice.as_str()]);
        assert_eq!(rooted_text(&siblings, "c/w.gcsl"), "{w = 1}");
        // A `#[shadowable]` one and the one that shadows it.
        let shadowed = [(
            "workspace.gcsl",
            "#[shadowable]\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { \"first\" }\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { f() }\n\
             def package x() { \"x\" }\n\
             y = package x()",
        )];
        assert_eq!(
            invoke_errors(&shadowed, "package", "x"),
            [format!(
                "def-type `package` is declared 2 times in the workspace at workspace.gcsl; \
                 workspace.gcsl: also declared here; {declared}"
            )]
        );
        assert_eq!(
            rooted_text(&shadowed, "workspace.gcsl"),
            "{package = fn(name: String | Future<String>, f: fn(): String): String; y = \"x\"}"
        );
    }

    #[test]
    fn a_definition_under_another_fails_every_invocation() {
        let note = "note: a definition invoked by name must not lie under another in the \
                    workspace";
        // In two sibling directories, which see neither the other: invoking
        // another definition, `apple`, which lies under nothing, fails, and
        // so does the listing.
        let siblings = [
            PACKAGE,
            ("a/x.gcsl", "def package app() { \"a\" }"),
            (
                "b/y.gcsl",
                "def package app.core() { \"b\" }\ny = package app.core()",
            ),
            ("c/z.gcsl", "def package apple() { \"c\" }"),
        ];
        let under = format!(
            "`package app.core` lies under another `package` definition at b/y.gcsl; \
             a/x.gcsl: `app` is defined here; {note}"
        );
        assert_eq!(
            invoke_errors(&siblings, "package", "apple"),
            [under.as_str()]
        );
        assert_eq!(listing_errors(&siblings), [under.as_str()]);
        // An ordinary run allows them, and runs.
        assert_eq!(rooted_text(&siblings, "b/y.gcsl"), "{y = \"app.core b\"}");
        // In one directory, through `#[shadowable]`.
        let shadowed = [
            PACKAGE,
            (
                "a.gcsl",
                "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }\n\
                 y = package app()",
            ),
        ];
        assert_eq!(
            listing_errors(&shadowed),
            [format!(
                "`package app.core` lies under another `package` definition at a.gcsl; \
                 a.gcsl: `app` is defined here; {note}"
            )]
        );
        assert_eq!(rooted_text(&shadowed, "a.gcsl"), "{y = \"app app\"}");
        // Three levels: `a.b` lies under one, `a.b.c` under two, labelled in
        // path then source order.
        let levels = [
            PACKAGE,
            ("a/one.gcsl", "def package a.b.c() { \"c\" }"),
            ("b/two.gcsl", "def package a.b() { \"b\" }"),
            ("c/three.gcsl", "def package a() { \"a\" }"),
        ];
        assert_eq!(
            listing_errors(&levels),
            [
                format!(
                    "`package a.b.c` lies under another `package` definition at a/one.gcsl; \
                     b/two.gcsl: `a.b` is defined here; c/three.gcsl: `a` is defined here; \
                     {note}"
                ),
                format!(
                    "`package a.b` lies under another `package` definition at b/two.gcsl; \
                     c/three.gcsl: `a` is defined here; {note}"
                ),
            ]
        );
        // Definitions of other def-types do not overlap: `build a.b.c` lies
        // under no `package` definition.
        let other = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def build(name: String, f: fn(): String): String { f() }",
            ),
            ("a/one.gcsl", "def build a.b.c() { \"c\" }"),
            levels[2],
            levels[3],
        ];
        assert_eq!(
            listing_errors(&other),
            [format!(
                "`package a.b` lies under another `package` definition at b/two.gcsl; \
                 c/three.gcsl: `a` is defined here; {note}"
            )]
        );
    }

    #[test]
    fn a_definition_both_duplicated_and_under_another_has_both_errors() {
        let defined = "note: a definition invoked by name must be the only one of its path \
                       in the workspace";
        let note = "note: a definition invoked by name must not lie under another in the \
                    workspace";
        // `app` twice, and `app.core` twice under it: each `app.core` lies
        // under both `app`s.
        let files = [
            PACKAGE,
            ("a/x.gcsl", "def package app() { \"a\" }"),
            ("b/y.gcsl", "def package app.core() { \"b\" }"),
            ("c/z.gcsl", "def package app.core() { \"c\" }"),
            ("d/w.gcsl", "def package app() { \"d\" }"),
        ];
        let under = |at: &str| {
            format!(
                "`package app.core` lies under another `package` definition at {at}; \
                 a/x.gcsl: `app` is defined here; d/w.gcsl: `app` is defined here; {note}"
            )
        };
        let expected = [
            format!(
                "`package app` is defined 2 times in the workspace at a/x.gcsl; \
                 d/w.gcsl: also defined here; {defined}"
            ),
            format!(
                "`package app.core` is defined 2 times in the workspace at b/y.gcsl; \
                 c/z.gcsl: also defined here; {defined}"
            ),
            under("b/y.gcsl"),
            under("c/z.gcsl"),
        ];
        assert_eq!(listing_errors(&files), expected);
        assert_eq!(invoke_errors(&files, "package", "app"), expected);
    }

    #[test]
    fn what_cannot_be_invoked_is_a_request_error() {
        let files = [
            BUILD,
            ("app.gcsl", "def build x(t) { t }"),
            (
                "lib/defs.gcsl",
                "#[def_type]\n\
                 def package(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def empty(name: String, f: fn(): String): String { f() }\n\
                 #[def_type]\n\
                 def deploy(name: String, f: fn(): String, target: String, mode: String): String { f() }\n\
                 def package dev.peios.b() { \"b\" }\n\
                 def package dev.peios.a() { \"a\" }\n\
                 def deploy site() { \"s\" }",
            ),
        ];
        assert_eq!(
            request_error(&files, "pkg", "x"),
            "no def-type `pkg`; note: the def-types are `build`, `deploy`, `empty`, `package`"
        );
        assert_eq!(
            request_error(&[("workspace.gcsl", "x = 1")], "pkg", "x"),
            "no def-type `pkg`; note: this workspace declares no def-types"
        );
        assert_eq!(
            request_error(&files, "package", "dev.peios.nope"),
            "`package` has no definition `dev.peios.nope`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
        );
        assert_eq!(
            request_error(&files, "empty", "x"),
            "`empty` has no definition `x`; note: it has no definitions"
        );
        assert_eq!(
            request_error(&files, "package", "dev.peios"),
            "`dev.peios` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
        );
        assert_eq!(
            request_error(&files, "package", "dev"),
            "`dev` is not a `package` definition; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
        );
        // A prefix of a name is no path above it.
        assert_eq!(
            request_error(&files, "package", "dev.pei"),
            "`package` has no definition `dev.pei`; \
             note: its definitions are `dev.peios.a`, `dev.peios.b`"
        );
        assert_eq!(
            request_error(&files, "build", "x"),
            "`build` takes 1 argument (`mode`), but 0 were given"
        );
        assert_eq!(
            request_error(&files, "deploy", "site"),
            "`deploy` takes 2 arguments (`target`, `mode`), but 0 were given"
        );
        // Before parameters, the path.
        assert_eq!(
            request_error(&files, "build", "y"),
            "`build` has no definition `y`; note: its only definition is `x`"
        );
        // A path that is a definition's, and above another's, in a sibling
        // directory: the one below lies under it, and neither is invoked.
        let under = "`package app.core` lies under another `package` definition";
        let note = "note: a definition invoked by name must not lie under another in the \
                    workspace";
        let files = [
            PACKAGE,
            ("a/x.gcsl", "def package app() { \"a\" }"),
            ("b/y.gcsl", "def package app.core() { \"b\" }"),
        ];
        let siblings = format!("{under} at b/y.gcsl; a/x.gcsl: `app` is defined here; {note}");
        assert_eq!(invoke_errors(&files, "package", "app"), [siblings.as_str()]);
        assert_eq!(
            invoke_errors(&files, "package", "app.core"),
            [siblings.as_str()]
        );
        // A `#[shadowable]` definition that one of another path, above it,
        // shadows in its own directory, which is in no tree: found, and the
        // one below.
        let files = [
            PACKAGE,
            (
                "a.gcsl",
                "#[shadowable] def package app.core() { \"core\" }\n\
                 def package app() { \"app\" }",
            ),
        ];
        let shadowed = format!("{under} at a.gcsl; a.gcsl: `app` is defined here; {note}");
        assert_eq!(
            invoke_errors(&files, "package", "app.core"),
            [shadowed.as_str()]
        );
        assert_eq!(invoke_errors(&files, "package", "app"), [shadowed.as_str()]);
    }

    #[test]
    fn a_request_error_names_the_only_one_there_is() {
        let files = [
            PACKAGE,
            ("lib/defs.gcsl", "def package dev.peios.a() { \"a\" }"),
        ];
        assert_eq!(
            request_error(&files, "pkg", "x"),
            "no def-type `pkg`; note: the only def-type is `package`"
        );
        assert_eq!(
            request_error(&files, "package", "dev.peios.nope"),
            "`package` has no definition `dev.peios.nope`; \
             note: its only definition is `dev.peios.a`"
        );
        assert_eq!(
            request_error(&files, "package", "dev.peios"),
            "`dev.peios` is not a `package` definition; \
             note: its only definition is `dev.peios.a`"
        );
        // Two or more are listed, as `what_cannot_be_invoked_is_a_request_error`
        // shows with more.
        let files = [
            BUILD,
            ("lib/defs.gcsl", PACKAGE.1),
            (
                "lib/more.gcsl",
                "def package a() { \"a\" }\ndef package b() { \"b\" }",
            ),
        ];
        assert_eq!(
            request_error(&files, "pkg", "x"),
            "no def-type `pkg`; note: the def-types are `build`, `package`"
        );
        assert_eq!(
            request_error(&files, "package", "c"),
            "`package` has no definition `c`; note: its definitions are `a`, `b`"
        );
    }

    /// A def-type of one invocation parameter, [`BUILD`]'s `build`, one of
    /// two, `deploy`, and one of none, `package`, with a definition of
    /// each.
    const ARGUMENTS: [(&str, &str); 3] = [
        BUILD,
        ("app.gcsl", "def build x(t) { t }"),
        (
            "lib/defs.gcsl",
            "#[def_type]\n\
             def deploy(name: String, f: fn(): String, target: String, mode: String): String {\n\
             \x20   \"{name} {f()} {target} {mode}\"\n\
             }\n\
             #[def_type]\n\
             def package(name: String, f: fn(): String): String { \"{name} {f()}\" }\n\
             def deploy site() { \"s\" }\n\
             def package p() { \"p\" }",
        ),
    ];

    #[test]
    fn invoking_gives_its_arguments_as_strings() {
        // One argument, two, and none, in the order of the parameters.
        let files = ARGUMENTS;
        assert_eq!(
            invoked_text_with(&files, ("build", "x"), &["debug"]),
            "x: debug"
        );
        assert_eq!(
            invoked_text_with(&files, ("deploy", "site"), &["prod", "fast"]),
            "site s prod fast"
        );
        assert_eq!(invoked_text_with(&files, ("package", "p"), &[]), "p p");
        // Each a string literal as it is, nothing in it read as GCSL.
        assert_eq!(
            invoked_text_with(&files, ("build", "x"), &["{mode} \\ \" -1"]),
            "x: {mode} \\ \" -1"
        );
        assert_eq!(invoked_text_with(&files, ("build", "x"), &[""]), "x: ");
        // Listed, the def-types' invocation parameters, as declared.
        let mut workspace = rooted(&files).unwrap();
        let parameters: Vec<(String, Vec<String>)> = workspace
            .def_types()
            .unwrap()
            .into_iter()
            .map(|def_type| (def_type.path, def_type.parameters))
            .collect();
        let owned = |names: &[&str]| names.iter().map(|&name| name.to_owned()).collect();
        assert_eq!(
            parameters,
            [
                ("deploy".to_owned(), owned(&["target", "mode"])),
                ("package".to_owned(), owned(&[])),
                ("build".to_owned(), owned(&["mode"])),
            ]
        );
        // Invoked again on one workspace, with other arguments.
        for mode in ["a", "b"] {
            let value = workspace.invoke("build", "x", &[mode.to_owned()]);
            assert_eq!(value.unwrap().value.to_string(), format!("x: {mode}"));
        }
    }

    #[test]
    fn the_arguments_must_be_as_many_as_the_parameters() {
        let files = ARGUMENTS;
        let error = |def_type: &str, path: &str, args: &[&str]| {
            request_error_with(&files, (def_type, path), args)
        };
        assert_eq!(
            error("build", "x", &[]),
            "`build` takes 1 argument (`mode`), but 0 were given"
        );
        assert_eq!(
            error("build", "x", &["a", "b"]),
            "`build` takes 1 argument (`mode`), but 2 were given"
        );
        assert_eq!(
            error("deploy", "site", &["a"]),
            "`deploy` takes 2 arguments (`target`, `mode`), but 1 was given"
        );
        assert_eq!(
            error("deploy", "site", &["a", "b", "c"]),
            "`deploy` takes 2 arguments (`target`, `mode`), but 3 were given"
        );
        assert_eq!(
            error("package", "p", &["a"]),
            "`package` takes no arguments, but 1 was given"
        );
        assert_eq!(
            error("package", "p", &["a", "b"]),
            "`package` takes no arguments, but 2 were given"
        );
        // Before the arguments, the path.
        assert_eq!(
            error("build", "y", &["a", "b"]),
            "`build` has no definition `y`; note: its only definition is `x`"
        );
    }

    #[test]
    fn only_string_parameters_can_be_given() {
        let files = [
            (
                "workspace.gcsl",
                "type Version = { version: String }\n\
                 type Name = String\n\
                 #[def_type]\n\
                 def count(name: String, f: fn(): String, n: Int): String { f() }\n\
                 #[def_type]\n\
                 def mixed(name: String, f: fn(): String, a: String, b: List<String>, c: Int): String { f() }\n\
                 #[def_type]\n\
                 def versioned(name: String, f: fn(): String, v: Version): String { f() }\n\
                 #[def_type]\n\
                 def later(name: String, f: fn(): String, s: Future<String>) -> String { f() }\n\
                 #[def_type]\n\
                 def fits(name: String, f: fn(): String, n: Name, s: String | Int): String {\n\
                 \x20   \"{f()} {n} {s}\"\n\
                 }",
            ),
            (
                "defs.gcsl",
                "def count x() { \"x\" }\n\
                 def mixed x() { \"x\" }\n\
                 def versioned x() { \"x\" }\n\
                 def later x() { \"x\" }\n\
                 def fits x() { \"x\" }",
            ),
        ];
        let error =
            |def_type: &str, args: &[&str]| request_error_with(&files, (def_type, "x"), args);
        // Before the number of the arguments, whatever it is.
        let count = "`count`'s parameter `n` is `Int`, but only `String` parameters can be \
                     given here yet";
        assert_eq!(error("count", &[]), count);
        assert_eq!(error("count", &["1"]), count);
        assert_eq!(error("count", &["1", "2"]), count);
        // The first that cannot be, its type as it is written.
        assert_eq!(
            error("mixed", &["a", "b", "c"]),
            "`mixed`'s parameter `b` is `List<String>`, but only `String` parameters can be \
             given here yet"
        );
        assert_eq!(
            error("versioned", &["1"]),
            "`versioned`'s parameter `v` is `Version`, but only `String` parameters can be \
             given here yet"
        );
        // A present string fits no future.
        assert_eq!(
            error("later", &["1"]),
            "`later`'s parameter `s` is `Future<String>`, but only `String` parameters can be \
             given here yet"
        );
        // An alias of `String`, and a union with it, a string fits.
        assert_eq!(
            invoked_text_with(&files, ("fits", "x"), &["n", "s"]),
            "x n s"
        );
    }

    #[test]
    fn a_bare_string_parameter_of_an_impure_def_type_is_given() {
        // `version: String` of an impure def-type is `String |
        // Future<String>`, which a string fits; of a memoised one, `String`.
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def run(name: String, f: fn(v: String) -> String, version: String) -> String {\n\
                 \x20   f(version)\n\
                 }\n\
                 #[def_type]\n\
                 def once(name: String, f: fn(v: String) -> String, version: String) ->> String {\n\
                 \x20   f(version)\n\
                 }",
            ),
            (
                "pkg/p.gcsl",
                "def run p(v) { \"run p at {v}\" }\ndef once p(v) { \"once p at {v}\" }",
            ),
        ];
        assert_eq!(
            invoked_text_with(&files, ("run", "p"), &["0.1.0"]),
            "run p at 0.1.0"
        );
        assert_eq!(
            invoked_text_with(&files, ("once", "p"), &["0.2.0"]),
            "once p at 0.2.0"
        );
        let workspace = rooted(&files).unwrap();
        let shown: Vec<String> = workspace
            .found_def_types()
            .iter()
            .map(|found| {
                let (_, ty) = workspace.leaf_cell(found.listed.file, found.item).unwrap();
                let sig = workspace.types.fn_sig(ty).unwrap();
                let version = workspace
                    .types
                    .display(sig.params[2].1, &workspace.scan.names);
                format!("{}: {version}", found.listed.path)
            })
            .collect();
        assert_eq!(shown, ["run: String | Future<String>", "once: String"]);
    }

    #[test]
    fn an_invoked_definition_is_called_as_from_its_directory() {
        // The def-type's caller is the definition's directory; without
        // `fileroot.helper()` in the def-type, the definition's is the
        // def-type's file's directory, as for any call.
        let files = [
            (
                "workspace.gcsl",
                "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn() -> String) -> { called: String, defined: String } {\n\
                 \x20   called = fileroot.caller\n\
                 \x20   defined = f()\n\
                 }",
            ),
            (
                "pkg/deep/p.gcsl",
                "import fileroot\ndef where x() { fileroot.caller }",
            ),
        ];
        assert_eq!(
            invoked_text(&files, "where", "x"),
            "{called = \"<root>/pkg/deep\"; defined = \"<root>\"}"
        );
    }

    #[test]
    fn invoking_runs_the_files_once_and_the_call_each_time() {
        // A log the definition's file writes to as it runs, and each call of
        // the impure def-type, and of the memoised one, once for its
        // arguments.
        let dir = std::env::temp_dir().join(format!("gcsl-invoke-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("log").display().to_string();
        let def_types = format!(
            "#[def_type]\n\
             def logged(name: String, f: fn(): String) -> String {{\n\
             \x20   let out = sh(\"echo {{name}} >> {log}\")\n\
             \x20   \"{{out}}{{f()}}\"\n\
             }}\n\
             #[def_type]\n\
             def cached(name: String, f: fn(): String) ->> String {{\n\
             \x20   let out = sh(\"echo cached {{name}} >> {log}\")\n\
             \x20   \"{{out}}{{f()}}\"\n\
             }}\n\
             #[def_type]\n\
             def pure(name: String, f: fn(): String): String {{ \"pure {{f()}}\" }}"
        );
        let package = format!(
            "x = sh(\"echo file >> {log}\")\n\
             def logged p() {{ \"l\" }}\n\
             def cached p() {{ \"c\" }}\n\
             def pure p() {{ \"p\" }}"
        );
        let files = [
            ("workspace.gcsl", def_types.as_str()),
            ("pkg/p.gcsl", package.as_str()),
        ];
        let mut workspace = rooted(&files).unwrap();
        let mut invoke = |def_type: &str| {
            workspace
                .invoke(def_type, "p", &[])
                .unwrap()
                .value
                .to_string()
        };
        for _ in 0..2 {
            assert_eq!(invoke("logged"), "l");
            assert_eq!(invoke("cached"), "c");
            assert_eq!(invoke("pure"), "pure p");
        }
        let written = std::fs::read_to_string(&log).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, "file\np\ncached p\np\n");
    }

    #[test]
    fn a_runtime_error_of_an_invoked_definition_names_its_file() {
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\ndef run(name: String, f: fn() -> String) -> String { f() }",
            ),
            ("pkg/p.gcsl", "def run p() { panic(\"boom\") }"),
        ];
        assert_eq!(invoke_errors(&files, "run", "p"), ["boom at pkg/p.gcsl"]);
        // A file that runs first fails as `run` reports it.
        let files = [
            files[0],
            ("pkg/p.gcsl", "def run p() { \"p\" }\nx = [1][3]"),
        ];
        assert_eq!(
            invoke_errors(&files, "run", "p"),
            ["index out of range at pkg/p.gcsl (the index is 3, and the list has 1 element)"]
        );
    }

    #[test]
    fn an_error_of_the_call_itself_is_at_the_definition_s_path() {
        // A def-type whose body is nested so deeply that a call of it from
        // a top level, as the invocation is, takes the nesting past the
        // limit: "call nested too deeply", at the call, which has no source.
        let depth = crate::MAX_DEPTH - 4;
        let body = format!("1{}", " + 1".repeat(depth - 1));
        let def_type =
            format!("#[def_type]\ndef deep(name: String, f: fn(): Int): Int {{ {body} }}");
        let (shown, errors) = std::thread::Builder::new()
            .stack_size(crate::STACK_SIZE)
            .spawn(move || {
                let files = [
                    ("workspace.gcsl", def_type.as_str()),
                    ("pkg/p.gcsl", "def deep p() { 1 }"),
                ];
                let shown = invoke_errors(&files, "deep", "p");
                let mut workspace = rooted(&files).unwrap();
                let Err(InvokeError::Diagnostics(errors)) = workspace.invoke("deep", "p", &[])
                else {
                    panic!("no error");
                };
                (shown, errors)
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            shown,
            [
                "call nested too deeply: the limit is 10000 levels of nesting, calls included \
                 at pkg/p.gcsl (invoked here); note: every level of expression nesting around \
                 a call counts, and so does the body of the function called; GCSL does not \
                 eliminate tail calls, so recursion this deep is not supported"
            ]
        );
        // At the definition's path, `p`, with nothing elsewhere.
        assert_eq!(errors[0].span, Span::new(9, 10));
        assert!(errors[0].labels.is_empty());
    }

    /// A def-type of the root of a workspace, for the metas' tests:
    /// `tagged`, whose definitions give a string and carry a pure meta of a
    /// tag and a number, read through functions that take less of it: one
    /// whose meta has the tag alone, and one with no meta.
    #[test]
    fn a_field_of_a_definition_s_meta_is_read_by_name() {
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def package(name: String, f: fn(v: String): String META-> {versions: List<String>}, \
                 version: String) -> String { f(version) }\n\
                 #[def_type]\n\
                 def plain(name: String, f: fn(): String): String { f() }",
            ),
            (
                "app/a.gcsl",
                "defmeta package dev.x { versions = [\"0.1.0\", \"0.2.0\"] }\n\
                 def package dev.x(v) { v }\n\
                 def plain y() { \"y\" }",
            ),
        ];
        let mut workspace = rooted(&files).unwrap();
        // Read from the definition's directory, its `defmeta` run.
        let versions = workspace.meta_field("package", "dev.x", "versions");
        assert_eq!(
            versions.unwrap().unwrap().value.to_string(),
            "[\"0.1.0\", \"0.2.0\"]"
        );
        // No such field, and no meta: nothing to read.
        assert!(
            workspace
                .meta_field("package", "dev.x", "nope")
                .unwrap()
                .is_none()
        );
        assert!(
            workspace
                .meta_field("plain", "y", "versions")
                .unwrap()
                .is_none()
        );
        // What cannot be invoked cannot be read either.
        assert_eq!(
            workspace
                .meta_field("package", "dev.nope", "versions")
                .unwrap_err(),
            InvokeError::Request(RequestError::NoDefinition {
                def_type: "package".to_owned(),
                path: "dev.nope".to_owned(),
                definitions: vec!["dev.x".to_owned()],
            })
        );
        // Invoking is unchanged.
        let value = workspace.invoke("package", "dev.x", &["0.1.0".to_owned()]);
        assert_eq!(value.unwrap().value.to_string(), "0.1.0");
    }

    const TAGGED: (&str, &str) = (
        "workspace.gcsl",
        "#[def_type]\n\
         def tagged(name: String, f: fn(): String META: {tag: String, n: Int}): String {\n\
         \x20   \"{name}: {plain(f)} {tag(f)} {f.n}\"\n\
         }\n\
         fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
         fn plain(g: fn(): String): String { g() }",
    );

    #[test]
    fn a_meta_is_a_leaf_of_its_directory_s_metas() {
        // The meta's leaf is its path as one name, under the meta key of its
        // def-type and directory; the definition and the meta are top-level
        // functions, and the definition's item mentions the meta's leaf and
        // function, by its read of its meta.
        let files = [
            TAGGED,
            (
                "app/a.gcsl",
                "def tagged dev.x() { \"x\" }\ndefmeta tagged dev.x { tag = \"t\"; n = 1 }",
            ),
        ];
        let scan = ws(&files).unwrap();
        let app = scan.files.iter().find(|f| f.path == "app/a.gcsl").unwrap();
        let tree = &app.mentions.tree;
        let meta_root = &tree.nodes[tree.roots[1] as usize];
        let key = scan.names.get(tree.nodes[tree.roots[0] as usize].name);
        assert_eq!(
            Some(scan.names.get(meta_root.name)),
            names::meta_key(key, app.dir).as_deref()
        );
        let leaf = &tree.nodes[meta_root.children[0] as usize];
        assert_eq!(scan.names.get(leaf.name), "dev.x");
        assert!(leaf.children.is_empty());
        assert_eq!(app.mentions.functions.len(), 2);
        assert_eq!(app.mentions.items[0], [1]);
        assert!(app.mentions.fields[0].contains(&meta_root.children[0]));
        assert!(app.unpaired.is_empty());
    }

    #[test]
    fn a_meta_is_read_as_fields_of_its_definition() {
        // The meta above or below its definition, in its file or in another
        // of its directory, a block or an expression; read in pure functions,
        // one whose meta's type is less than the definition's and one that
        // forgets the meta.
        let files = [
            TAGGED,
            (
                "app/a.gcsl",
                "defmeta tagged x { tag = \"t-x\"; n = 1 }\n\
                 def tagged x() { \"x\" }\n\
                 def tagged y() { \"y\" }\n\
                 defmeta tagged y = { tag = \"t-y\"; n = 2 }",
            ),
            (
                "app/b.gcsl",
                "defmeta tagged dev.z { tag = \"t-z\"; n = 3 }",
            ),
            (
                "app/c.gcsl",
                "def tagged dev.z() { \"z\" }\n\
                 r = [tagged x(), tagged y(), tagged dev.z()]",
            ),
        ];
        assert_eq!(
            value(&files, "app/c.gcsl"),
            "{r = [\"x: x t-x 1\", \"y: y t-y 2\", \"dev.z: z t-z 3\"]}"
        );
        // A def-type with no meta works as it always did.
        let files = [
            BUILD,
            ("app.gcsl", "def build x(t) { t }\nr = build x(\"m\")"),
        ];
        assert_eq!(value(&files, "app.gcsl"), "{r = \"x: m\"}");
    }

    #[test]
    fn each_marker_reads_its_meta_as_its_effect() {
        let path = std::env::temp_dir().join(format!("gcsl-slice-38-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
        let _ = std::fs::remove_file(path);
        // `META->` runs its body at each read, `META->>` once for the run.
        let impure = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META-> {n: Int}) -> List<Int> { [f.n, f.n] }";
        let app = format!(
            "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()"
        );
        let files = [("workspace.gcsl", impure), ("app.gcsl", app.as_str())];
        assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
        assert_eq!(count(), 2);
        let memo = impure.replace("META->", "META->>");
        let files = [
            ("workspace.gcsl", memo.as_str()),
            ("app.gcsl", app.as_str()),
        ];
        assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]}");
        assert_eq!(count(), 3);
        std::fs::remove_file(path).unwrap();
        // A `META: Future<...>` meta is read purely, giving a future, which
        // `defmeta ... = lazy` makes: its field is awaited, which only
        // impure code may.
        let future = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def later(name: String, f: fn(): Int META: Future<{v: Int}>) -> Int {\n\
                 \x20   f.v + f()\n\
                 }",
            ),
            (
                "app.gcsl",
                "defmeta later x = lazy { v = 41 }\ndef later x() { 1 }\nr = later x()",
            ),
        ];
        assert_eq!(value(&future, "app.gcsl"), "{r = 42}");
        let pure = future[0].1.replace(") -> Int", "): Int");
        let files = [("workspace.gcsl", pure.as_str()), future[1]];
        assert_eq!(
            check_errors(&files),
            [
                "cannot use a value that may be a future in a pure function at workspace.gcsl \
                 (this is Future<{v: Int}>); workspace.gcsl: this function is pure, declared \
                 with `:`; note: using the value here would await it, which may run effects; a \
                 pure function may hold and pass on a future but not await it. Declare the \
                 function with `->` to make it impure"
            ]
        );
        // Under `META->`, a field that may be a future is awaited where it is
        // used.
        let lazy = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def sum(name: String, f: fn(): Int META-> {ns: List<Int>}) -> Int {\n\
                 \x20   let mut total = f()\n\
                 \x20   for n in f.ns { total := total + n }\n\
                 \x20   total\n\
                 }",
            ),
            (
                "app.gcsl",
                "defmeta sum x { ns = lazy [1, 2, lazy 3] }\ndef sum x() { 4 }\nr = sum x()",
            ),
        ];
        assert_eq!(value(&lazy, "app.gcsl"), "{r = 10}");
    }

    #[test]
    fn a_meta_s_caller_is_its_reader_s() {
        // The meta's `fileroot.this` is its own file's directory, and its
        // caller the directory of the code that reads it: the def-type's, or
        // through its `fileroot.helper()`, the def-type's own caller's.
        let files = [
            (
                "workspace.gcsl",
                "import fileroot\n\
                 #[def_type]\n\
                 def where(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }\n\
                 #[def_type]\n\
                 def helped(name: String, f: fn(): String META-> {this: String, caller: String}) \
                 -> String {\n\
                 \x20   fileroot.helper()\n\
                 \x20   \"{f.this} {f.caller}\"\n\
                 }",
            ),
            (
                "a/a.gcsl",
                "import fileroot\n\
                 defmeta where x { this = fileroot.this; caller = fileroot.caller }\n\
                 def where x() { \"\" }\n\
                 defmeta helped x { this = fileroot.this; caller = fileroot.caller }\n\
                 def helped x() { \"\" }",
            ),
            ("a/sub/c.gcsl", "r = [where x(), helped x()]"),
        ];
        assert_eq!(
            rooted_text(&files, "a/sub/c.gcsl"),
            "{r = [\"<root>/a <root>\", \"<root>/a <root>/a/sub\"]}"
        );
    }

    #[test]
    fn a_meta_pairs_with_its_definition_in_its_directory() {
        // A `#[shadowable]` definition shadowed in a subdirectory, which
        // brings its own meta: each view has its own pair.
        let files = [
            (
                "workspace.gcsl",
                format!(
                    "{}\n#[shadowable]\ndef tagged s() {{ \"top\" }}\n\
                     defmeta tagged s {{ tag = \"top-meta\"; n = 0 }}",
                    TAGGED.1
                ),
            ),
            ("root.gcsl", "r = tagged s()".to_owned()),
            (
                "sub/a.gcsl",
                "def tagged s() { \"sub\" }\n\
                 defmeta tagged s { tag = \"sub-meta\"; n = 1 }\n\
                 below = tagged s()"
                    .to_owned(),
            ),
        ];
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        assert_eq!(value(&files, "root.gcsl"), "{r = \"s: top top-meta 0\"}");
        assert_eq!(
            value(&files, "sub/a.gcsl"),
            "{below = \"s: sub sub-meta 1\"}"
        );
        // A chain of `#[shadowable]` definitions in one directory has one
        // meta.
        let files = [
            TAGGED,
            ("app/a.gcsl", "#[shadowable]\ndef tagged c() { \"first\" }"),
            (
                "app/b.gcsl",
                "def tagged c() { \"second\" }\n\
                 defmeta tagged c { tag = \"one\"; n = 1 }\n\
                 r = tagged c()",
            ),
        ];
        assert_eq!(value(&files, "app/b.gcsl"), "{r = \"c: second one 1\"}");
        // A meta in a subdirectory of its definition is no meta of it.
        let files = [
            TAGGED,
            ("app/a.gcsl", "def tagged x() { \"x\" }"),
            ("app/sub/b.gcsl", "defmeta tagged x { tag = \"t\"; n = 1 }"),
        ];
        assert_eq!(
            check_errors(&files),
            ["`defmeta tagged x` has no definition `x` in its directory at app/sub/b.gcsl"]
        );
        let files = [TAGGED, ("app/a.gcsl", "def tagged x() { \"x\" }")];
        assert_eq!(
            check_errors(&files),
            [
                "`tagged x` has no `defmeta` in its directory at app/a.gcsl; note: `tagged` \
                 declares a meta, `META: {tag: String, n: Int}`, for each of its definitions"
            ]
        );
    }

    #[test]
    fn a_meta_is_invoked_with_its_definition() {
        // The meta in another file of the definition's directory: invoking
        // the definition by name runs that file too.
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def tag(name: String, f: fn(): String META->> {t: String}) -> String {\n\
                 \x20   \"{f()} {f.t}\"\n\
                 }",
            ),
            ("pkg/a.gcsl", "defmeta tag p { t = \"meta\" }"),
            ("pkg/p.gcsl", "def tag p() { \"p\" }"),
        ];
        assert_eq!(invoked_text(&files, "tag", "p"), "p meta");
    }

    #[test]
    fn meta_errors() {
        // Each error with its labels and notes, joined by `; `, the errors
        // by newlines.
        let errors = |text: &str| {
            let files = [TAGGED, ("app.gcsl", text)];
            check_errors(&files).join("\n")
        };
        assert_eq!(
            errors("x: fn(): Int META: Int = 1"),
            "a meta must be a record type: its fields are read from the function at app.gcsl"
        );
        assert_eq!(
            errors("x: fn(): Int META-> Future<List<Int>> = 1"),
            "a meta must be a record type: its fields are read from the function at app.gcsl"
        );
        assert_eq!(
            errors("fn f(g: fn(): Int META: {a: Int, b: Int}): Int { g.c }"),
            "the meta of `g` has no field `c` at app.gcsl; note: the fields are `a` and `b`"
        );
        assert_eq!(
            errors("fn f(g: fn(): Int META-> {a: Int}): Int {\nlet held = g.a\n1\n}"),
            "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: reading the \
             meta of `g` is impure (`META->`); app.gcsl: this function is pure, declared with \
             `:`; note: a pure function, declared with `:`, may read only pure metas, `META:`; \
             declare it with `->` to make it impure"
        );
        // In a memoised function, as calling an impure function is.
        let memo = [
            TAGGED,
            (
                "app.gcsl",
                "fn f(g: fn(): Int META-> {a: Int}) ->> Int { g.a }\n\
                 fn h(g: fn(): Int META->> {a: Int}) -> Int { g.a }",
            ),
        ];
        assert_eq!(value(&memo, "app.gcsl"), "{}");
        // A function with no meta has no fields, as before.
        assert_eq!(
            errors("fn f(g: fn(): Int): Int { g.a }"),
            "cannot access field `a` of `fn(): Int` at app.gcsl; app.gcsl: this is fn(): Int"
        );
        // A function with no meta, or with a meta of a greater marker, does
        // not fit a type with one.
        assert_eq!(
            errors("fn f(g: fn(): Int META: {a: Int}): Int { g.a }\nx = f(fn(): Int { 1 })"),
            "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int); \
             app.gcsl: the function is fn(g: fn(): Int META: {a: Int}): Int"
        );
        assert_eq!(
            errors(
                "fn f(g: fn(): Int META: {a: Int}): Int { g.a }\n\
                 fn h(g: fn(): Int META-> {a: Int}): Int { f(g) }"
            ),
            "mismatched types at app.gcsl (expected fn(): Int META: {a: Int}, found fn(): Int \
             META-> {a: Int | Future<Int>} | Future<{a: Int | Future<Int>}>); app.gcsl: the \
             function is fn(g: fn(): Int META: {a: Int}): Int"
        );
        // The pairing.
        assert_eq!(
            errors(
                "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = \"a\"; n = 1 }\n\
                 defmeta tagged x { tag = \"b\"; n = 2 }\n\
                 defmeta tagged y { tag = \"c\"; n = 3 }"
            ),
            "`defmeta tagged x` is declared twice in this directory at app.gcsl; app.gcsl: \
             first declared here\n\
             `defmeta tagged y` has no definition `y` in its directory at app.gcsl"
        );
        assert_eq!(
            errors(
                "#[def_type]\ndef plain(name: String, f: fn(): Int): Int { f() }\n\
                 def plain p() { 1 }\n\
                 defmeta plain p { tag = \"x\" }"
            ),
            "`plain` declares no meta for its definitions at app.gcsl; note: a def-type \
             declares one by its definition parameter's type, as in `func: fn(): PackageDef \
             META: PackageMeta`"
        );
        assert_eq!(
            errors("defmeta nope x { a = 1 }"),
            "undefined variable `nope` at app.gcsl"
        );
        // The meta's body is checked against the meta's type, with its marker.
        assert_eq!(
            errors("def tagged x() { \"x\" }\ndefmeta tagged x { tag = 1; n = 1 }"),
            "mismatched types at app.gcsl (expected String, found Int); app.gcsl: the return \
             type is declared here"
        );
        assert_eq!(
            errors(
                "def tagged x() { \"x\" }\n\
                 defmeta tagged x { tag = exec([\"true\"]).stdout; n = 1 }"
            ),
            "cannot call an impure function from a pure function at app.gcsl; app.gcsl: this \
             is fn(argv: List<String>) -> {status: Int, stderr: String, stdout: String}; \
             app.gcsl: this function is pure, declared with `:`; note: a pure function, \
             declared with `:`, may call only pure functions; declare it with `->` to make it \
             impure"
        );
        // A definition needs its meta: one that reads a variable bound later
        // is shown as written in the chain.
        let found = errors(
            "r = tagged x()\nlet k = 1\ndefmeta tagged x { tag = \"t\"; n = k }\n\
             def tagged x() { \"x\" }",
        );
        assert!(
            found.contains("uses `defmeta tagged x`, and `defmeta tagged x` reads `k`"),
            "{found}"
        );
    }

    /// A def-type of the root of a workspace, for the tests of reading a
    /// meta through a reference: `info`, whose definitions give a number and
    /// carry a pure meta of a record, a list and a string.
    const INFO: (&str, &str) = (
        "workspace.gcsl",
        "#[def_type]\n\
         def info(name: String, f: fn(): Int META: {about: {owner: String}, ns: List<Int>, \
         peios: String}): Int {\n\
         \x20   f() + len(f.ns)\n\
         }",
    );

    #[test]
    fn a_reference_reads_its_definition_s_meta() {
        // In parentheses and by a path past the definition, deeper reads, a
        // reference stored and passed as a function with less of the meta,
        // in a pure function: the meta is pure.
        let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
        let files = [
            ("workspace.gcsl", def_types.as_str()),
            (
                "app/a.gcsl",
                "defmeta info dev.x { about = { owner = \"jack\" }; ns = [4, 5]; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 defmeta tagged dev.x { tag = \"t\"; n = 7 }\n\
                 def tagged dev.x() { \"x\" }\n\
                 fn owner(): String { info dev.x.about.owner }\n\
                 fn tag(g: fn(): String META: {tag: String}): String { g.tag }\n\
                 r = [\n\
                 \x20   info dev.x.peios,\n\
                 \x20   (info dev.x).peios,\n\
                 \x20   owner(),\n\
                 \x20   \"{info dev.x.ns[1]}\",\n\
                 \x20   \"{(info dev.x).ns[0]}\",\n\
                 ]\n\
                 let stored = tagged dev.x\n\
                 s = [stored.tag, tag(stored), tag(tagged dev.x), \"{stored.n}\"]\n\
                 called = info dev.x()",
            ),
        ];
        assert_eq!(
            value(&files, "app/a.gcsl"),
            "{r = [\"p\", \"p\", \"jack\", \"5\", \"4\"]; s = [\"t\", \"t\", \"t\", \"7\"]; \
             called = 3}"
        );
        // Under `META->`, each field past the definition that may be a future
        // is awaited where the next reads it, a lazy one included.
        let files = [
            (
                "workspace.gcsl",
                "#[def_type]\n\
                 def t(name: String, f: fn(): Int META-> {about: {owner: String}}) -> Int { f() }",
            ),
            (
                "app.gcsl",
                "defmeta t x { about = { owner = \"o\" } }\n\
                 def t x() { 1 }\n\
                 defmeta t y { about = lazy { owner = lazy \"lazy\" } }\n\
                 def t y() { 2 }\n\
                 r = \"{t x.about.owner} {t y.about.owner} {(t y).about.owner}\"",
            ),
        ];
        assert_eq!(value(&files, "app.gcsl"), "{r = \"o lazy lazy\"}");
    }

    #[test]
    fn a_reference_means_its_longest_definition() {
        // `info dev` is a definition, and so is `tagged dev.peios.x`: each
        // path means the definition of its own def-type, `info
        // dev.peios` the field `peios` of `info dev`'s meta.
        let def_types = format!("{}\n{}", INFO.1, TAGGED.1);
        let files = [
            ("workspace.gcsl", def_types.as_str()),
            (
                "app/a.gcsl",
                "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"meta of dev\" }\n\
                 def info dev() { 1 }\n\
                 defmeta tagged dev.peios.x { tag = \"t\"; n = 1 }\n\
                 def tagged dev.peios.x() { \"x\" }\n\
                 r = [info dev.peios, tagged dev.peios.x.tag]",
            ),
        ];
        assert_eq!(
            value(&files, "app/a.gcsl"),
            "{r = [\"meta of dev\", \"t\"]}"
        );
        // A name past the definition that is no field of its meta: the
        // meta's error, noted with how the path was read.
        let files = [
            INFO,
            (
                "app/a.gcsl",
                "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.peios.x",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            ["cannot access field `x` of String at app/a.gcsl; app/a.gcsl: this is String"]
        );
        let files = [
            INFO,
            (
                "app/a.gcsl",
                "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = info dev.pieos.x",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            [
                "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`; note: `info dev.pieos` is read as the field \
                 `pieos` of the meta of `info dev`, its longest prefix that is a definition"
            ]
        );
        // In parentheses the reference is all there is: no second note.
        let files = [
            INFO,
            (
                "app/a.gcsl",
                "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev).pieos",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            [
                "the meta of `info dev` has no field `pieos` at app/a.gcsl; note: the fields \
                 are `about`, `ns` and `peios`"
            ]
        );
        // A path no prefix of which is a definition: as before.
        let files = [
            INFO,
            (
                "app/a.gcsl",
                "defmeta info dev.x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev.x() { 1 }\n\
                 r = info dev.y.peios\n\
                 s = info dev",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            [
                "`info` has no definition `dev.y.peios` at app/a.gcsl; note: the only \
                 definition of `info` here is `dev.x`",
                "`dev` is not a `info` definition at app/a.gcsl"
            ]
        );
        // The result of a call has the fields of the result, not of the meta.
        let files = [
            INFO,
            (
                "app/a.gcsl",
                "defmeta info dev { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info dev() { 1 }\n\
                 r = (info dev()).peios",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            ["cannot access field `peios` of Int at app/a.gcsl; app/a.gcsl: this is Int"]
        );
    }

    #[test]
    fn a_def_type_without_a_meta_reads_no_path_past_a_definition() {
        // `build x.y` names the definition `x.y`, as it always did.
        let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = build x.y")];
        assert_eq!(
            check_errors(&files),
            [
                "`build` has no definition `x.y` at app.gcsl; note: the only definition of \
                 `build` here is `x`"
            ]
        );
        // Nor does its reference have a meta, nor any field.
        let files = [BUILD, ("app.gcsl", "def build x(t) { t }\nr = (build x).y")];
        assert_eq!(
            check_errors(&files),
            [
                "cannot access field `y` of `fn(mode: String): String` at app.gcsl; app.gcsl: \
                 this is fn(mode: String): String"
            ]
        );
    }

    #[test]
    fn a_meta_read_through_a_reference_needs_the_meta() {
        // The meta in one file, the definition in another, the read in a
        // directory below: each runs before what reads it.
        let files = [
            INFO,
            (
                "pkg/m.gcsl",
                "defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = \"m\" }",
            ),
            ("pkg/d.gcsl", "def info x() { 1 }"),
            ("pkg/sub/app.gcsl", "r = info x.peios"),
        ];
        assert_eq!(value(&files, "pkg/sub/app.gcsl"), "{r = \"m\"}");
        // Read in the definition's file, above its meta, which reads a field
        // of the file.
        let files = [
            INFO,
            (
                "pkg/d.gcsl",
                "r = info x.peios\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = owner }\n\
                 owner = \"later\"",
            ),
        ];
        assert_eq!(
            value(&files, "pkg/d.gcsl"),
            "{r = \"later\"; owner = \"later\"}"
        );
        // A meta that reads a variable bound below the read cannot be read
        // there: the chain names the definition and its meta.
        let files = [
            INFO,
            (
                "app.gcsl",
                "r = info x.peios\n\
                 let k = \"k\"\n\
                 def info x() { 1 }\n\
                 defmeta info x { about = { owner = \"o\" }; ns = [1]; peios = k }",
            ),
        ];
        let found = check_errors(&files).join("\n");
        assert!(
            found.starts_with("`info x` cannot be used here yet at app.gcsl")
                && found.contains("`info x` uses `defmeta info x`, and `defmeta info x` reads `k`"),
            "{found}"
        );
    }

    #[test]
    fn a_meta_read_through_a_reference_is_the_definition_s_meta() {
        let path = std::env::temp_dir().join(format!("gcsl-slice-39-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let count = || std::fs::read_to_string(path).map_or(0, |text| text.lines().count());
        let _ = std::fs::remove_file(path);
        // `META->>`: one run for the def-type's two reads and the
        // references' three, in parentheses, past the definition and
        // stored; at the top level of a file, which is impure.
        let memo = "#[def_type]\n\
             def twice(name: String, f: fn(): Int META->> {n: Int}) -> List<Int> { [f.n, f.n] }";
        let app = format!(
            "defmeta twice x {{ n = exec([\"sh\", \"-c\", \"echo x >> {path}\"]).status }}\n\
             def twice x() {{ 1 }}\n\
             r = twice x()\n\
             let stored = twice x\n\
             s = [(twice x).n, twice x.n, stored.n]"
        );
        let files = [("workspace.gcsl", memo), ("app.gcsl", app.as_str())];
        assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
        assert_eq!(count(), 1);
        // `META->`: one run for each read.
        let impure = memo.replace("META->>", "META->");
        let files = [
            ("workspace.gcsl", impure.as_str()),
            ("app.gcsl", app.as_str()),
        ];
        assert_eq!(value(&files, "app.gcsl"), "{r = [0, 0]; s = [0, 0, 0]}");
        assert_eq!(count(), 6);
        std::fs::remove_file(path).unwrap();
        // In a pure function, the read is the error, however the reference
        // is written; in an impure one, it is allowed.
        let files = [
            ("workspace.gcsl", impure.as_str()),
            (
                "app.gcsl",
                "defmeta twice x { n = 1 }\n\
                 def twice x() { 1 }\n\
                 fn a(): Int {\nlet held = twice x.n\n1\n}\n\
                 fn b(): Int {\nlet held = (twice x).n\n1\n}\n\
                 fn c() -> Int { twice x.n }",
            ),
        ];
        assert_eq!(
            check_errors(&files),
            [
                "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure",
                "cannot read an impure meta from a pure function at app.gcsl; app.gcsl: \
                 reading the meta of `twice x` is impure (`META->`); app.gcsl: this function is \
                 pure, declared with `:`; note: a pure function, declared with `:`, may read \
                 only pure metas, `META:`; declare it with `->` to make it impure"
            ]
        );
    }

    #[test]
    fn dumps_show_a_reference_past_its_definition() {
        let mut workspace = checked(&[
            INFO,
            (
                "app.gcsl",
                "defmeta info x { about = { owner = \"o\" }; ns = []; peios = \"p\" }\n\
                 def info x() { 1 }\n\
                 r = info x.about.owner",
            ),
        ]);
        let index = workspace.file("app.gcsl").unwrap();
        let dump = workspace.dump_types(index).unwrap();
        let meta = "META: {about: {owner: String}, ns: List<Int>, peios: String}";
        assert!(
            dump.contains(&format!(
                "(field r @84..106\n    (. owner : String @88..106\n      (. about : {{owner: \
                 String}} @88..100\n        (def-type-ref info x : fn(): Int {meta} @88..94))))"
            )),
            "{dump}"
        );
        // The syntax tree alone has the reference as it is written.
        let source = SourceFile::new("app.gcsl", "r = info x.about.owner").unwrap();
        let (tokens, names, strings) = crate::lexer::tokenize(&source).unwrap();
        let ast = crate::parser::parse(&tokens, names, strings, &source).unwrap();
        assert!(
            ast.pretty()
                .to_string()
                .contains("(def-type-ref info x.about.owner @4..22)"),
            "{}",
            ast.pretty()
        );
        // The reads of the meta read it through the definition's cell.
        let ir = workspace.dump_ir(index).unwrap();
        assert!(
            ir.contains(
                "(field owner @93..106\n      (field about @93..100\n        (call @93..100\n"
            ),
            "{ir}"
        );
    }
}
