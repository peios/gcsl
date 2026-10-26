//! Parsing a workspace's files and the standard library, and the shadowable
//! declarations and leaves of their trees.
//!
//! # Shadowable declarations
//!
//! A top-level field (a dotted one or a `def` included) or an `import` of
//! any form may be marked `#[shadowable]` (see the parser's
//! [Attributes](crate::parser::attributes#attributes)): it may then be shadowed by a
//! later declaration of its name. A LEAF is *shadowable* if its declaration
//! is marked ([`crate::mentions::FieldLeaf::shadowable`],
//! [`Node::shadowable`]), if it reaches a view through a marked import
//! ([`File::marked`], [`ViewPart::marked`](super::ViewPart::marked)), or if it was shadowable where
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
//! view's node of its name being the winner's alone ([`Merged::One`](super::Merged::One),
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
//! # Syntax errors
//!
//! A file with lexical or syntax errors is scanned and checked as its
//! parser recovered it (see the parser's [Errors and
//! recovery](crate::parser::recovery#errors-and-recovery)), so that one file's typo
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
//! - The errors of the scan itself (see [Scan errors](crate::workspace::scanning#scan-errors)) are
//!   reported with the syntax errors, but those of a file that sees a file
//!   with syntax errors, as above, or one of whose imports that failed
//!   names a module with one, whose declarations may be missing or
//!   misplaced: the scan recovers from those all the same.
//!
//! Files that parse cleanly and see no file with errors are checked as ever. A
//! session's input with an error is not checked: nothing of it is run or kept.
//! [`check_partial`](super::check_partial) leaves what a failed check found
//! ([`Partial`](super::Partial)), and
//! [`index_with_standard`](super::index_with_standard) (and, for a lone file,
//! [`index_lone_with_standard`](super::index_lone_with_standard)) checks as
//! [`check_with_standard`](super::check_with_standard) does, each file's check
//! recording what its names refer to (see [`crate::check`], "Recording"), and
//! indexes what every checked file's check found, failed or not, for a language
//! server ([`crate::index`]); a file that is not checked, unbalanced or needing
//! a file whose check failed, has nothing indexed.

use std::mem;
use std::collections::{HashMap, HashSet};

use crate::parser;
use crate::ast::{self, Ast, ItemKind, PathSegment};
use crate::check::{Builtin, Fileroot};
use crate::diagnostic::Diagnostic;
use crate::lexer::{self, TokenKind};
use crate::mentions::{self, Sweep};
use crate::names::{Name, Names};
use crate::source::{SourceFile, Span};

use super::{
    BUILTINS, Dir, File, FileScope, LIBRARIES, Library, Node, PRELUDE, Placing, ROOT, STANDARD_DIR,
    Scan, count, parent_path, standard_source,
};

/// A file parsed: its tree and the names it spells, with its lexical and
/// syntax errors, in source order (see [`parse`]).
pub(super) type Parsed = (Ast, Vec<Name>, Vec<Diagnostic>);

/// Files parsed, each its tree and the names it spells.
pub(super) type Trees = Vec<(Ast, Vec<Name>)>;

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
pub(super) fn parse_files(
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
pub(super) fn load_standard(
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
pub(super) fn standard_imports<'a>(
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
pub(super) fn place_dirs(
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
pub(super) fn builtin_nodes(scan: &mut Scan) {
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
pub(super) fn reserved(scan: &Scan, library: Library, errors: &mut Vec<Diagnostic>) {
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
pub(super) fn no_prelude_in_input(scan: &Scan, errors: &mut Vec<Diagnostic>) {
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
pub(super) fn shadow(scan: &mut Scan) {
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
pub(super) fn shadowable_nodes(scan: &mut Scan) {
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
pub(super) fn resolve(scan: &mut Scan) {
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
pub(super) fn parse(source: &SourceFile, names: &mut Names) -> Parsed {
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

/// Whether `path`, a directory's or a file's, is a library's, under
/// [`LIBRARIES`] (see [Libraries](crate::workspace#libraries)).
#[must_use]
pub(crate) fn is_library_path(path: &str) -> bool {
    path == LIBRARIES || path.starts_with(&format!("{LIBRARIES}/"))
}

/// Whether `path`, a directory's or a file's, is foreign to the workspace:
/// a library's, the standard library's or the built-in module's, none of
/// the workspace's own code (see [Errors in a
/// library](crate::eval::calls#errors-in-a-library)).
#[must_use]
pub(crate) fn is_foreign_path(path: &str) -> bool {
    is_library_path(path)
        || path == STANDARD_DIR
        || path.starts_with(&format!("{STANDARD_DIR}/"))
        || path == BUILTINS
}

/// The directory `path`, added with the directories above it if new; a
/// library's directory with none.
pub(super) fn intern_dir(dirs: &mut Vec<Dir>, index: &mut HashMap<String, u32>, path: &str) -> u32 {
    if let Some(&dir) = index.get(path) {
        return dir;
    }
    // A library's directory is a root of its own, as a standard file's is,
    // apart from the workspace's tree and every other library's (see
    // [Libraries](crate::workspace#libraries)).
    let parent = (!path.is_empty() && !is_library_path(path))
        .then(|| intern_dir(dirs, index, parent_path(path)));
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
pub(super) fn leaf_spans(file: &File, own: u32) -> Option<(Span, Span)> {
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
pub(super) fn first_leaf(file: &File, own: u32) -> Option<(Span, Span)> {
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
pub(super) fn leaves_under(file: &File, own: u32) -> Vec<usize> {
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
