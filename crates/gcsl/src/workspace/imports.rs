//! Resolving imports: the modules a file imports, the members of each, and
//! value files.
//!
//! # Imports
//!
//! An `import` gives a file the rest of the workspace, by name. Its path is
//! absolute from the root: each segment but the last a directory, the last a
//! directory or a file's stem (`import lib/x` is `lib/x.gcsl`); the root
//! directory itself has no path. The scan resolves every import of every file,
//! mentioned or not ([`scan_with`](super::scan_with)): a path that names
//! nothing is "no module `a/b`", with a note listing what the last directory
//! that exists has (its subdirectories, with `/`, then its file stems), or
//! saying that the directory is a workspace of its own. The library cannot see
//! that from the files alone, so [`scan_with`](super::scan_with) and
//! [`check_with`](super::check_with) take the root-relative directories the
//! caller left out as nested workspaces; [`scan`](super::scan) and
//! [`check`](super::check) pass none. A directory and a file of one name are an
//! error once per pair, at the first import that names them.
//!
//! A segment is a directory's or a file's name as it is, dots and all, so
//! `pkgs/net.busybox.busybox` names the directory `net.busybox.busybox`
//! (see the parser's [Imports](crate::parser::imports#imports)). A path whose
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
//! view of a file](crate::workspace::views#the-view-of-a-file)), private to the file too, and
//! clash only with a whole import or a top-level `fn`, `let`, `let mut` or
//! `:=` of their name. `import * from` a value file is "a value file has
//! no members to import". The aliases are a lookup of the [`FileScope`](super::FileScope)
//! apart from the merged trees ([`FileScope::alias`](super::FileScope::alias)), so a reference
//! through one means its longest declared prefix in the target's tree, as
//! every reference does: the numbers past the merged nodes are targets
//! ([`FileScope::target`](super::FileScope::target)).
//!
//! The needs through an import are per leaf ([`Scan::target_files`]): a
//! leaf, a `def` or an interior node needs the files that own it, a file
//! read whole needs that file, a directory read whole needs every file of
//! it, and a view node read whole every owner of every leaf under it. A
//! reach through an import into a node the file itself owns or contributes
//! to, its own file or directory read whole through a whole import, and a
//! whole read of a directory one of whose files needs this one, are the
//! one-file form of the cycle error, "`app.gcsl` needs itself"; files
//! importing each other are the cycle error of [Order](crate::workspace::needs#order).
//!
//! # Imports of definitions
//!
//! `import package * from pkgs/*` (see the parser's
//! [Imports](crate::parser::imports#imports)) binds the definitions of the
//! def-type its member names, `package`, in each directory its path and
//! glob name (`def_type_import`): the one the path names, or every
//! directory directly in it, or at any depth below it
//! ([`Scan::dirs_below`]), a hidden one, its name starting with `.`, and
//! what is below it left out, as discovery leaves them out of a workspace
//! read from a disk; a nested workspace is no directory of this one. The
//! def-type is found as a definition's is, in the importing file's view,
//! its key named with the definitions' (`name_namespaces`), so the import
//! binds once the trees are made again: an alias of the namespace's key
//! for each directory whose tree has the namespace, which the file's view
//! merges, so that `package dev.peios.kernel` reads the definition of
//! whichever directory has it. Two of them that define one path are the
//! conflict of the view, "field `package x` is defined in two files", at
//! the import, labelled with each definition. A name that is no def-type
//! is "`x` is no def-type", a path that names nothing the usual error, a
//! glob on a file's path "a glob names directories, and `a/b` is a file";
//! a glob that finds no definition is no error, as an `import *` of an
//! empty module is not. A failed import leaves the file blind, as a failed
//! `*` does, so that no reference through it is reported again. The
//! import's `DT` is a mention of the def-type, so the file needs its
//! declaration however many definitions it finds; through the aliases, it
//! needs the files of the definitions it reads.
//!
//! A def-type whose definitions are made with a variant keeps each
//! variant's in a namespace of its own (see [`crate::names`], "Namespace
//! keys"). `import Toolchain::C * from PATH` binds the namespace of `C`'s,
//! the key the use's variant gives it, and `import Toolchain::* from PATH`
//! every variant's namespace each directory has, each a root of its tree
//! whose key is a variant's of the def-type's ([`names::is_variant_key_of`]),
//! so that a variant declared in any library is found. As any import's
//! module's, the variants each module it reads declares are seen
//! ([`File::method_modules`]), so a variant that a library declares beside
//! its definitions names them. The checker reports
//! `import Toolchain * from PATH`, which would find none, and a variant
//! written for a def-type that takes none.
//!
//! `import build hello from app` and `import Toolchain::C gcc from lib`
//! import one definition: the namespace of each directory that defines it,
//! seen through it alone ([`Target::Through`]), so that the file sees the
//! definition and none of the namespace's others (`one_definition`). Each
//! such node, and each on the way down to the definition, is numbered among
//! the targets once the files' imports are resolved
//! ([`Scan::register_through`]). One that no directory defines is
//! "`Toolchain::C` has no definition `gcc` in \"lib\"", but for one
//! imported without the variant its def-type's definitions are made with,
//! which the checker reports.
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

