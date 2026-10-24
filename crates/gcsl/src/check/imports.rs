//! Checking imports and directory scope: what a file's imports name, and the
//! references through them to other files and directories.
//!
//! # Directory scope
//!
//! The files of a [`crate::workspace`] are checked as the inputs of a
//! session are, in the workspace's order, each by `Checker::in_workspace`
//! from a scope joined from the scopes of its directory and of each
//! directory above (only their declarations of names the file spells, which
//! are all it can look up, so that checking stays linear in the files),
//! but with what the scan found: the file's mentions are
//! resolved against its view (the merged trees of its directory chain and
//! what it imports, see [`crate::workspace`], "The view of a file"), and a
//! context gives every cell of the workspace its slot (its node's index
//! among the workspace's nodes, slots below every private one, the
//! built-in functions' the first; a session's input's own cells take the
//! slots after the session's instead) and, once the file that computes it
//! is checked, its type. A cell of another file
//! is read as a cell of this file is: a load of its slot at the top level,
//! and inside a function or a `lazy` a read of the top level's frame, never
//! a capture, since it is written before this file runs. Only a root has a
//! binding, committed to its directory's scope once the last file
//! contributing to it is checked (its type is whole only then), and every
//! `def` has one, so calls of another file's `def` are checked as calls of
//! an earlier function. A root that several sources of the file's view
//! have (`com` in a directory and the directory above) is never read
//! through a binding: it is a node of the view (see [Imports](#imports)).
//! An interior node shared with other files of the directory has their
//! children in its type. The file's own leaves are as in [Top-level
//! fields](crate::check::toplevel#top-level-fields).
//!
//! What a file declares privately (`let`, `let mut`, `fn`, and the soft
//! fields of its spreads) is not committed: later files are checked from a
//! scope without those bindings, though their slots stay taken, since the
//! file's closures read them. A top-level `let`, `let mut`, `:=`, dotted
//! `let` or `fn` of a name that is a root of its directory's tree or of a
//! directory above, declared in another file, is "`a` is a field of this
//! directory", labelled on the declaration, unless what the file's view has
//! of the name is shadowable, which it then shadows silently
//! ([`crate::workspace::FileScope::yields`]); within a file, "`a` is a field
//! of this file" stands as it was, but for a shadowable root (see [Shadowed
//! declarations](crate::check::items#shadowed-declarations)).
//!
//! A file is checked after every file it needs, so their cells' types are
//! known; there is nothing to be ready across files, since a needed file
//! runs as a whole first. Readiness ("cannot be used here yet") is within
//! a file only. Effects follow: every effect of a needed file happens
//! before the first of the file that needs it.
//!
//! # Imports
//!
//! An `import` names a node of another part of the workspace, or a module
//! of the standard library (see [`crate::workspace`], "Imports" and "The
//! standard library"), and the scan has resolved it, and reported its
//! clashes ("`lib` is imported in this file", "`version` is a field of this
//! directory"), before any file is checked. A lone file and a REPL input
//! are checked beside the standard library, with no workspace: a path of
//! anything else names nothing, and the checker, which knows its context is
//! a lone one, reports "`lib` is not a built-in or standard module, and
//! there is no `workspace.gcsl` above this file" at the import, with the
//! input's other errors. A file checked with no context at all ([`check`](super::check))
//! has nothing to import: "`import` needs a workspace: no `workspace.gcsl`
//! above this file". Otherwise the item itself checks nothing.
//!
//! An imported name, and a root of the file's view that several sources
//! have, has no binding. A name the scope does not bind nearer (a
//! parameter, a local `let`, anything but the top level, shadows it) and
//! that the file's [`crate::workspace::FileScope`] reads through its view
//! ([`crate::workspace::FileScope::reads_through`]) is a reference,
//! resolved like a reference to a field: to the longest prefix the view
//! declares, the accesses after it ordinary ones. A name the view has as
//! something shadowable (the implicit `#[shadowable] import * from
//! prelude`'s names among them, see [`crate::workspace`], "Shadowable
//! declarations") yields instead to a binding of its name that is not a
//! declaration of the view's own order
//! ([`crate::workspace::FileScope::yields`],
//! `Checker::binding_shadows_view`): a top-level `let` or `fn` of the file,
//! or an earlier session input's variable, shadows it, and no clash is
//! reported ("`len` is a field of this directory" is not); a field root of
//! the file or one its directory chain committed is what the view says it
//! is. An import a
//! session's input inherits from an earlier input
//! ([`crate::workspace::FileScope::inherited`]) is shadowed as an earlier
//! input's variable is, with no clash reported: by the input's top-level
//! `fn`, field, `def` or `:=` of its name in the whole input (the scan
//! drops it), and by a top-level `let` from its point on. What the prefix
//! reaches decides
//! what is read, always a cell, so never a capture: a load of its slot at
//! the top level, a read of the top level's frame inside a function or a
//! `lazy`.
//!
//! - A node of a directory's tree is read from its cell, its type the
//!   cell's. A `def`'s cell is its function's slot (the merged node of a
//!   `def` is never written), and its type the function's.
//! - A value file's value is read from the cell the file stores it in, its
//!   type the file's.
//! - A directory, a file, a file's part of a shared node, or a node of the
//!   view that several sources have and no leaf of the file is under, read
//!   whole, is a record the reading file assembles itself: before the walk,
//!   the checker gives each such target the file's mentions reach a private
//!   top-level slot of its own (nested parts first), with the record type
//!   of its roots or children, each a cell's slot and type as above;
//!   lowering builds them before the file's first item. Where a reference
//!   is checked the record is only looked up, never allocated, so that no
//!   such slot is taken from a function's frame.
//! - A node of the view that several sources have and a leaf of the file
//!   is under is one of the file's field nodes, numbered after its tree's
//!   ([`crate::mentions::Mentions::views`]): it has a private slot, and,
//!   read whole, it is computed at its point in the file's schedule as an
//!   interior node is (`Checker::compute_view`), its type the record of its
//!   children in the view's order (root directory first), the file's own
//!   as computed by then and every other one another file's cell or a
//!   record assembled as the file started. Its readiness and cycles are an
//!   interior node's: "`com` has `com.b.v`, and ...".
//!
//! A call of a `def` reached through an import (`f(x)` after `import f from
//! lib`, or `lib.f(x)`) is of a known callee, `KnownCallee::Imported`, whose
//! measured parameters and defaults the workspace carries
//! ([`KnownParams`](super::KnownParams)), so its call-site diagnostics are
//! those of a `def` of the directory, and it is given its defaults (see "Calls"
//! under [Functions](crate::check::functions#functions)); the prelude's `half`
//! is such a `def`. So is a call of a `def` named by a dotted path, through an
//! import or by its path from another file of the directory chain
//! (`dev.peios.f(x)`), which reads the root the chain committed, a record that
//! knows no function: the reference is resolved in the file's view to the
//! merged node (`Checker::committed_callee`), whose measured parameters and
//! defaults the workspace carries as a `def`'s of one name (a dotted one has no
//! binding: they are its function's).
//!
//! The `fileroot` module (see [`crate::workspace`], "The standard library")
//! is no record: a reference through its whole import resolves, as any
//! does, to the longest prefix it declares, a member or the module itself
//! ([`crate::workspace::Target::Fileroot`]), and is typed by
//! `Checker::fileroot_reference`, resolving to [`VarRef::Fileroot`], which
//! lowering makes the member's own node. `fileroot.this`,
//! `fileroot.workspace` and `fileroot.caller` are each a `String`;
//! `fileroot.helper`, only as the callee of a call (not in parentheses), is
//! a pure function of nothing to `{}`, so `fileroot.helper()` may be called
//! from any function and at the top level, and its arguments are bound as
//! any function's ("expected 0 arguments, found 1"). The module's own name
//! is typed `{}`, which nothing reads. Every other use is an error, naming
//! the module by the name the file imports it as: the module read whole,
//! "`fileroot`'s members are read as `fileroot.caller`"; `helper` not
//! called, "`fileroot.helper` is not a value"; a name it lacks, "`fileroot`
//! has no `nope`" (`Checker::fileroot_lacks`); and a member used as what it
//! is not, the error any `String` gets, as `fileroot.this(1)`'s "cannot
//! call a value of type String". A nearer binding of the name shadows the
//! import, as it shadows any. The checker records the file's place, given
//! by the workspace's context ([`Place`](super::Place)), in the resolution, for lowering.
//!
//! A session's input is checked beside the standard library as a lone file
//! is, from the session's scope ([`crate::workspace`]'s context marks it a
//! session's): the variables of earlier inputs are read as a session's
//! input reads them, captured by a function that reads one, rather than
//! through the top level's frame as a directory's committed declarations
//! are.

