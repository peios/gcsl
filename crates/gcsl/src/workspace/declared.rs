//! Declaring the workspace's types, methods, enum variants and contexts, and
//! their conflicts and cycles.
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
//!   [`crate::mentions`]), but one named like a built-in type, and a second of
//!   one name in one file, which the checker reports. A directory's types are
//!   its files' ([`declare_types`]): one name declared in two files of it is
//!   "type `Foo` is declared in two files", labelled "declared here" at both,
//!   as two leaves of one path are.
//! - A member import of a module's type binds it (`import PackageDef from
//!   lib.types`), and of a name that is both a type and a root binds both; `*`
//!   binds every type of the module, besides its roots
//!   ([`members`](super::imports::members), [`File::type_imports`]). A
//!   directory's types are its files', a file's its own. `*` from a value file
//!   is still an error, but its types may be imported by member or read
//!   qualified.
//! - Each file's view of types ([`File::type_view`], [`type_views`]) merges its
//!   own directory's types, each directory's upward and its imported types, in
//!   that order, the order fields' views report their conflicts in; and, for a
//!   session's input, those it inherits from the earlier inputs, last, which
//!   anything shadows. One declaration reached twice is one type. Two of one
//!   name are "type `Foo` is declared in two files", once per pair, at the
//!   later source, labelled "declared here" at each declaration and "imported
//!   here" at each import, with the note "one is in a directory above" when one
//!   is. Types are never shadowable: a `#[shadowable]` import brings them as
//!   plain types, and the prelude declares none. A type imported as a built-in
//!   type's name is "`Int` is a built-in type".
//! - A type marked `#[attribute]` is an attribute's declaration (see the
//!   checker's [Attributes](crate::check::attributes#attributes)): the store
//!   records it by its number, with what a host knows it by, its file's path
//!   without `.gcsl` and its name
//!   ([`TypeStore::mark_attribute`](crate::ast::TypeStore::mark_attribute)), so
//!   that a session's later inputs know it too, but one named like a built-in
//!   attribute, "`retry` is a built-in attribute". A use of an attribute is a
//!   name of a type of the file's view, read as any is, and its file's checker
//!   asks whether that type is one (`Context::attribute`).
//! - A qualified name, `t.PackageDef` ([`Scan::qualified_type`]), names through
//!   a whole import of the file (`import lib/types as t`) a type of the module
//!   it names: "`t` has no type `Foo`", with a note listing its types, if it
//!   has none of that name; "unknown type `a.b`" if `a` is no module the file
//!   imports whole; and "`t.Foo` is a type, not a module" for a path that goes
//!   on after a type. A module's name written as a type, `t`, is "`t` is a
//!   module, not a type" ([`Scan::type_links`]).
//! - A generic type, `type Pair<A, B> = ...`, is used applied to as many type
//!   arguments as it has type parameters, `Pair<Int, String>` or `t.Pair<Int,
//!   String>`, which the store records of each declaration
//!   ([`TypeStore::set_arity`](crate::ast::TypeStore::set_arity)); used bare it
//!   is "type `Pair` needs its type arguments", with too few or too many "type
//!   `Pair` takes 2 type arguments, given 1", and a type that is not generic,
//!   applied, "type `Foo` takes no type argument" ([`Scan::applied_link`]). An
//!   attribute's type is never generic: "attribute `Tag` takes no type
//!   parameters".
//!
//! Then the types are defined and the files' trees expanded ([`define_types`]):
//! each declaration's definition is copied into the scan's
//! [`TypeStore`](crate::ast::TypeStore), each name in it of another declared
//! type a reference to it, as the declaring file's view resolves it, an
//! instance of it if applied to arguments, and each of a generic declaration's
//! own type parameters a parameter; the references are a graph, whose cycles
//! are each "type `A` refers to itself", at the first declaration of the cycle
//! in path then source order, labelled at each step ("`B` uses `A` here", in
//! `B`'s file); every declaration is settled, those it refers to first, a
//! reference made what it refers to, an instance what it refers to with its
//! arguments in place of its parameters, and a spread merged, one on a cycle,
//! with a spread's problem or nested deeper than [`crate::MAX_DEPTH`] once
//! expanded erroneous; and each file's tree is expanded
//! ([`Ast::expand_types`](crate::ast::Ast::expand_types)): each name of a
//! declared type in it made a copy of the definition, as if written there, a
//! generic one's with the use's arguments in place of its parameters, each
//! record type with spreads the record of its merged fields. A session keeps
//! its store, each input's declarations after the earlier inputs'
//! ([`check_input`](super::check_input)). The checker then sees no declared
//! type's name; before any file is checked, the driver checks every file's
//! `type` items ([`check_type_items`](super::checking::check_type_items)),
//! since a file may use a type of a file checked after it, so that what is
//! wrong in a definition is reported once, where it is written, and every file
//! using it fails quietly (see the checker's "Type declarations").
//!
//! # Methods
//!
//! A method (see the checker's [Methods](crate::check::methods#methods)) binds no
//! name, so it is in no tree, no view and no record: a file's methods are
//! its tree's ([`Ast::methods`](crate::ast::Ast::methods)), each a top-level function of its file
//! under a binding no source can write. A file sees the `def` methods of
//! other files as a flat view, apart from its fields, as it sees types
//! ([`File::method_view`], `method_views`): those of the other files of
//! its directory and of every directory above it, and those of every file
//! of each module it imports, in any form, whole, by member or by `*`, a
//! directory's own files or a file ([`File::method_modules`]), the
//! implicit prelude's among them. A `fn` method is seen by its own file
//! only. Methods of one name merge, never conflict, in a view: which one a
//! call means is the checker's to say, by the receiver's type.
//!
//! Which method a call `x.m(...)` means is known only once `x` is typed,
//! after the order is fixed. So a call of a field named `m` makes its file
//! need the file of every method `m` it sees (`find_needs`):
//! conservatively, so that a file calling a record's field `get` needs a
//! file whose `def` method `get` it sees, and two such files that need each
//! other otherwise too are the usual cycle, labelled "may call the method
//! `get` of `lib.gcsl` here". Once a file is checked, each of its methods'
//! binding, by the slot it leaves its closure in, its type and what a call
//! of it needs to know of its parameters, is recorded
//! (`CheckedMethod`, `checked_methods`); a later file's context gives
//! its checker those it sees by name (`Context::methods`), so a call of
//! another file's method reads its closure from its slot, the other file
//! having run first, and is of a known callee, its defaults given. A
//! session keeps the standard files' for its inputs.
//!
//! # Enums
//!
//! An enum is a `type` declaration defined by no type expression (see the
//! checker's [Enums](crate::check::enums#enums)): its declaration in the store
//! is a leaf, [`TypeExprKind::Enum`](crate::ast::TypeExprKind::Enum), of its
//! key, a name of its name and of its file's path, or `input N` for a session's
//! input (`Scan::origin`, [`names::enum_key`]), so it is seen, imported and
//! qualified as any type is, and every use of it is the one nominal type. A
//! variant binds no name: once the types are defined, each variant is found
//! with its enum, by the declaration its file's view of types names by what it
//! writes, and given a key of its own ([`DeclaredVariant`],
//! `declare_variants`). Variants are seen as `def` methods are, by a flat view
//! of each file ([`File::variant_view`]): the variants of the files it sees,
//! its own included, and of a session's input, the earlier inputs' that it does
//! not shadow, by its enum's key and its name. A variant is static data: a use
//! of it is a constant, and makes its file need nothing. The bounds and
//! payloads are resolved before any file is checked (`check_enum_items`).
//!
//! # Context
//!
//! A context (see the checker's [Context](crate::check::context#context)) is in no
//! tree, no view and no record. The scan finds every file's declarations
//! (`declare_contexts`), each with its declaration's key, of its name and
//! of its file's path, or `input N` for a session's input, and its default's
//! binding; their types are resolved before any file is checked
//! (`check_context_items`, with the enums', `check_declared_items`), and a
//! declaration of a name a file of the scan declared before it is "context
//! `arch` is declared twice", labelled "first declared here". A session's
//! input inherits the earlier inputs' declarations ([`Earlier`](super::Earlier)), and one
//! of a name it declares shadows theirs. Once a file is checked, the slot
//! of each of its defaults' closures is recorded (`ForLater`), so a later
//! file's read of one calls it from there. A file that reads a context
//! declared, with a default, in another file needs that file
//! (`context_needs`), the need shown in a cycle as "reads the context `x`
//! of `lib/a.gcsl` here".
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
//! What a `DT` names is known only once the views are, which are made of the
//! trees the definitions are leaves of. So the scan makes the trees, resolves
//! the imports and makes the views (`scan_trees`), and then finds the key of
//! each definition and reference whose `DT` is not known yet
//! (`name_namespaces`): the leaf the file's view has at its path, if it is a
//! `def` marked `#[def_type]` (`def_type_key`, through
//! [`FileScope`](super::FileScope)'s leaf's owner), and for a session's input
//! whose view has nothing of its first name, the def-type of that path an
//! earlier input declared ([`Earlier`](super::Earlier)). Each is given its key
//! ([`Ast::set_def_type_key`](crate::ast::Ast::set_def_type_key)), which its
//! file is then taken to spell. If a definition got one, the trees, views and
//! types are forgotten (`unscan`) and made again, once, with the definitions
//! under their keys; a scan with no definition makes them once. A reference's
//! key changes no tree, and its read of the definition is resolved with the
//! other references ([`resolve`](super::resolve)): it needs the definition's
//! file, and its `DT` the def-type's, and a definition's `DT` makes its file
//! need the def-type's.
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