use std::collections::{HashMap, HashSet};

use crate::ast::{
    Ast, DefTypeId, ImportDecl, ImportForm, ImportGlob, ImportMember, ItemId, ItemKind, PathSegment,
};
use crate::check::Fileroot;
use crate::diagnostic::Diagnostic;
use crate::lexer;
use crate::names::{self, Name};
use crate::source::Span;

use super::parse::{is_foreign_path, is_library_path};
use super::{
    Alias, BUILTINS, File, LIBRARIES, PRELUDE, STANDARD_DIR, Scan, Target, TypeImport, clashes,
};

/// `error` with the label `text` at `span` of file `input`: a label of the
/// primary span's own file when it is that file, else an excerpt of its
/// own.
pub(super) fn context_in(error: Diagnostic, input: u32, span: Span, text: &str) -> Diagnostic {
    if error.input == Some(input) {
        error.with_context(span, text)
    } else {
        error.with_context_in(input, span, text)
    }
}

/// What [`imports`] looks modules up in.
pub(crate) struct Modules<'a> {
    /// The directories by path.
    pub(super) dirs: &'a HashMap<String, u32>,
    /// The files by path.
    pub(super) files: HashMap<&'a str, u32>,
    /// The directories that are workspaces of their own.
    pub(super) nested: &'a [String],
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
pub(super) fn imports(
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
        // errors](crate::workspace::scanning#scan-errors)).
        let mut failed_imports: Vec<(ItemId, Option<Target>)> = Vec::new();
        let mut poisoned: Vec<Name> = Vec::new();
        let mut blind = false;
        let mut modules_named: Vec<Target> = Vec::new();
        let mut import =
            |item: ItemId, (aliases, types): (&mut Vec<Alias>, &mut Vec<TypeImport>)| {
                let ItemKind::Import(id) = file.ast.item(item).kind else {
                    return;
                };
                let decl = file.ast.import_decl(id);
                let path = file.ast.path(decl.path);
                let before = own_errors.len();
                if let Some(member) = def_type_member(&file.ast, decl) {
                    let at = (file, item, decl);
                    let bound = (&mut *aliases, &mut modules_named);
                    def_type_import(scan, &modules, at, member, bound, &mut own_errors);
                    if own_errors.len() > before {
                        failed_imports.push((item, None));
                        blind = true;
                    }
                    return;
                }
                let resolved = module(scan, &modules, path, &mut both);
                let resolved = library_sees(scan, (file, decl.path_at), resolved);
                let module_of = resolved.as_ref().ok().copied();
                if let Some(target @ (Target::Dir(_) | Target::File(_))) = module_of {
                    modules_named.push(target);
                }
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
        found.push((
            aliases,
            types,
            (poisoned, blind, failed_imports),
            modules_named,
        ));
    }
    register_throughs(scan, found.iter().flat_map(|(aliases, ..)| aliases));
    for (file, (aliases, types, (poisoned, blind, failed), modules_named)) in
        scan.files.iter_mut().zip(found)
    {
        file.aliases = aliases;
        file.type_imports = types;
        file.failed_imports = failed;
        file.method_modules = modules_named;
        for name in poisoned {
            file.poison(name);
        }
        file.blind |= blind;
    }
}