use crate::ast::{Effect, ExprId, ExprKind, ImportForm, ImportMember, ItemId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::mentions::{Need, Resolve};
use crate::names::Name;
use crate::source::Span;
use crate::suggest;
use crate::types::TyId;
use crate::workspace::{Context, Target};

use super::{Checker, FILEROOT_MEMBERS, Fileroot, Slot, VarRef};

impl<'a> Checker<'a> {
    /// Checks the import item `id`, spanning `span` (see the parser's
    /// [Imports](crate::parser::imports#imports)). Checked with no workspace at all,
    /// as [`check`](super::check) checks a file alone, there is nothing for its path to
    /// name, which is an error at the import, reported here with the
    /// input's other errors. A lone file and a session's input are checked
    /// beside the standard library, whose modules the scan has resolved; a
    /// path of anything else is the error, "`lib` is not a built-in or
    /// standard module, and there is no `workspace.gcsl` above this file".
    /// In a workspace the scan has resolved it, and its names are read
    /// where they are used (see [Imports](#imports)): the item itself
    /// checks nothing.
    pub(super) fn check_import(&mut self, id: ItemId, span: Span) {
        // An import of a def-type's definitions reads the def-type, which
        // the workspace has found one, as a definition does.
        if let ItemKind::Import(decl) = self.ast.item(id).kind
            && let ImportForm::Members(range) = self.ast.import_decl(decl).form
        {
            for member in self.ast.import_members(range) {
                if let ImportMember::DefType { def, .. } = *member {
                    self.check_def_type_expr(self.ast.def_type(def).dt);
                }
            }
        }
        let Some(workspace) = self.workspace else {
            self.errors.push(Diagnostic::error(
                "`import` needs a workspace: no `workspace.gcsl` above this file",
                span,
            ));
            return;
        };
        if !workspace.needs_workspace(id) {
            return;
        }
        let ItemKind::Import(decl) = self.ast.item(id).kind else {
            return;
        };
        let names = self.ast.names();
        let segments = self.ast.path(self.ast.import_decl(decl).path);
        let path: Vec<&str> = segments
            .iter()
            .filter_map(|segment| Some(names.get(segment.name()?)))
            .collect();
        // A module of one name may be a standard one misspelled.
        let suggestion = match (segments, path.as_slice()) {
            ([segment], [text]) => {
                let modules = crate::STANDARD
                    .iter()
                    .map(|&(stem, _)| stem)
                    .chain([crate::workspace::BUILTINS, Fileroot::MODULE])
                    .map(|module| (module, 0));
                suggest::best(text, modules).map(|best| (segment.span, best.to_owned()))
            }
            _ => None,
        };
        self.errors.push(
            Diagnostic::error(
                format!(
                    "`{}` is not a built-in or standard module, and there is no \
                     `workspace.gcsl` above this file",
                    path.join("/")
                ),
                span,
            )
            .with_suggestion(suggestion),
        );
    }

    /// The cells of the file's imports (see [Imports](#imports)): its
    /// result's cell if it is a value file another file imports, and a
    /// private slot for each record it reads whole through an import, from
    /// the targets its mentions reach.
    pub(super) fn declare_imports(&mut self, workspace: &'a Context<'a>) {
        if let Some(cell) = workspace.value_slot() {
            self.value_cell = cell;
            self.frame_size = self.frame_size.max(cell.0.saturating_add(1));
        }
        // A shadowed declaration's reads are not assembled for it (see
        // [Shadowed declarations](crate::check::items#shadowed-declarations)).
        let shadowed = &self.mentions.shadowed;
        let mut reached: Vec<Target> = self
            .mentions
            .foreign
            .iter()
            .enumerate()
            .filter(|&(position, _)| shadowed.get(position) != Some(&true))
            .flat_map(|(_, foreign)| foreign)
            .filter_map(|&(need, _)| match need {
                Need::Field(node) => workspace.scope.target(node),
                Need::Function(_) => None,
            })
            .collect();
        // The records under the view's nodes this file assembles at their
        // points are assembled as it starts, as an imported record is.
        let tree = self.mentions.tree.nodes.len();
        for at in 0..self.mentions.views.len() {
            let number = u32::try_from(tree + at).unwrap_or(u32::MAX);
            if self
                .field_cells
                .get(tree + at)
                .is_some_and(|cell| cell.slot != Slot::NONE)
            {
                reached.extend(
                    workspace
                        .scope
                        .view_children(number)
                        .into_iter()
                        .filter_map(|(_, child)| workspace.scope.target(child)),
                );
            }
        }
        for target in reached {
            self.imported_record(workspace, target);
        }
    }

    /// The private slot and type of the record of `target`, read whole
    /// through an import, allocated with its nested records the first time
    /// (see [Imports](#imports)); `None` for a target that is a cell, and
    /// for the `fileroot` module and its members, which have no record.
    pub(super) fn imported_record(
        &mut self,
        workspace: &'a Context<'a>,
        target: Target,
    ) -> Option<(Slot, TyId)> {
        if matches!(target, Target::Node(_) | Target::Fileroot(_))
            || workspace.value_of(target).is_some()
        {
            return None;
        }
        if let Some(at) = self.imported_targets.iter().position(|&(t, _)| t == target) {
            let slot = self.imported_records.get(at)?.0;
            let ty = self.imported_targets.get(at)?.1;
            return Some((slot, ty));
        }
        let mut fields = Vec::new();
        let mut erroneous = false;
        for (name, child) in workspace.target_fields(target) {
            // A def-type's namespace, or its metas', binds nothing a record
            // holds (see [Def-types](crate::check::deftypes#def-types)).
            if crate::names::is_namespace_key(self.ast.names().get(name)) {
                continue;
            }
            let (slot, ty) = match child {
                Target::Node(node) => (workspace.cell_slot(node), workspace.cell_type(node)),
                part => self
                    .imported_record(workspace, part)
                    .unwrap_or((Slot::NONE, TyId::ERROR)),
            };
            erroneous |= ty == TyId::ERROR || slot == Slot::NONE;
            fields.push((name, slot, ty));
        }
        let ty = if erroneous {
            TyId::ERROR
        } else {
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        // Every slot is named, so that `slot_names` stays aligned with the
        // slots. A record is only reached through an import of this file,
        // so a name is always found; without one, no slot is taken and the
        // reference is left unresolved rather than misnaming later slots.
        let first = fields.first().map(|&(name, _, _)| name);
        let name = workspace.target_name(target).or(first)?;
        let slot = Slot(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        self.frame_size = self.frame_size.max(self.next_slot);
        self.slot_names.push(name);
        self.imported_records.push((slot, fields));
        self.imported_targets.push((target, ty));
        Some((slot, ty))
    }

    /// The cell a reference through an import to `target` reads, and its
    /// type: a merged node's cell (a `def`'s is its function's slot), an
    /// imported value file's value, or the record the file assembles of a
    /// directory, a file or a part (see [Imports](#imports)). A record is
    /// only looked up here, never allocated: [`Self::declare_imports`]
    /// allocated one, among the top-level slots, for every whole read the
    /// file's mentions reach, before the walk; allocating one here, inside
    /// a function, would take a slot of the function's frame.
    pub(super) fn import_cell(
        &self,
        workspace: &'a Context<'a>,
        target: Target,
    ) -> Option<(Slot, TyId)> {
        if let Target::Node(node) = target {
            return Some((workspace.cell_slot(node), workspace.cell_type(node)));
        }
        if let Some(cell) = workspace.value_of(target) {
            return Some((workspace.slot(cell), workspace.cell_type(cell)));
        }
        let at = self
            .imported_targets
            .iter()
            .position(|&(t, _)| t == target)?;
        let slot = self.imported_records.get(at)?.0;
        let ty = self.imported_targets.get(at)?.1;
        Some((slot, ty))
    }

    /// Records that the expression `id`, a reference through an import,
    /// reads `slot`: a `Load` at the top level, a `Top` inside a function.
    pub(super) fn read_import(&mut self, id: ExprId, slot: Slot) {
        self.expr_vars[id.index()] = if self.functions.is_empty() {
            VarRef::Local(slot)
        } else {
            VarRef::Fn(slot)
        };
    }

    /// The type of the reference `id` to `node`, a node of another file of
    /// the workspace in the numbering of the file's scope, if `id` is a path
    /// through its root that nothing of this file shadows: read from the
    /// node's slot as a cell of this file is, the other file having been
    /// checked and run before this one. A root alone is read through its
    /// committed binding, as a variable of an earlier input is.
    pub(super) fn foreign_reference(
        &mut self,
        workspace: &'a Context<'a>,
        node: u32,
        id: ExprId,
    ) -> Option<TyId> {
        let mut base = id;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let shadowed = self
            .lookup(name)
            .is_some_and(|(index, _)| index >= self.top_end);
        if shadowed {
            return None;
        }
        // Through an import, a root alone too, which has no binding (see
        // [Imports](#imports)).
        let (slot, ty) = match workspace.scope.target(node) {
            Some(Target::Fileroot(member)) => {
                return Some(self.fileroot_reference(id, (base, name), member));
            }
            Some(target) => {
                if let Target::Node(merged) = target {
                    self.imported_callees.insert(id, merged);
                }
                match self.import_cell(workspace, target) {
                    Some(cell) => cell,
                    // A record only a shadowed declaration reads is not
                    // assembled: erroneous for it, which is never run.
                    None if self.checking_shadowed => (Slot::NONE, TyId::ERROR),
                    None => return None,
                }
            }
            None if base == id => return None,
            None => {
                // A `def`'s cell is its function's slot, and now typed as
                // its function, so never its merged node's unwritten one.
                let global = workspace.scope.merged(node)?;
                // Called, a `def` named by a dotted path of another file of
                // the chain is a known callee, as one imported is.
                self.imported_callees.insert(id, global);
                (workspace.cell_slot(global), workspace.cell_type(global))
            }
        };
        self.read_cell(id, base, slot, ty);
        Some(ty)
    }

    /// The type of the reference `id`, through `base`, the name `name` that
    /// a whole import of the `fileroot` module binds, to the module's member
    /// `member`, or to the module itself (`None`), as nothing nearer shadows
    /// the name (see [Imports](#imports)): `String` for `this`, `workspace`
    /// and `caller`, which lowering makes a constant or a read of the
    /// running frame's caller; for `helper` as the callee of a call
    /// (`call_site`), a pure function of nothing to `{}`, to which the
    /// call's arguments are bound as any function's. The module read whole
    /// is "`fileroot`'s members are read as `fileroot.caller`", and `helper`
    /// anywhere but as a callee "`fileroot.helper` is not a value", each
    /// naming the module by the name the file imports it as. The module's
    /// own name has no value: it is typed `{}`, which nothing reads.
    pub(super) fn fileroot_reference(
        &mut self,
        id: ExprId,
        (base, name): (ExprId, Name),
        member: Option<Fileroot>,
    ) -> TyId {
        let written = self.ast.names().get(name).to_owned();
        let span = self.ast.get(id).span;
        self.expr_types[base.index()] = TyId::EMPTY;
        match member {
            None => {
                self.errors.push(
                    Diagnostic::error(
                        format!(
                            "`{written}`'s members are read as `{written}.{}`",
                            Fileroot::Caller.name()
                        ),
                        span,
                    )
                    .with_note(FILEROOT_MEMBERS),
                );
                TyId::ERROR
            }
            Some(Fileroot::Helper) if self.call_site == Some(id) => {
                self.expr_vars[id.index()] = VarRef::Fileroot(Fileroot::Helper);
                let none: [(Option<Name>, TyId); 0] = [];
                self.types.fn_type(none, TyId::EMPTY, Effect::Pure)
            }
            Some(Fileroot::Helper) => {
                let helper = Fileroot::Helper.name();
                self.errors.push(
                    Diagnostic::error(format!("`{written}.{helper}` is not a value"), span)
                        .with_note(format!(
                            "it is only ever called, as `{written}.{helper}()`, which makes the \
                             calls after it pass on this function's caller"
                        )),
                );
                TyId::ERROR
            }
            Some(member) => {
                self.expr_vars[id.index()] = VarRef::Fileroot(member);
                TyId::STRING
            }
        }
    }

    /// The type of the field access `id`, `record.name`, if `record` is a
    /// name that a whole import of the `fileroot` module binds, which
    /// nothing nearer shadows, and `name` is none of its members: the error
    /// "`fileroot` has no `nope`", naming the module as the file imports it,
    /// for any field the module lacks (a member reaches
    /// [`Checker::fileroot_reference`] instead).
    pub(super) fn fileroot_lacks(
        &mut self,
        id: ExprId,
        record: ExprId,
        name: Name,
    ) -> Option<TyId> {
        let workspace = self.workspace?;
        let ExprKind::Name(module) = self.ast.get(record).kind else {
            return None;
        };
        let (node, at) = workspace.scope.reference(self.ast, record)?;
        if at != record
            || workspace.scope.target(node) != Some(Target::Fileroot(None))
            || self
                .lookup(module)
                .is_some_and(|(index, _)| index >= self.top_end)
        {
            return None;
        }
        let names = self.ast.names();
        let text = names.get(name);
        let members = Fileroot::ALL.map(|member| (member.name(), 0));
        let suggestion = suggest::best(text, members).map(|best| {
            let span = self.ast.written_name(self.ast.get(id).span.end, text);
            (span, best.to_owned())
        });
        self.errors.push(
            Diagnostic::error(
                format!("`{}` has no `{text}`", names.get(module)),
                self.ast.get(id).span,
            )
            .with_note(FILEROOT_MEMBERS)
            .with_suggestion(suggestion),
        );
        self.expr_types[record.index()] = TyId::EMPTY;
        Some(TyId::ERROR)
    }

    /// Before the walk checks the top-level item `id`, a `let`, `let mut`,
    /// `:=` or private `fn` declaring `name`, which is not the root of a
    /// field of this file: "`a` is a field of this directory" if `name` is
    /// a declaration of another file of the directory or of a directory
    /// above (see [Directory scope](#directory-scope)).
    pub(super) fn check_directory_clash(&mut self, id: ItemId, name: Name) {
        let Some(workspace) = self.workspace else {
            return;
        };
        // A root of this file is the clash of one file, its own errors'; a
        // name of the implicit prelude, or of an import a session's input
        // inherits, yields to the declaration silently; and a name the scan
        // reported a clash or a conflict of is not reported again (see
        // [`crate::workspace`], "Scan errors").
        if workspace.scope.root(name).is_none()
            || self.mentions.tree.root(name).is_some()
            || workspace.scope.yields(name)
            || workspace.scope.inherited(name)
            || workspace.poisoned(name)
        {
            return;
        }
        let text = self.ast.names().get(name);
        self.field_clash = Some(id);
        self.errors.push(
            Diagnostic::error(
                format!("`{text}` is a field of this directory"),
                self.name_span(id),
            )
            .with_note(
                "a field or `def` declared at the top level of a file is visible in every \
                 file of its directory and of the directories below it, so its name cannot \
                 also be a variable",
            ),
        );
    }
}
