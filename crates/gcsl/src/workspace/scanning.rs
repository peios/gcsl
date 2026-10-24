//! Scanning files into a workspace: placing them in directories, merging their
//! trees, and the namespaces and metas of their names.
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
//!   is (see [Shadowable declarations](crate::workspace::parse#shadowable-declarations)): in no
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

use std::mem;
use std::collections::{HashMap, HashSet};

use crate::ast::{DefTypeId, DefTypeRole, ItemKind, PathSegment, TypeStore, TypeStoreMark};
use crate::check::Fileroot;
use crate::diagnostic::Diagnostic;
use crate::mentions::{self, Resolve};
use crate::names::{self, Name, Names};
use crate::source::{SourceFile, Span};

use super::{
    Alias, BUILTINS, Dir, File, FileScope, Inheriting, Library, ROOT, STANDARD_DIR, Scan,
    builtin_nodes, declare_types, define_types, find_needs, imports, merge, merge_files,
    method_views, no_prelude_in_input, order, parse_files, place_dirs, report_cycles, reserved,
    resolve, schedule, shadow, shadowable_nodes, type_views, value_cells, views,
};

/// A count as a `u32`, saturating; a workspace's counts are bounded by its
/// sources' lengths.
pub(super) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The directory part of `path`, empty for a file of the root.
pub(super) fn parent_path(path: &str) -> &str {
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
/// parser recovered them (see [Syntax errors](crate::workspace::parse#syntax-errors)), and every
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
pub(super) enum Placing<'a> {
    /// The files of a workspace, by their paths relative to its root, with
    /// the directories left out as workspaces of their own.
    Workspace(&'a [String]),
    /// One file outside any workspace, a lone file or a session's input, in the
    /// root directory by the last part of its name, with the imports of the
    /// session's earlier inputs it inherits (see
    /// [`inherit_imports`](super::imports::inherit_imports)), none for a lone
    /// file, and whether it is a session's input, which cannot be marked
    /// `#![no_prelude]` ([`no_prelude_in_input`]).
    Lone(&'a [Alias], bool),
}

/// [`scan_with`] of `sources` placed as `placing` says, beside `library`,
/// interning into `names`, which the scan holds once it succeeds
/// ([`Scan::names`]) and which are given back to `names` if it fails. It fails
/// only for a session's input with a syntax error or an error of the scan,
/// which is not checked; any other scan recovers from its errors (see [Scan
/// errors](#scan-errors)). The indices in the library's table of the standard
/// files it loads, which are files `0..` [`Scan::standard`], are left in
/// `loaded` whether it succeeds or fails, so that a caller can render a
/// diagnostic of any file against its source (see
/// [`check_with_standard`](super::check_with_standard)). Its types are its own
/// ([`scan_into_with`]).
pub(super) fn scan_into(
    placed: (&[SourceFile], Placing),
    library: Library,
    names: &mut Names,
    loaded: &mut Vec<usize>,
) -> Result<Scan, Vec<Diagnostic>> {
    scan_into_with(
        placed,
        library,
        (names, loaded),
        (&mut TypeStore::new(), (&[], &[], &[])),
        (&[], None),
    )
}

/// [`scan_into`], the files' types defined after those of `store`, which the
/// scan holds once it succeeds ([`Scan::type_store`]) and which is given back
/// to `store` if it fails, and with `inherited`, the types a session's input
/// inherits from the earlier inputs, each with its declaration's number in
/// `store` (see [Types](crate::workspace::declared#types)), and the variants,
/// each by its enum's key and its name with its own key (see
/// [Enums](crate::workspace::declared#enums)); and for a session's input, with
/// `def_types`, the def-types it inherits from the earlier inputs, by path with
/// their namespaces' keys, and its number among the session's inputs (see
/// [Def-types](crate::workspace::declared#def-types)).
pub(super) fn scan_into_with(
    (sources, placing): (&[SourceFile], Placing),
    library: Library,
    (names, loaded): (&mut Names, &mut Vec<usize>),
    (store, (inherited, variants, contexts)): (&mut TypeStore, Inheriting<'_>),
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
        variants: Vec::new(),
        inherited_variants: variants.to_vec(),
        contexts: Vec::new(),
        inherited_contexts: contexts.to_vec(),
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
pub(super) fn scan_files(
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
    // any definition has one (see [Def-types](crate::workspace::declared#def-types)).
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
        // report it as a conflict (see [Def-types](crate::workspace::declared#def-types)); it is
        // dropped. The trees made again find again what these found.
        let (unpaired, dropped) = unpaired_metas(scan);
        errors.extend(unpaired);
        unscan(scan, mark, &dropped);
    }
    mark_unpaired(scan);
    type_views(scan, &mut errors);
    method_views(scan);
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
/// [Def-types](crate::workspace::declared#def-types)). Adds to `errors` the conflicts of the
/// directories' trees, the types declared twice, the imports' errors and
/// the conflicts of the views, recovering from each (see [Scan
/// errors](#scan-errors)).
pub(super) fn scan_trees(
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
pub(super) fn merge_trees(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
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
pub(super) fn conflicted_nodes(scan: &mut Scan) {
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
pub(super) fn unscan(scan: &mut Scan, mark: TypeStoreMark, dropped: &[(u32, usize)]) {
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
    scan.variants.clear();
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
pub(super) type NamedUse = (u32, DefTypeId, String, Option<(String, String)>);

/// Names the namespace of each def-type definition and reference of every file
/// of `scan` whose namespace is not known yet: what its `DT` names in the
/// file's view, if that is a `def` marked `#[def_type]`, or, for a session's
/// input, a def-type of the earlier inputs by its path, gives it the def-type's
/// key ([`Ast::set_def_type_key`](crate::ast::Ast::set_def_type_key)), which
/// the file is then taken to spell (see
/// [Def-types](crate::workspace::declared#def-types)); and a definition or a
/// meta its leaf in the namespace of the metas of the def-type's definitions in
/// its file's directory, the meta key and its path as one name (see
/// [`crate::names`], "Namespace keys"), which the file is taken to spell too.
/// Returns whether a definition or a meta was given one, which changes the
/// trees.
pub(super) fn name_namespaces(scan: &mut Scan) -> bool {
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
            let Some(mut key) = def_type_key(scan, index, &scope, &names) else {
                continue;
            };
            // A use with a variant, `Toolchain::C gcc`, is of the variant's
            // own namespace.
            if let Some((variant, _)) = def.variant {
                let Some(variant_key) = names::variant_namespace_key(&key, scan.names.get(variant))
                else {
                    continue;
                };
                key = variant_key;
            }
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
/// [Def-types](crate::workspace::declared#def-types)): that leaf, its file, its use of its def-type
/// and its item's position among the file's items.
pub(super) type MetaLeaf = ((Name, Name), u32, DefTypeId, usize);

/// The definitions and the metas of `scan`'s files whose def-types are
/// known, each by its leaf in the namespace of the metas of its def-type's
/// definitions in its directory, in path order of their files and then in
/// source order: so a definition and a meta of one path, def-type and
/// directory have one leaf.
pub(super) fn meta_leaves(scan: &Scan) -> (Vec<MetaLeaf>, Vec<MetaLeaf>) {
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
/// directory's one meta of a definition of it (see
/// [Def-types](crate::workspace::declared#def-types)): a meta whose directory
/// declares another of its path and def-type before it, "`defmeta package x` is
/// declared twice in this directory", at it, labelled "first declared here" at
/// the first; and one whose directory declares no definition of its path and
/// def-type, "`defmeta package x` has no definition `x` in its directory", at
/// it. Each at the meta's `defmeta`, def-type and path, in path order of their
/// files, then in source order. With them, each such meta, by its file and its
/// position among the file's items, which the scan drops (see [Scan
/// errors](#scan-errors)).
pub(super) fn unpaired_metas(scan: &Scan) -> (Vec<Diagnostic>, Vec<(u32, usize)>) {
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
pub(super) fn mark_unpaired(scan: &mut Scan) {
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
pub(super) fn def_type_key(
    scan: &Scan,
    index: u32,
    scope: &FileScope,
    names: &[Name],
) -> Option<String> {
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