use std::mem;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{AttributeId, ItemId, ItemKind, PathSegment, Syntax, TypeExprId, TypeLink};
use crate::check::{BUILTIN_ATTRIBUTES, BUILTIN_TYPE_NAMES, check_context_types};
use crate::diagnostic::Diagnostic;
use crate::mentions::{self, TypeRead};
use crate::names::{self, Name, Names};
use crate::source::Span;
use crate::types::TypeTable;

use super::{
    DeclaredContext, DeclaredType, DeclaredVariant, File, InheritedContext, MethodRef, Scan,
    SeenVariant, Target, context_in, count, in_file,
};

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
    pub(super) fn target_type(&self, target: Target, name: Name) -> Option<u32> {
        self.target_types(target)
            .into_iter()
            .find(|&decl| self.type_name(decl) == Some(name))
    }

    /// The name the type `decl`, an index in [`Scan::types`], declares.
    pub(super) fn type_name(&self, decl: u32) -> Option<Name> {
        self.types.get(decl as usize).map(|declared| declared.name)
    }

    /// The enums file `index` declares, each by its item and its key (see
    /// [Enums](#enums)).
    pub(super) fn file_enums(&self, index: u32) -> Vec<(ItemId, Name)> {
        let Some(file) = self.files.get(index as usize) else {
            return Vec::new();
        };
        file.types
            .iter()
            .filter_map(|&decl| {
                let declared = self.types.get(decl as usize)?;
                declared.ty.is_none().then_some(())?;
                let (key, _) = self
                    .type_store
                    .enum_of(self.type_base.saturating_add(decl))?;
                Some((declared.item, key))
            })
            .collect()
    }

    /// What declares what file `file` declares, for the keys of its enums
    /// and variants ([`names::enum_key`]): its path, or `input N` for a
    /// session's input, every input of a session sharing one file's place.
    pub(crate) fn origin(&self, file: u32) -> String {
        match self.input_number {
            Some(number) if file >= self.standard => format!("input {number}"),
            _ => self
                .files
                .get(file as usize)
                .map_or_else(String::new, |file| file.path.clone()),
        }
    }

    /// The contexts file `index` declares, as a session's later input
    /// inherits them (see [Context](#context)).
    pub(super) fn own_contexts(&self, index: u32) -> Vec<InheritedContext> {
        self.contexts
            .iter()
            .filter(|context| context.file == index)
            .map(|context| (context.name, context.key, context.binding))
            .collect()
    }

    /// The name of the type `decl`, an index in [`Scan::types`], as text.
    pub(super) fn show_type(&self, decl: u32) -> &str {
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
    pub(super) fn type_links(&self, index: u32) -> Vec<(TypeExprId, TypeLink)> {
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
                    // Through a name the file reads as `Error`, the checker's,
                    // quietly (see [Scan
                    // errors](crate::workspace::scanning#scan-errors)).
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
    pub(super) fn applied_link(
        &self,
        decl: u32,
        (shown, given): (&str, u32),
        span: Span,
    ) -> TypeLink {
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
pub(super) fn declare_types(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
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
                // errors](crate::workspace::scanning#scan-errors)).
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
    mark_attributes(scan, &types, errors);
    scan.types = types;
    scan.dir_types = dir_types;
}

/// Marks each of `types` that its file marks `#[attribute]` as an
/// attribute's declaration, by its number, and as deferring if it is marked
/// `#[defers]` too; but one named like a built-in attribute, or one with
/// type parameters, which is reported, and `#[defers]` on a type that is no
/// attribute's, which is reported too.
fn mark_attributes(scan: &mut Scan, types: &[DeclaredType], errors: &mut Vec<Diagnostic>) {
    for (at, decl) in (0u32..).zip(types) {
        let Some(file) = scan.files.get(decl.file as usize) else {
            continue;
        };
        if !file.ast.is_attribute(decl.item) {
            if file.ast.is_defers(decl.item) {
                errors.push(
                    Diagnostic::error(
                        format!(
                            "`#[defers]` marks an attribute's declaration, and `{}` is none",
                            scan.names.get(decl.name)
                        ),
                        decl.span,
                    )
                    .with_note("mark the type `#[attribute]` too")
                    .in_input(decl.file),
                );
            }
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
        if file.ast.is_defers(decl.item) {
            scan.type_store
                .mark_deferring(scan.type_base.saturating_add(at));
        }
    }
}

/// Where one name of a file's view of types comes from, as [`type_views`]
/// meets it: its declaration, by its index in [`Scan::types`], and whether
/// it reaches the view through an import, written at a span of the file,
/// or from a directory of the file's chain, a directory above its own
/// (`above`) or its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TypeSource {
    /// The declaration.
    pub(super) decl: u32,
    /// Where the file imports it, if it does.
    pub(super) import: Option<Span>,
    /// Whether it is a type of a directory above the file's own.
    pub(super) above: bool,
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
pub(super) fn type_views(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
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
        // `Error` (see [Scan errors](crate::workspace::scanning#scan-errors)).
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

/// Makes each file's view of methods ([`File::method_view`], see
/// [Methods](#methods)): the `def` methods of the other files of its
/// directory and of each directory above it, and of every file of each
/// module it imports, a directory's own files or a file, each once.
pub(super) fn method_views(scan: &mut Scan) {
    let by_dir = files_by_dir(scan);
    let mut views = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let mut view: HashMap<Name, Vec<MethodRef>> = HashMap::new();
        for source in visible_files(scan, index, &by_dir) {
            if source == index {
                continue;
            }
            let Some(other) = scan.files.get(source as usize) else {
                continue;
            };
            for (method, decl) in (0u32..).zip(other.ast.methods()) {
                if decl.public {
                    view.entry(decl.name).or_default().push(MethodRef {
                        file: source,
                        method,
                    });
                }
            }
        }
        views.push(view);
    }
    for (file, view) in scan.files.iter_mut().zip(views) {
        file.method_view = view;
    }
}

/// The files of each directory, by its index.
pub(super) fn files_by_dir(scan: &Scan) -> HashMap<u32, Vec<u32>> {
    let mut by_dir: HashMap<u32, Vec<u32>> = HashMap::new();
    for (index, file) in (0u32..).zip(&scan.files) {
        by_dir.entry(file.dir).or_default().push(index);
    }
    by_dir
}

/// The files whose methods and variants file `index` sees (see
/// [Methods](#methods)), each once, in order, the file itself first: the
/// files of its directory and of each directory above it, and of each
/// module it imports, a directory's own files or a file. `by_dir` lists
/// each directory's files.
pub(super) fn visible_files(scan: &Scan, index: u32, by_dir: &HashMap<u32, Vec<u32>>) -> Vec<u32> {
    let Some(file) = scan.files.get(index as usize) else {
        return Vec::new();
    };
    let mut sources: Vec<u32> = vec![index];
    sources.extend(
        scan.chain(file.dir)
            .flat_map(|dir| by_dir.get(&dir).into_iter().flatten().copied()),
    );
    for &module in &file.method_modules {
        match module {
            Target::Dir(dir) => sources.extend(by_dir.get(&dir).into_iter().flatten()),
            Target::File(other) => sources.push(other),
            _ => {}
        }
    }
    let mut seen: HashSet<u32> = HashSet::new();
    sources.retain(|&source| seen.insert(source));
    sources
}

/// Makes each file's view of variants ([`File::variant_view`], see
/// [Enums](#enums)), once the variants are found: the variants of the
/// files it sees ([`visible_files`]) and, for a session's input, the
/// earlier inputs', by their enum and name, each key once.
pub(super) fn variant_views(scan: &mut Scan) {
    let by_dir = files_by_dir(scan);
    let mut views = Vec::with_capacity(scan.files.len());
    for index in 0..count(scan.files.len()) {
        let seen: HashSet<u32> = visible_files(scan, index, &by_dir).into_iter().collect();
        let mut view: HashMap<(Name, Name), Vec<SeenVariant>> = HashMap::new();
        for (at, variant) in (0u32..).zip(&scan.variants) {
            if let Some((of, _)) = variant.enum_of
                && seen.contains(&variant.file)
            {
                let entry = view.entry((of, variant.name)).or_default();
                if !entry.iter().any(|seen| seen.key == variant.key) {
                    entry.push(SeenVariant {
                        key: variant.key,
                        declared: Some(at),
                    });
                }
            }
        }
        // A session's input sees its earlier inputs' variants too, but of
        // a name it declares a variant of, which shadows theirs.
        if index >= scan.standard {
            for &(of, name, key) in &scan.inherited_variants {
                let entry = view.entry((of, name)).or_default();
                if entry.is_empty() {
                    entry.push(SeenVariant {
                        key,
                        declared: None,
                    });
                }
            }
        }
        views.push(view);
    }
    for (file, view) in scan.files.iter_mut().zip(views) {
        file.variant_view = view;
    }
}

/// Finds every variant of an enum the scan's files declare (see
/// [Enums](#enums)), once their types are defined, each with its enum, by
/// the declaration the file's view of types names by what it writes, if
/// that is an enum's, and its key, made of its enum's and of its file.
pub(super) fn declare_variants(scan: &mut Scan) {
    let mut found = Vec::new();
    for index in 0..count(scan.files.len()) {
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        let variants: Vec<(ItemId, TypeExprId, Name, Span)> = file
            .ast
            .items(file.ast.root())
            .iter()
            .filter_map(|&item| match file.ast.item(item).kind {
                ItemKind::Variant {
                    of,
                    name,
                    name_span,
                    ..
                } => Some((item, of, name, name_span)),
                _ => None,
            })
            .collect();
        if variants.is_empty() {
            continue;
        }
        let links = scan.type_links(index);
        let origin = scan.origin(index);
        for (item, of, name, span) in variants {
            let enum_of =
                links
                    .iter()
                    .find(|&&(at, _)| at == of)
                    .and_then(|(_, link)| match link {
                        TypeLink::Decl(decl) => scan.type_store.enum_of(*decl),
                        TypeLink::Error(_) => None,
                    });
            let of_text =
                enum_of.map_or_else(String::new, |(key, _)| scan.names.get(key).to_owned());
            let text = names::variant_key(scan.names.get(name), &of_text, &origin);
            let key = scan.names.intern(&text);
            found.push(DeclaredVariant {
                file: index,
                item,
                name,
                span,
                of,
                enum_of,
                key,
            });
        }
    }
    scan.variants = found;
    variant_views(scan);
}

/// Finds every context the scan's files declare (see [Context](#context)),
/// each with its declaration's key, of its name and of its file.
pub(super) fn declare_contexts(scan: &mut Scan) {
    let mut found = Vec::new();
    for index in 0..count(scan.files.len()) {
        let Some(file) = scan.files.get(index as usize) else {
            continue;
        };
        let declared: Vec<(u32, Name, Span, Option<Name>)> = (0u32..)
            .zip(file.ast.contexts())
            .map(|(decl, context)| (decl, context.name, context.name_span, context.binding))
            .collect();
        if declared.is_empty() {
            continue;
        }
        let origin = scan.origin(index);
        for (decl, name, span, binding) in declared {
            let text = names::context_key(scan.names.get(name), &origin);
            let key = scan.names.intern(&text);
            found.push(DeclaredContext {
                file: index,
                decl,
                name,
                span,
                key,
                binding,
            });
        }
    }
    scan.contexts = found;
}

/// Resolves the declared types of the contexts of the scan's files from
/// number `from` on, once their `type` items are checked (see
/// [Context](#context)), lent the workspace's `names`, interning into
/// `types`, which records them; and reports each declaration of a name a
/// file of the scan declared before it, "context `arch` is declared twice",
/// labelled at the first. Returns the errors found, each in its file.
pub(super) fn check_context_items(
    (scan, from): (&mut Scan, u32),
    names: &mut Names,
    types: &mut TypeTable,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut first: HashMap<Name, (u32, Span)> = HashMap::new();
    for context in &scan.contexts {
        match first.get(&context.name) {
            Some(&(earlier, at)) if context.file >= from => {
                let shown = names.get(context.name);
                errors.push(context_in(
                    Diagnostic::error(format!("context `{shown}` is declared twice"), context.span)
                        .in_input(context.file),
                    earlier,
                    at,
                    "first declared here",
                ));
            }
            Some(_) => {}
            None => {
                first.insert(context.name, (context.file, context.span));
            }
        }
    }
    let mut by_file: Vec<Vec<(u32, Name)>> = vec![Vec::new(); scan.files.len()];
    for context in &scan.contexts {
        if let Some(entry) = by_file.get_mut(context.file as usize) {
            entry.push((context.decl, context.key));
        }
    }
    for (index, file) in (0u32..).zip(&mut scan.files).skip(from as usize) {
        let Some(contexts) = by_file.get(index as usize) else {
            continue;
        };
        if contexts.is_empty() || file.ast.syntax() == Syntax::Unbalanced {
            continue;
        }
        file.ast.replace_names(mem::take(names));
        let found = check_context_types(&file.ast, types, contexts);
        *names = file.ast.replace_names(Names::new());
        errors.extend(in_file(found, index));
    }
    errors
}

/// The conflict in file `index`'s view of types of the type `name` between
/// the later source and the earlier one (see [`type_views`]).
pub(super) fn type_conflict(
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
/// ([`TypeStore::settle`](crate::ast::TypeStore::settle)); and expands each
/// file's tree with them
/// ([`Ast::expand_types`](crate::ast::Ast::expand_types)), a use of an
/// erroneous one erroneous, its check failing quietly. A scan with no type to
/// define and nothing that reads one costs one look at each file's type
/// expressions.
pub(super) fn define_types(scan: &mut Scan, errors: &mut Vec<Diagnostic>) {
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
        if let Some(ty) = decl.ty {
            scan.type_store
                .declare(base.saturating_add(at), (&file.ast, ty), links);
        } else {
            let origin = scan.origin(decl.file);
            let text = names::enum_key(scan.names.get(decl.name), &origin);
            let key = scan.names.intern(&text);
            scan.type_store
                .declare_enum(base.saturating_add(at), (key, decl.name), decl.span);
        }
    }
    declare_variants(scan);
    declare_contexts(scan);
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
    // errors](crate::workspace::scanning#scan-errors)).
    for (file, links) in scan.files.iter_mut().zip(&links) {
        file.ast.expand_types(&scan.type_store, links);
    }
}

/// The error that the types of `component`, indices in [`Scan::types`]
/// that refer to each other, `edges` being what each refers to, refer to
/// themselves: "type `A` refers to itself", at the first of them in path
/// then source order, labelled at each step of a shortest cycle through it
/// ("`B` uses `A` here", at the name of `A` in `B`'s definition).
pub(super) fn type_cycle(scan: &Scan, component: &[u32], edges: &[Vec<u32>]) -> Option<Diagnostic> {
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
