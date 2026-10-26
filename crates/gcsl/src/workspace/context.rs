//! The checker's view of the workspace, `Context`: the cells, types, methods
//! and contexts a file being checked can reach.

use std::rc::Rc;

use crate::ast::{AttributeId, DefTypeId, ExprKind, ItemId, ItemKind, TypeExprKind};
use crate::check::{KnownParams, Place, Slot};
use crate::names::{self, Name, Names};
use crate::source::Span;
use crate::types::TyId;

use super::{
    CheckedMethod, ContextDefault, DeclaredContext, File, FileScope, ForLater, ForeignMethod, Scan,
    SeenContext, SeenVariant, Target, target_roots,
};

/// What the checker knows of the workspace while it checks one of its
/// files (see [Directory scope](crate::check::imports#directory-scope)).
#[derive(Debug)]
pub(crate) struct Context<'a> {
    /// The workspace.
    pub(super) scan: &'a Scan,
    /// The file being checked.
    pub(super) file: &'a File,
    /// The scope the file's references are resolved in.
    pub(crate) scope: FileScope<'a>,
    /// The file's index.
    pub(super) index: u32,
    /// The type of each cell of the workspace, by its slot: `Error` until
    /// the file that computes it is checked.
    pub(super) cell_types: &'a [TyId],
    /// The slot each cell of the workspace is read from, by its number: its
    /// own, but a `def`'s function's slot, known once its file is checked.
    pub(super) cell_slots: &'a [Slot],
    /// For each cell that is a `def`'s, by its number, what a call of its
    /// function needs to know of its parameters, known once its file is
    /// checked ([`KnownParams`]): which are present because its body
    /// measures them, for the call-site form of the implicit-await error,
    /// and their defaults, which a call of it through an import or by its
    /// path gives the parameters it leaves unfilled.
    pub(super) cell_known: &'a [Option<Rc<KnownParams>>],
    /// The methods and the contexts' defaults of the files checked so far.
    pub(super) for_later: &'a ForLater,
    /// Whether the file is a session's input, whose scope is the session's:
    /// its variables are read as a session's input reads them, captured
    /// by a function, not read through the top level's frame as a
    /// directory's committed declarations are (see [`check_input`](super::check_input)).
    pub(super) session: bool,
    /// Where the file is, as the driver resolved it: its directory's index
    /// in the run's table, and what `fileroot.this` and `fileroot.workspace`
    /// are in it (see the module documentation, "The standard library").
    pub(super) place: Place,
}

/// What a [`Context`] reads cells by: the slot each cell is read from, for
/// each `def`'s cell what a call of it needs to know of its parameters, and
/// the methods and the contexts' defaults of the files checked.
pub(super) type CellReads<'a> = (&'a [Slot], &'a [Option<Rc<KnownParams>>], &'a ForLater);

/// What a name read as an attribute's names in a file (see the parser's
/// [Attributes](crate::parser::attributes#attributes)).
#[derive(Clone, Copy, Debug)]
pub(crate) enum AttributeLookup<'a> {
    /// No type of the file's view.
    Nothing,
    /// A type of the file's view that is not marked `#[attribute]`.
    NotAttribute,
    /// A type marked `#[attribute]`, known to a host as this, and whether
    /// it is marked `#[defers]` too.
    Attribute(&'a AttributeId, bool),
}

impl<'a> Context<'a> {
    /// What `name`, read as an attribute's, names in the file: a type of its
    /// view of types, as a type's name is read (see [Types](crate::workspace::declared#types)), and
    /// whether it is marked `#[attribute]`.
    pub(crate) fn attribute(&self, name: Name) -> AttributeLookup<'a> {
        match self.file.type_view.get(&name) {
            None => AttributeLookup::Nothing,
            Some(&decl) => match self.scan.type_store.attribute(decl) {
                Some(id) => AttributeLookup::Attribute(id, self.scan.type_store.defers(decl)),
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
    pub(super) fn new(
        scan: &'a Scan,
        index: u32,
        cell_types: &'a [TyId],
        (cell_slots, cell_known, for_later): CellReads<'a>,
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
            for_later,
            session: false,
            place: scan.place(index),
        })
    }

    /// Whether the file numbered `file` is one of the standard library's.
    pub(crate) fn is_standard(&self, file: u32) -> bool {
        file < self.scan.standard
    }

    /// The methods named `name` of other files that the file sees, whose
    /// files are checked, each by its position among those of the name the
    /// file sees, which [`Context::method`] reads by (see
    /// [Methods](crate::workspace::declared#methods)).
    pub(crate) fn methods(&self, name: Name) -> Vec<(u32, ForeignMethod)> {
        let Some(found) = self.file.method_view.get(&name) else {
            return Vec::new();
        };
        (0u32..)
            .zip(found)
            .filter_map(|(index, &method)| {
                let checked = self.for_later.methods.get(&method)?.clone();
                let decl = self
                    .scan
                    .files
                    .get(method.file as usize)?
                    .ast
                    .methods()
                    .get(method.method as usize)?;
                let foreign = ForeignMethod {
                    file: method.file,
                    span: decl.name_span,
                    checked,
                };
                Some((index, foreign))
            })
            .collect()
    }