/// Registers each node seen through a leaf that one of `aliases` binds,
/// what an import of one definition binds, so that it is numbered among the
/// targets ([`Scan::register_through`]).
fn register_throughs<'a>(scan: &mut Scan, aliases: impl Iterator<Item = &'a Alias>) {
    for alias in aliases {
        if let Target::Through { node, leaf } = alias.target {
            scan.register_through(node, leaf);
        }
    }
}

/// The names the import `decl` of the tree `ast` binds, each member's (or
/// its alias), or a whole import's, and whether it has a `*`, whose names
/// are not known from it alone.
pub(super) fn import_binds(ast: &Ast, decl: &ImportDecl) -> (Vec<Name>, bool) {
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
                    // Names under a namespace's key, unknown from the import.
                    ImportMember::All(_) | ImportMember::DefType { .. } => star = true,
                }
            }
            (names, star)
        }
    }
}

/// Adds to `aliases`, the imports of `file`, a session's input, each of
/// `inherited`, the imports of the session's earlier inputs, that the input
/// neither imports again nor declares in the whole of its top level (by a `fn`,
/// field or `def` whose path starts with its name, or a `:=` of it, which means
/// the variable of its name): a name of an earlier input is shadowed by a later
/// input's, whether it was imported or declared (see [`crate::session`]). A
/// `let` shadows one only from its point on, as it does an earlier input's
/// variable: the checker reads an inherited import through the view until a
/// binding of its name is found
/// ([`FileScope::inherited`](super::FileScope::inherited)). They follow the
/// input's own imports, before the implicit prelude's aliases but last in the
/// static order (see the module documentation, "Shadowable declarations"), so
/// they shadow the prelude's names, and they clash with nothing.
pub(super) fn inherit_imports(file: &File, inherited: &[Alias], aliases: &mut Vec<Alias>) {
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

/// Where a library is looked for, as GCSL's hosts look for it.
const LIBRARY_ROOTS_NOTE: &str = "a library is looked for in each directory of \
                                  `$GCSL_LIBRARY_PATH`, then in `.gcsl/libraries/` at the \
                                  workspace's root";

/// The library whose module path is `shown`, `<lib>` and its segments,
/// written at `written`: its directory, whole, no directory above it being
/// a module (see [Libraries](crate::workspace#libraries)), or that it is not
/// found.
fn library_module(
    modules: &Modules,
    shown: &[&str],
    written: Span,
) -> Result<Target, Option<Diagnostic>> {
    match modules.dirs.get(&shown.join("/")) {
        Some(&dir) => Ok(Target::Dir(dir)),
        None => Err(Some(
            Diagnostic::error(
                format!(
                    "library `{}` is not found",
                    shown.get(1..).unwrap_or_default().join("/")
                ),
                written,
            )
            .with_note(LIBRARY_ROOTS_NOTE),
        )),
    }
}

/// `resolved`, what an import of `file` written at `at` names, unless the
/// file is a library's and it names a workspace's module, which is an error:
/// a library sees libraries and the standard library alone, so that it means
/// the same in every workspace (see [Libraries](crate::workspace#libraries)).
fn library_sees(
    scan: &Scan,
    (file, at): (&File, Span),
    resolved: Result<Target, Option<Diagnostic>>,
) -> Result<Target, Option<Diagnostic>> {
    match resolved {
        Ok(target) if is_library_path(&file.path) && of_workspace(scan, target) => {
            Err(Some(Diagnostic::error(
                "a library imports only libraries and the standard library, never a workspace's \
                 modules",
                at,
            )))
        }
        other => other,
    }
}

/// Whether `target` is a module of the workspace's own: none of a library's,
/// of the standard library's or a built-in one.
fn of_workspace(scan: &Scan, target: Target) -> bool {
    let path = match target {
        Target::Dir(dir) => scan.dirs.get(dir as usize).map(|dir| dir.path.as_str()),
        Target::File(file) => scan.files.get(file as usize).map(|file| file.path.as_str()),
        _ => None,
    };
    path.is_some_and(|path| !is_foreign_path(path))
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
    if shown.first() == Some(&LIBRARIES) {
        return library_module(modules, &shown, written);
    }
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
/// [Imports](crate::parser::imports#imports)).
pub(super) fn slashed_module(modules: &Modules<'_>, shown: &[&str]) -> Option<String> {
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
/// [Suggestions](crate::check::names#suggestions)): the nearest of the directories
/// in `at` (none a workspace of its own), the stems of its files if `last`
/// (a file has no modules in it), and, for a path of one segment, the
/// standard modules, `builtins` and `fileroot`.
pub(super) fn module_suggestion(
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
pub(super) const WORKSPACE_FILE: &str = "workspace.gcsl";

/// Gives a cell to the value of each value file (one whose outermost block
/// ends in a result) that some file imports whole ([`Scan::value_slot`]).
pub(super) fn value_cells(scan: &mut Scan) {
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
pub(super) fn is_value_file(file: &File) -> bool {
    file.ast
        .items(file.ast.root())
        .last()
        .is_some_and(|&item| matches!(file.ast.item(item).kind, ItemKind::Result(_)))
}

/// What directory `dir` holds, as a "no module" note lists it: its
/// directories with `/`, then its files' stems, each in path order.
pub(super) fn contents(scan: &Scan, dir: Option<u32>) -> String {
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
/// `*`, into `types` (see [Types](crate::workspace::declared#types)): a member that is both a root
/// and a type binds both, and one that is neither is "`lib` has no `x`".
/// An import of a def-type's definitions is resolved apart, by its key
/// (`def_type_import`).
pub(super) fn members(
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
                                shadowable: file.ast.is_shadowable(item),
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
                    ImportMember::DefType { .. } => {}
                    ImportMember::All(star) => {
                        let shadowable = file.ast.is_shadowable(item);
                        let at = (item, star, implicit, shadowable);
                        star_aliases(scan, target, at, (&mut *aliases, &mut *types), errors);
                    }
                }
            }
        }
    }
}

/// Binds every root of `target` into `aliases`, and every type of it into
/// `types`, for the `*`, written at `star`, of the import `item`, implicit
/// if it is the prelude's, shadowable if it is `#[shadowable]`: a value file
/// has none, "a value file has no members to import".
fn star_aliases(
    scan: &Scan,
    target: Target,
    (item, star, implicit, shadowable): (ItemId, Span, bool, bool),
    (aliases, types): (&mut Vec<Alias>, &mut Vec<TypeImport>),
    errors: &mut Vec<Diagnostic>,
) {
    if let Target::File(of) = target
        && scan.files.get(of as usize).is_some_and(is_value_file)
    {
        errors.push(Diagnostic::error(
            "a value file has no members to import",
            star,
        ));
        return;
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
                shadowable,
            });
        }
    }
}