    /// What the check of the method of another file that the file sees as
    /// the `index`th of those named `name` found, if its file is checked
    /// (see [Methods](crate::workspace::declared#methods)).
    pub(crate) fn method(&self, name: Name, index: u32) -> Option<&'a CheckedMethod> {
        let found = self.file.method_view.get(&name)?.get(index as usize)?;
        self.for_later.methods.get(found)
    }

    /// The context named `name` (see [Context](crate::workspace::declared#context)): the file's own
    /// declaration of it; or for a session's input, an earlier input's; or
    /// else the first of the scan's files' (a later one is reported where
    /// it is declared). `None` if none declares it.
    pub(crate) fn context(&self, name: Name) -> Option<SeenContext> {
        let declared = |context: &DeclaredContext| SeenContext {
            key: context.key,
            default: match context.binding {
                None => ContextDefault::None,
                Some(binding) if context.file == self.index => ContextDefault::Binding(binding),
                Some(_) => {
                    ContextDefault::Foreign(self.for_later.contexts.get(&context.key).copied())
                }
            },
            site: Some((context.file, context.span)),
        };
        let mut scan = self.scan.contexts.iter().filter(|c| c.name == name);
        if let Some(own) = scan.clone().find(|c| c.file == self.index) {
            return Some(declared(own));
        }
        if self.index >= self.scan.standard
            && let Some(&(_, key, binding)) = self
                .scan
                .inherited_contexts
                .iter()
                .find(|&&(earlier, ..)| earlier == name)
        {
            return Some(SeenContext {
                key,
                default: binding.map_or(ContextDefault::None, ContextDefault::Binding),
                site: None,
            });
        }
        scan.next().map(declared)
    }

    /// The names of the contexts the file sees, for suggestions (see
    /// [Context](crate::workspace::declared#context)).
    pub(crate) fn context_names(&self) -> Vec<Name> {
        let mut found: Vec<Name> = self.scan.contexts.iter().map(|c| c.name).collect();
        if self.index >= self.scan.standard {
            found.extend(self.scan.inherited_contexts.iter().map(|&(name, ..)| name));
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The variants named `name` of the enum of the key `of` that the file
    /// sees (see [Enums](crate::workspace::declared#enums)): one, or several declared in different
    /// files, which is an error of any use.
    pub(crate) fn variants(&self, of: Name, name: Name) -> &'a [SeenVariant] {
        self.file
            .variant_view
            .get(&(of, name))
            .map_or(&[], Vec::as_slice)
    }

    /// The names of the variants of the enum of the key `of` that the file
    /// sees, sorted, for a note.
    pub(crate) fn variant_names(&self, of: Name) -> Vec<Name> {
        let mut found: Vec<Name> = self
            .file
            .variant_view
            .keys()
            .filter(|&&(enum_key, _)| enum_key == of)
            .map(|&(_, name)| name)
            .collect();
        found.sort_unstable();
        found
    }

    /// Where the variant the scan's [`Scan::variants`] has as number
    /// `declared` is declared: its file and its name's span.
    pub(crate) fn variant_site(&self, declared: u32) -> Option<(u32, Span)> {
        let variant = self.scan.variants.get(declared as usize)?;
        Some((variant.file, variant.span))
    }

    /// Whether the file sees a method named `name` of another file (see
    /// [Methods](crate::workspace::declared#methods)).
    pub(crate) fn sees_method(&self, name: Name) -> bool {
        self.file.method_view.contains_key(&name)
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
    /// (see [Syntax errors](crate::workspace::parse#syntax-errors)).
    pub(crate) fn sees_broken(&self) -> bool {
        self.scan.sees_broken(self.index)
    }

    /// The roots of the file's view, each with the rank of its scope for a
    /// suggestion (see [Suggestions](crate::check::names#suggestions)), the nearer
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
    /// [Scan errors](crate::workspace::scanning#scan-errors)).
    pub(crate) fn poisoned(&self, name: Name) -> bool {
        self.file.poisoned.binary_search(&name).is_ok()
    }

    /// Whether the scan of the workspace found errors, its own or syntax
    /// errors, which it recovered from: nothing runs, so a check that found
    /// an erroneous type with nothing to report, of a declaration the scan
    /// poisoned, fails quietly (see [Scan errors](crate::workspace::scanning#scan-errors)).
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
    /// [Def-types](crate::workspace::declared#def-types)). `None` otherwise, as for one a session's
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
        // The definition's parameter, the second, or the third after a
        // variant's.
        let meta = ast
            .params(ast.fn_decl(function).params)
            .iter()
            .skip(1)
            .take(2)
            .find_map(|param| match ast.written_type(param.ty) {
                TypeExprKind::Fn {
                    meta: Some(meta), ..
                } => Some(meta),
                _ => None,
            })?;
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