/// The def-type member of the import `decl` of `ast`, `DT *`, with where it
/// is written and whether it is of every variant, `DT::*`, if it has one,
/// its only member.
fn def_type_member(ast: &Ast, decl: &ImportDecl) -> Option<(DefTypeId, Span, bool)> {
    let ImportForm::Members(range) = decl.form else {
        return None;
    };
    ast.import_members(range)
        .iter()
        .find_map(|member| match *member {
            ImportMember::DefType { def, span, every } => Some((def, span, every)),
            _ => None,
        })
}

/// Binds the definitions of the def-type that `used`, the one member of
/// the import `item` of `file`, `decl`, written at `written`, names, in
/// each directory its path and glob name (see [Imports of
/// definitions](#imports-of-definitions)): for each, the root of the
/// def-type's namespace in it, or of its variant's, as an alias of its key,
/// which the file's view merges; for `DT::*`, `every`, the root of each of
/// its variants' namespaces; and adds each module it reads to `read`, whose
/// variants the file sees, so that a variant a library declares names its
/// definitions. A def-type the file does not see is "`x` is no def-type"; a
/// path that names nothing, the usual error; a glob on a file's path "a
/// glob names directories, and `a/b` is a file".
fn def_type_import(
    scan: &Scan,
    modules: &Modules,
    (file, item, decl): (&File, ItemId, &ImportDecl),
    (used, written, every): (DefTypeId, Span, bool),
    (aliases, read): (&mut Vec<Alias>, &mut Vec<Target>),
    errors: &mut Vec<Diagnostic>,
) {
    let ast = &file.ast;
    let Some(key) = ast.def_type(used).key else {
        let shown: Vec<&str> = ast
            .def_type_names(used)
            .into_iter()
            .map(|name| scan.names.get(name))
            .collect();
        let message = format!("`{}` is no def-type", shown.join("."));
        errors.push(Diagnostic::error(message, written));
        return;
    };
    let path = ast.path(decl.path);
    let base = if path.is_empty() {
        modules.dirs.get("").map(|&dir| Target::Dir(dir))
    } else {
        match module(scan, modules, path, &mut Vec::new()) {
            Ok(target) => Some(target),
            Err(error) => {
                errors.extend(error);
                return;
            }
        }
    };
    let targets = match (base, decl.glob) {
        (Some(target @ (Target::Dir(_) | Target::File(_))), ImportGlob::Exact) => vec![target],
        (Some(Target::Dir(dir)), glob) => scan
            .dirs_below(dir, glob == ImportGlob::Descendants)
            .into_iter()
            .map(Target::Dir)
            .collect(),
        (Some(Target::File(_)), _) => {
            let shown: Vec<&str> = path
                .iter()
                .filter_map(PathSegment::name)
                .map(|name| scan.names.get(name))
                .collect();
            let shown = shown.join("/");
            errors.push(Diagnostic::error(
                format!("a glob names directories, and `{shown}` is a file"),
                written,
            ));
            return;
        }
        _ => {
            errors.push(Diagnostic::error(
                "what this path names holds no definitions to import",
                written,
            ));
            return;
        }
    };
    // Its modules' variants are seen, as any import's are: a variant a
    // library declares names its definitions.
    read.extend(targets.iter().copied());
    let definition = ast.path(ast.def_type(used).path);
    if !definition.is_empty() {
        let found = one_definition(scan, &targets, key, definition);
        // A def-type whose definitions have variants, imported without one,
        // is the checker's to report.
        let unvaried = ast.def_type(used).variant.is_none() && has_variants(scan, &targets, key);
        if found.is_empty() && !unvaried {
            errors.push(no_definition(scan, (ast, used), path, written));
        }
        aliases.extend(found.into_iter().map(|target| Alias {
            name: key,
            span: written,
            item: Some(item),
            target,
            whole: false,
            implicit: false,
            inherited: false,
        }));
        return;
    }
    let of = scan.names.get(key);
    for target in targets {
        let keys = if every {
            target_roots(scan, target)
                .into_iter()
                .filter(|&root| names::is_variant_key_of(scan.names.get(root), of))
                .collect()
        } else {
            vec![key]
        };
        for name in keys {
            if let Some(child) = scan.target_child(target, name) {
                aliases.push(Alias {
                    name,
                    span: written,
                    item: Some(item),
                    target: child,
                    whole: false,
                    implicit: false,
                    inherited: false,
                });
            }
        }
    }
}

/// Whether any of `targets` has definitions made with a variant of the
/// def-type whose namespace's key is `key`.
fn has_variants(scan: &Scan, targets: &[Target], key: Name) -> bool {
    let of = scan.names.get(key);
    targets.iter().any(|&target| {
        target_roots(scan, target)
            .into_iter()
            .any(|root| names::is_variant_key_of(scan.names.get(root), of))
    })
}

/// "`Toolchain::C` has no definition `gcc` in \"lib\"", of `used`, an
/// import of `ast` of one definition from `path`, written at `written`: a
/// library's path as it is written, in quotes, any other in backticks. The
/// scan has the names, not the tree.
fn no_definition(
    scan: &Scan,
    (ast, used): (&Ast, DefTypeId),
    path: &[PathSegment],
    written: Span,
) -> Diagnostic {
    let shown = |segments: &[PathSegment], by: &str| {
        let names: Vec<&str> = segments
            .iter()
            .filter_map(PathSegment::name)
            .map(|name| scan.names.get(name))
            .collect();
        names.join(by)
    };
    let dt: Vec<&str> = ast
        .def_type_names(used)
        .into_iter()
        .map(|name| scan.names.get(name))
        .collect();
    let variant = ast
        .def_type(used)
        .variant
        .map_or_else(String::new, |(name, _)| {
            format!("::{}", scan.names.get(name))
        });
    let from = shown(path, "/");
    let from = from
        .strip_prefix(&format!("{LIBRARIES}/"))
        .map_or_else(|| format!("`{from}`"), |library| format!("\"{library}\""));
    let definition = shown(ast.path(ast.def_type(used).path), ".");
    let message = format!(
        "`{}{variant}` has no definition `{definition}` in {from}",
        dt.join(".")
    );
    Diagnostic::error(message, written)
}

/// The namespace of key `key` in each of `targets`, directories or a file,
/// seen through its definition of path `definition` alone, of each that has
/// one: its merged node seen through the definition's leaf, a definition a
/// file's own if the target is that file.
fn one_definition(
    scan: &Scan,
    targets: &[Target],
    key: Name,
    definition: &[PathSegment],
) -> Vec<Target> {
    targets
        .iter()
        .filter_map(|&target| {
            let node = match scan.target_child(target, key)? {
                Target::Node(node) => node,
                Target::Part { file, node } => {
                    *scan.files.get(file as usize)?.global.get(node as usize)?
                }
                _ => return None,
            };
            let mut at = Target::Node(node);
            for segment in definition {
                at = scan.target_child(at, segment.name()?)?;
            }
            let Target::Node(leaf) = at else {
                return None;
            };
            let owner = scan.nodes.get(leaf as usize)?.leaf?;
            match target {
                Target::File(file) if file != owner => None,
                _ => Some(Target::Through { node, leaf }),
            }
        })
        .collect()
}

/// The suggestion for `name`, imported by member from `target`, which has
/// no root or type of that name (see
/// [Suggestions](crate::check::names#suggestions)): the nearest of its roots and
/// its types.
pub(super) fn member_suggestion<'s>(scan: &'s Scan, target: Target, name: &str) -> Option<&'s str> {
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
pub(super) fn fileroot_member_import(scan: &Scan, member: ImportMember) -> Diagnostic {
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
        ImportMember::All(star) | ImportMember::DefType { span: star, .. } => {
            (Fileroot::Caller, star)
        }
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
        Target::Part { .. }
        | Target::Node(_)
        | Target::Through { .. }
        | Target::View(_)
        | Target::Fileroot(_) => Vec::new(),
    }
}
