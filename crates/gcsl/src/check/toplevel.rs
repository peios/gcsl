//! Checking top-level functions and fields: their cells, the order they are
//! computed in, the views of other files and the cycles among them.
//!
//! # Top-level functions
//!
//! A *top-level function* of an input is a named function item (`fn f`)
//! or a `def` (`def f`, or `def dev.peios.f`, named by a dotted path: see
//! below) that is directly an item of the input's outermost block;
//! nothing inside a nested block, a loop body, a labelled block, a function
//! or a `lazy` is one. The input's top-level fields are order-free too (see
//! [Top-level fields](#top-level-fields)); together with its top-level
//! functions they are its *top-level declarations*. Everything else is
//! ordered as described above: `let`, `let mut`, spreads, `:=`, statements,
//! every field of an inner block, and every `fn` or `def` that is not at
//! the top level of an input.
//!
//! **Names.** A top-level function's name is in scope in its whole input,
//! above its declaration as well as below, and inside every function and
//! `lazy` of the input, unless a nearer declaration shadows it there. As
//! the input starts (`Checker::declare_top_fns`), every top-level function
//! is declared, in source order, with the type its signature declares, so
//! its binding comes first among the input's bindings and its slot is one
//! of the input's first. The input's other bindings are declared after
//! them as the walk reaches them, so a scope's bindings still occupy slots
//! in order, and a session's [`Scope`](super::Scope) keeps that order. A top-level
//! function hides a variable or field of its name from an earlier input,
//! and a soft field of its name that a spread of this input defines (the
//! spread still defines the record's field; a `def` of that name overrides
//! it, as a field does). Two top-level `fn`s of one name are "function `f`
//! is declared twice", at the second (two `def`s, or a `def` and a field,
//! are the field defined twice); a top-level `let`, `let mut`, field or
//! `:=` whose name, or its path's first segment, is a top-level function's
//! is "`f` is a function of this file", but a field all of whose leaves
//! are `#[shadowable]`, which a `fn` of its name shadows in the whole input
//! (see [Shadowed declarations](crate::check::items#shadowed-declarations)). In inner blocks a
//! name shadows as it always has.
//!
//! **What a function sees.** A function sees the variables and fields
//! declared above its own declaration and copies the ones it uses when it
//! is made, as any function does. A top-level variable declared below it
//! is "undefined variable" inside it, with a label on that declaration,
//! "`k` is declared here, below the function". What is new is only how one
//! top-level function refers to another of its input, or to a top-level
//! field of it: through the top level's frame, by [`VarRef::Fn`], never by
//! a capture. That is what lets two functions refer to each other, and a
//! closure never wait for a field.
//!
//! **Mentions.** What follows is decided from one relation, found from the
//! tree before the input is checked (see [`crate::mentions`]): an item or
//! a body *mentions* a top-level function when a name in it is the
//! function's and no declaration the tree shows shadows it there. A name
//! that a spread inside a nested block defines is not known before
//! checking, so it does not shadow: a read of it counts as a mention
//! although it reads the spread's field. The relation only ever
//! over-approximates, which can only make a body be checked earlier or an
//! item be held to the rule of [When a function may be
//! used](crate::check::schedule#when-a-function-may-be-used).
//!
//! **Groups, and the order bodies are checked in.** A caller needs more
//! than a callee's written signature (what the body decides: inferred
//! parameters, an inferred result, narrowed callbacks), and that must not
//! depend on which is written first. The top-level functions are grouped
//! by mutual recursion, the strongly connected components of "mentions"
//! between them, and every body is checked exactly once
//! (`Checker::check_group`): before the walk checks a top-level item that
//! is not itself a top-level function, the bodies of the functions it
//! mentions, and of those they mention, and so on, that are not checked
//! yet (`Checker::check_mentioned`); after the last item, the rest
//! (`Checker::check_remaining_bodies`). In both, the groups a group
//! mentions come first, then the group's members in source order. A use of
//! a top-level function from outside its group sees the function fully
//! resolved, exactly as a use below a function's declaration does. A use
//! from inside its group, its own body included, sees the declared type
//! and is treated as a recursive call: the declared type at the bare
//! positions, arguments measured conservatively, no `KnownCallee`,
//! nothing iterated. A function whose signature or body is erroneous is,
//! for its users, an erroneous function declared above them.
//!
//! **The view a body is checked in.** A body checked before the walk
//! reaches item `P` sees, of the input's top level, the bindings declared
//! above whichever is higher, `P` or its own declaration, and every
//! top-level function, and everything of earlier inputs; a body checked
//! when the walk reaches it, or after the last item, sees the bindings
//! above its own declaration, which is what a function always saw. The
//! bindings are in declaration order, so the view is a prefix, and
//! `Checker::check_top_body` makes it by setting the suffix aside while the
//! body is checked and putting it back after: slots and `measured` are not
//! touched. A body checked at an item above its own declaration reads
//! nothing declared between the two: a read of a name that a top-level
//! item between them declares (by `let`, a field, a dotted path's first
//! segment, or a `:=` of a variable that is not mutable) is not resolved
//! to an older binding but makes the function not usable at the item (see
//! below), and so is any read of a variable or field of the top level when
//! a top-level spread lies between them, since its names are not known
//! yet. That spread rule is conservative: it holds whether or not the
//! spread defines the name.
//!
//! **Dotted names.** A `def` named by a dotted path, `def
//! dev.peios.count(n: Int): Int { ... }`, is exactly a `def` whose name is
//! its path, in every rule above and below: what differs is only where its
//! function is kept and how it is read. It is the leaf of its path in the
//! tree of top-level fields (see [Top-level fields](#top-level-fields)), and
//! has no binding, since no variable could name it: the leaf's cell, a slot
//! of the top level's frame after the roots', holds its closure, stored
//! where the closure is made, and holds its type
//! (`Checker::type_dotted_defs`): the type its signature declares, and,
//! once its group is checked (`Checker::finish_group`), the type its body
//! gives it, as a plain function's binding has. A use of it is a read of
//! its path, a reference whose longest declared prefix is its leaf, which
//! reads the cell (`Checker::field_reference`): from top-level code a slot
//! of the frame, from inside any function or `lazy` [`VarRef::Fn`] of it,
//! the node a plain function's mention is, so mutual recursion with any
//! top-level function works as between plain ones, and a call from inside
//! its group is treated as a recursive call (`Checker::is_recursive`, by the
//! cell's slot). Once its group is checked, a call by its path is of a
//! known callee, as a call by a plain name is (`Checker::known_callee`).
//! There is no short self-name: inside `def dev.peios.count`, `count` is
//! whatever else binds it, if anything. Its diagnostics name it by its
//! path ("`dev.peios.count` cannot be used here yet"). When the walk
//! reaches it, its path joins the record of the input as any dotted field's
//! does, its leaf typed as the function is by then and retyped when its
//! group is checked, and its root's name clashes as a dotted field's
//! (`Checker::reach_dotted_def`); one whose leaf today's rules reject (a
//! leaf twice, a leaf under a leaf or over another) is no top-level
//! function but the field it is written as, reported as such. A read of a
//! prefix of its path whole (`dev.peios`, `dev`) is a record of the leaves
//! under it, its closure among them: it mentions the function (see
//! [`crate::mentions`], "Dotted `def`s"), so the read waits for the
//! function as a use of it does, and the function's group is checked before
//! the reader. The prefix's type is the record of its children's when it is
//! computed, which may be before that group is checked, so when the check
//! changes the function's type, every node computed above its leaf is
//! retyped (`Checker::retype_above`), before anything reads it; a node of a
//! workspace file's view above it too. A function that reads a prefix of
//! its own path whole is no cycle: the record holds its closure, which
//! reads the record when it runs.
//!
//! # Top-level fields
//!
//! A field declared directly at the top level of an input, plain (`a = e`),
//! quoted (`"a b" = e`), dotted (`a.b.c = e`) or a `def` (`def f`, `def
//! a.b.f`), is a *top-level field*, and, like a top-level function, it is
//! visible in the whole input.
//!
//! **The tree.** The top-level field items form a tree by their paths
//! ([`crate::mentions::FieldTree`], built before the walk): each item is a
//! *leaf*, and every proper prefix of a leaf's path an *interior* node, the
//! record of its children in the order of their first items. Every node has
//! a *cell*, a slot of the top level's frame written once: a root's is its
//! binding's slot, declared as the input starts, after the top-level
//! functions (`Checker::declare_field_cells`; a `def`'s of one name is its
//! function's), and every other node's is a slot after those that no name
//! finds, a dotted `def`'s holding its closure. The
//! rules of [Dotted fields](crate::check::records#dotted-fields) that keep a leaf from also
//! being interior hold in either order of the two items.
//!
//! **References.** A *reference* to a top-level field is `NAME`, `NAME.f`,
//! `NAME.f.g`, ..., a variable and a chain of field accesses directly on
//! it, where `NAME` resolves to a top-level field root of this input. It
//! means the longest declared prefix of its path, a leaf or an interior
//! node, whose type it has (`Checker::field_reference`); the segments after
//! that prefix are ordinary field accesses. So `com.x.lib` reads only what
//! is under `com.x.lib`, not all of `com`. Anything else (`(com).x`,
//! `f(com).x`) means the whole root.
//!
//! **Names.** A field root's name follows the rules for a top-level
//! function's: it hides an earlier input's binding of the name, and
//! overrides a soft field of the name from a spread of this input, above it
//! or below (see [Spreads and soft
//! assignment](crate::check::records#spreads-and-soft-assignment)); a top-level `let`, `let
//! mut`, `:=` or dotted `let` of the name is "`a` is a field of this file",
//! with the field's first item labelled "declared here"; a top-level
//! function of the name is "`a` is a function of this file" at the field.
//! A root all of whose leaves are `#[shadowable]` is shadowed instead,
//! silently, by a `let` (a dotted one too) from its item on and by a `fn`
//! in the whole input (see [Shadowed
//! declarations](crate::check::items#shadowed-declarations)).
//!
//! **What an initialiser sees.** As a function body does: the variables of
//! the input's top level declared above the field's own item, everything of
//! earlier inputs, and every top-level declaration. A read inside a function
//! expression or a `lazy` the initialiser builds counts as the
//! initialiser's. A `let` below the field is "undefined variable", labelled
//! "`k` is declared here, below the field", with the note "a field's
//! initialiser sees the variables declared above it".
//!
//! **Points.** Each node has a *point*, the item of the outermost block
//! before which its cell is computed ([`crate::mentions::Schedule`]). An
//! ordered item (anything that is not a top-level declaration) *needs* every
//! top-level declaration it mentions and what those need: a leaf what its
//! initialiser mentions, an interior node its children, a function what its
//! body mentions. A leaf's point is the earliest of its own item and the
//! points of what needs it, directly or through functions; an interior
//! node's is the earliest of the item after its last child's and the points
//! of what needs it. A function's closure keeps its own rule ([When a
//! function may be used](crate::check::schedule#when-a-function-may-be-used)) and never pulls a
//! field earlier: a closure reads fields when it runs. Before each item,
//! the nodes whose point it is are computed, dependencies first, otherwise
//! in source order; after the last item, the rest. So a field nothing
//! mentions earlier is computed at its own line, and one mentioned earlier
//! just before the first item that needs it; the effects of its initialiser
//! happen there, which may be above its line. A field computed before its
//! own item is not code that runs at its line: the "this is never run"
//! warning neither starts at nor counts it; one computed at its own item is
//! warned about and counted as any item. Between the files of a workspace,
//! the order is by file: a file's effects all happen before the first
//! effect of a file that needs it ([Directory scope](crate::check::imports#directory-scope)).
//!
//! **When a field may be used.** A leaf may be computed at a point only if
//! what it reads is bound by then, as a function may be used only once what
//! it copies is bound. Whatever needs, at its point, a leaf that cannot be
//! computed by then (one that reads a top-level variable bound at or after
//! that point, or a mutable variable, which makes the leaf computed at its
//! own item with the value the variable has there, or, through the chain, a
//! function or leaf that is not ready) is "`a.c` cannot be used here yet",
//! at the mention, with the chain of [When a function may be
//! used](crate::check::schedule#when-a-function-may-be-used) and "has" for the steps from an
//! interior node to what it holds: "`com` has `com.x.app.name`, and
//! `com.x.app.name` reads `version`, which is bound here". The rules for a
//! name declared, or a spread, between an early use and the declaration
//! apply to leaves as to functions. `Checker::check_uses` skips the search
//! for a reason when nothing a mention needs is ever blocked
//! (`Checker::closure_ready`), so a chain of N fields costs N steps.
//!
//! **Cycles.** Mutual recursion between functions is fine, but a cycle of
//! "needs" through a leaf or an interior node is "`a` depends on itself",
//! at the first item of the cycle's leaf earliest in the source, labelled
//! along the cycle ("`a` uses `b` here", ...), once per cycle, with the
//! note "a field cannot be computed from itself, even through a function"
//! (`Checker::report_cycles`, from [`crate::mentions::Mentions::cycles`]).
//! The nodes on a cycle are erroneous for their users. The rule asks what
//! is mentioned, not what is called, and a mention inside a `lazy` or a
//! function expression the initialiser only builds counts, so `steps = lazy
//! { run(steps.name) }` is an error: a function that reads the field is the
//! way to write it. It is conservative in another way too: the first
//! segment of a dotted field of an inner block counts as a mention of the
//! top-level root of that name, so `a.c = { a.b = 1 }` is a cycle.
//!
//! **Types: initialisers are checked when they are needed.** A field has no
//! declared type, which is why a cycle through one is an error rather than
//! a conservative view. (An annotated field has one, its annotation, which
//! is its type; a cycle through one is an error all the same.) Each leaf's
//! initialiser is checked exactly once, against its annotation if it has
//! one (`Checker::compute_fields`, walking the schedule's order with no
//! recursion): before the first item that needs it, after the
//! declarations it needs, or after the last item, with the top-level scope
//! cut to the bindings above whichever is higher, that item or the field's
//! own, as a body checked early is (see [Top-level
//! functions](#top-level-functions)). An interior node's type is the record
//! of its children's types. A reference has its node's type.

use std::rc::Rc;

use crate::ast::{ExprId, ExprKind, FnId, ItemId, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::mentions::{self, FieldLeaf, Need, Resolve};
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;
use crate::workspace::Context;

use super::{
    Binding, BindingKind, BlockSpreads, BlockerKind, BodyState, Checker, FieldCell, FieldDecl,
    KnownParams, MadeAt, NO_NODE, SharedChildren, Slot, TopFn, VarRef, bareness_of,
};

impl<'a> Checker<'a> {
    /// As the input starts: finds what its top-level items mention (see
    /// [`crate::mentions`]) and declares each top-level function, in source
    /// order, with its declared type, so the bindings of the input's
    /// top-level functions take its first slots and come first among its
    /// bindings (see [Top-level functions](#top-level-functions)).
    pub(super) fn declare_top_fns(&mut self) {
        self.mentions = match self.workspace {
            Some(workspace) => workspace.file().mentions.clone(),
            None => mentions::find(self.ast),
        };
        self.find_def_types();
        self.top_start = self.bindings.len();
        for index in 0..self.mentions.functions.len() {
            let Some(&top) = self.mentions.functions.get(index) else {
                break;
            };
            let kind = match self.ast.item(top.item).kind {
                ItemKind::Field { .. } => BindingKind::Field,
                _ => BindingKind::Variable,
            };
            let span = self.ast.item(top.item).span;
            // A `def` named by a dotted path has no binding: its leaf's
            // cell, a slot after the roots', holds it, and is typed by it
            // (`Checker::declare_field_cells`).
            let binding = match top.name {
                None => usize::MAX,
                Some(name) => {
                    if kind == BindingKind::Variable {
                        self.check_directory_clash(top.item, name);
                    }
                    // A `fn` named like a shadowable field root of the input
                    // shadows it in the whole input: its binding is declared
                    // after the roots' (`Checker::declare_shadowing_fns`).
                    if kind == BindingKind::Variable && self.shadowable_root(name) {
                        usize::MAX
                    } else {
                        let ty = self.declared_type(top.function);
                        let binding = self.bindings.len();
                        self.declare_binding(name, ty, Some(span), kind);
                        binding
                    }
                }
            };
            let leaf = match top.name {
                None => self
                    .mentions
                    .tree
                    .item_leaf
                    .get(top.position)
                    .copied()
                    .flatten(),
                Some(_) => None,
            };
            self.top_fns.push(TopFn {
                item: top.item,
                expr: top.expr,
                function: top.function,
                name: top.name,
                binding,
                leaf,
                node: NO_NODE,
                cut: None,
                state: BodyState::Unchecked,
                ready: 0,
                blocker: None,
                made: MadeAt::Own,
            });
        }
        self.top_copies = vec![Vec::new(); self.top_fns.len()];
        self.declare_field_cells();
        self.type_dotted_defs();
        self.closure_ready = vec![None; self.mentions.node_count() + self.top_fns.len()];
    }

    /// Whether `name` is the root of a top-level field of the input all of
    /// whose leaves are marked `#[shadowable]`, which a top-level `let` or
    /// `fn` of the input shadows silently (see [Shadowed
    /// declarations](crate::check::items#shadowed-declarations)).
    pub(super) fn shadowable_root(&self, name: Name) -> bool {
        let tree = &self.mentions.tree;
        tree.root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            .is_some_and(|root| root.shadowable)
    }

    /// The binding of the `def` of one name whose item is `leaf`'s, if it
    /// is one: its cell is its function's binding. A `def` named by a
    /// dotted path has none, and its cell is a slot of its own.
    pub(super) fn plain_def_binding(&self, leaf: Option<FieldLeaf>) -> Option<usize> {
        let leaf = leaf?;
        self.top_fns
            .iter()
            .find(|top| top.item == leaf.item && top.name.is_some())
            .map(|top| top.binding)
    }

    /// As the input starts, once its field roots are declared: declares each
    /// top-level `fn` named like a shadowable field root of the input
    /// ([`Checker::shadowable_root`]), so that its binding, later than the
    /// root's, is the one its name finds in the whole input.
    pub(super) fn declare_shadowing_fns(&mut self) {
        for index in 0..self.top_fns.len() {
            let Some(top) = self.top_fns.get(index).copied() else {
                continue;
            };
            let (usize::MAX, Some(name)) = (top.binding, top.name) else {
                continue;
            };
            let ty = self.declared_type(top.function);
            let binding = self.bindings.len();
            let span = self.ast.item(top.item).span;
            self.declare_binding(name, ty, Some(span), BindingKind::Variable);
            if let Some(entry) = self.top_fns.get_mut(index) {
                entry.binding = binding;
            }
        }
    }

    /// As the input starts, after its top-level functions: declares the
    /// root of every top-level field that is not a `def` (a `def`'s is its
    /// function's binding), in order of first item, with no type yet, and
    /// gives every other node of the tree a slot of its own after them,
    /// which no name finds (see [Top-level fields](#top-level-fields)).
    /// Finds when each node is computed.
    pub(super) fn declare_field_cells(&mut self) {
        self.schedule = match self.workspace {
            Some(workspace) => workspace.file().schedule.clone(),
            None => self.mentions.schedule(),
        };
        self.scheduled = 0;
        let count = self.mentions.tree.nodes.len();
        if let Some(workspace) = self.workspace {
            self.declare_workspace_cells(workspace);
            return;
        }
        self.field_cells = Vec::with_capacity(count);
        let root_items = self.ast.items(self.ast.root());
        for index in 0..count {
            let Some(node) = self.mentions.tree.nodes.get(index) else {
                break;
            };
            let (name, parent, first) = (node.name, node.parent, node.first);
            let def = self.plain_def_binding(node.leaf);
            let cell = match (parent, def) {
                (_, Some(binding)) => FieldCell {
                    slot: self.bindings.get(binding).map_or(Slot::NONE, |b| b.slot),
                    binding: Some(binding),
                    ..FieldCell::NEW
                },
                (None, None) => {
                    let span = root_items.get(first).map(|&item| self.ast.item(item).span);
                    let binding = self.bindings.len();
                    let slot = self.declare_binding(name, TyId::ERROR, span, BindingKind::Field);
                    FieldCell {
                        slot,
                        binding: Some(binding),
                        ..FieldCell::NEW
                    }
                }
                (Some(_), None) => FieldCell::NEW,
            };
            self.field_cells.push(cell);
        }
        self.declare_shadowing_fns();
        self.top_end = self.bindings.len();
        self.report_cycles();
        // The cells of the nodes that are not roots, after the roots'.
        for index in 0..count {
            let named = self
                .mentions
                .tree
                .nodes
                .get(index)
                .is_some_and(|node| node.parent.is_none());
            if named {
                continue;
            }
            let slot = Slot(self.next_slot);
            self.next_slot = self.next_slot.saturating_add(1);
            self.frame_size = self.frame_size.max(self.next_slot);
            let name = self.mentions.tree.nodes.get(index).map(|node| node.name);
            if let Some(name) = name {
                self.slot_names.push(name);
            }
            if let Some(cell) = self.field_cells.get_mut(index) {
                cell.slot = slot;
            }
        }
        self.top_slots_end = self.top_slots_end.max(self.next_slot);
    }

    /// As the check ends: what a call of each of this input's functions in
    /// a later input, whose tree is gone, needs to know of its parameters
    /// ([`KnownParams`]: what the call-site form of the implicit-await error
    /// needs, and the defaults), for every binding of the outermost scope.
    pub(super) fn known(&mut self) -> Vec<Option<Rc<KnownParams>>> {
        let mut known = std::mem::take(&mut self.scope_known);
        for binding in self.bindings.get(known.len()..).unwrap_or_default() {
            known.push(binding.function.and_then(|function| {
                let attributes = self
                    .lowering
                    .fn_attributes
                    .get(&function)
                    .map_or(&[][..], |a| a);
                self.fn_resolutions
                    .get(function.index())
                    .map(|resolution| KnownParams::of(resolution, attributes))
            }));
        }
        known
    }

    /// As the check ends: the type of each node's cell (`Error` for a
    /// `def`'s), and, for a file of a workspace, the children of each node
    /// of its tree that it shares with other files, in the merged node's
    /// order, with their cells and types, its own children's as this check
    /// found them; empty outside a workspace (see [Directory
    /// scope](crate::check::imports#directory-scope)).
    pub(super) fn cell_tables(&self) -> (Vec<TyId>, SharedChildren) {
        let tree = self.mentions.tree.nodes.len();
        let types = self.cell_types();
        let Some(workspace) = self.workspace else {
            return (types, Vec::new());
        };
        let shared = (0..tree)
            .map(|node| {
                let node = u32::try_from(node).unwrap_or(u32::MAX);
                let children = workspace.merged_children(node).into_iter();
                children
                    .map(|(name, own, slot, ty)| match own {
                        Some(own) => self
                            .field_cells
                            .get(own as usize)
                            .map_or((name, slot, TyId::ERROR), |cell| {
                                (name, cell.slot, cell.ty.unwrap_or(TyId::ERROR))
                            }),
                        None => (name, slot, ty),
                    })
                    .collect()
            })
            .collect();
        (types, shared)
    }

    /// The type of each node's cell of the input's tree of top-level fields,
    /// by its index there, as the check has found it so far: a `def`'s is
    /// its function's binding's.
    pub(super) fn cell_types(&self) -> Vec<TyId> {
        let tree = self.mentions.tree.nodes.len();
        self.field_cells
            .iter()
            .take(tree)
            .map(|cell| {
                cell.ty
                    .filter(|&ty| ty != TyId::ERROR)
                    .or_else(|| Some(self.bindings.get(cell.binding?)?.ty))
                    .unwrap_or(TyId::ERROR)
            })
            .collect()
    }

    /// [`Checker::declare_field_cells`] for a file of a workspace: each
    /// node's cell is the slot of its merged node, allocated by the
    /// workspace, a `def`'s excepted (its function's binding's); each root
    /// that is not a `def` is bound in its cell; and each node shared with
    /// another file takes a private slot for the file's own record of it
    /// (see [Directory scope](crate::check::imports#directory-scope)).
    pub(super) fn declare_workspace_cells(&mut self, workspace: &'a Context<'a>) {
        let count = self.mentions.tree.nodes.len();
        let file = workspace.file();
        self.field_cells = Vec::with_capacity(count);
        self.own_records = vec![Slot::NONE; count];
        let root_items = self.ast.items(self.ast.root());
        for index in 0..count {
            let Some(node) = self.mentions.tree.nodes.get(index) else {
                break;
            };
            let (name, parent, first) = (node.name, node.parent, node.first);
            let global = file.global.get(index).copied().unwrap_or(u32::MAX);
            let slot = workspace.slot(global);
            let def = self.plain_def_binding(node.leaf);
            let cell = match (parent, def) {
                (_, Some(binding)) => FieldCell {
                    slot: self.bindings.get(binding).map_or(Slot::NONE, |b| b.slot),
                    binding: Some(binding),
                    ..FieldCell::NEW
                },
                (None, None) => {
                    let binding = self.bindings.len();
                    self.bind(Binding {
                        name,
                        slot,
                        ty: TyId::ERROR,
                        declared: root_items.get(first).map(|&item| self.ast.item(item).span),
                        kind: BindingKind::Field,
                        function: None,
                        callback: None,
                    });
                    FieldCell {
                        slot,
                        binding: Some(binding),
                        ..FieldCell::NEW
                    }
                }
                (Some(_), None) => FieldCell {
                    slot,
                    ..FieldCell::NEW
                },
            };
            // A session's input's `def` takes no slot of its own.
            if slot != Slot::NONE {
                self.frame_size = self.frame_size.max(slot.0.saturating_add(1));
            }
            self.field_cells.push(cell);
            if workspace.shared(global) {
                let own = Slot(self.next_slot);
                self.next_slot = self.next_slot.saturating_add(1);
                self.frame_size = self.frame_size.max(self.next_slot);
                self.slot_names.push(name);
                if let Some(entry) = self.own_records.get_mut(index) {
                    *entry = own;
                }
            }
        }
        // The nodes of the file's view with a leaf of the file under them:
        // a private slot each for the record the file assembles, if it
        // reads it whole (see [Imports](crate::check::imports#imports)).
        self.view_records = self
            .mentions
            .views
            .iter()
            .map(|view| (Slot::NONE, view.first, Vec::new()))
            .collect();
        for at in 0..self.mentions.views.len() {
            let number = count + at;
            let scheduled = self
                .schedule
                .point
                .get(number)
                .is_some_and(|&point| point != usize::MAX);
            let mut cell = FieldCell::NEW;
            if scheduled && let Some(view) = self.mentions.views.get(at) {
                cell.slot = Slot(self.next_slot);
                self.next_slot = self.next_slot.saturating_add(1);
                self.frame_size = self.frame_size.max(self.next_slot);
                self.slot_names.push(view.name);
            }
            self.field_cells.push(cell);
        }
        self.declare_shadowing_fns();
        self.top_end = self.bindings.len();
        self.report_cycles();
        self.declare_imports(workspace);
        self.top_slots_end = self.top_slots_end.max(self.next_slot);
    }

    /// Reports each cycle through the tree of top-level fields, once: "`a`
    /// depends on itself", at the item of the cycle's leaf that is first in
    /// the source, with a label on each mention along the cycle back to it
    /// (an interior node has its children, which it does not mention, so
    /// that step has none), and marks every node on it erroneous for its
    /// users (see [Top-level fields](#top-level-fields)).
    pub(super) fn report_cycles(&mut self) {
        self.cyclic_fns.clear();
        for cycle in self.mentions.cycles() {
            for &member in &cycle {
                match member {
                    Need::Field(node) => {
                        if let Some(cell) = self.field_cells.get_mut(node as usize) {
                            cell.cyclic = true;
                            cell.ready = usize::MAX;
                        }
                    }
                    Need::Function(function) => self.cyclic_fns.push(function),
                }
            }
            let leaf = cycle
                .iter()
                .filter_map(|&member| match member {
                    Need::Field(node) => self
                        .mentions
                        .tree
                        .nodes
                        .get(node as usize)
                        .and_then(|entry| entry.leaf)
                        .map(|leaf| (node, leaf)),
                    Need::Function(_) => None,
                })
                .min_by_key(|&(_, leaf)| leaf.position);
            let Some((start, leaf)) = leaf else {
                continue;
            };
            let start = Need::Field(start);
            // Breadth first inside the cycle, back to its start.
            let count = self.mentions.node_count() + self.top_fns.len();
            let mut parent: Vec<Option<(Need, Option<Span>)>> = vec![None; count];
            let (mut queue, mut head) = (vec![start], 0);
            let mut back = None;
            while let Some(&need) = queue.get(head) {
                head += 1;
                for (next, at) in self.mentions.needs(need) {
                    if !cycle.contains(&next) {
                        continue;
                    }
                    if next == start {
                        back = Some((need, at));
                        break;
                    }
                    if let Some(entry) = parent.get_mut(self.need_index(next))
                        && entry.is_none()
                    {
                        *entry = Some((need, at));
                        queue.push(next);
                    }
                }
                if back.is_some() {
                    break;
                }
            }
            // The steps from the start round to it, in order.
            let mut steps = Vec::new();
            let mut edge = back.map(|(from, at)| (from, start, at));
            while let Some((from, to, at)) = edge {
                steps.push((from, to, at));
                edge = (from != start)
                    .then(|| parent.get(self.need_index(from)).copied().flatten())
                    .flatten()
                    .map(|(up, at)| (up, from, at));
            }
            steps.reverse();
            let text = self.need_text(start);
            let mut error = Diagnostic::error(
                format!("`{text}` depends on itself"),
                self.name_span(leaf.item),
            )
            .with_note("a field cannot be computed from itself, even through a function");
            for (from, to, at) in steps {
                if let Some(at) = at {
                    let label = format!(
                        "`{}` uses `{}` here",
                        self.need_text(from),
                        self.need_text(to)
                    );
                    error = error.with_context(at, label);
                }
            }
            self.errors.push(error);
        }
    }

    /// Before the walk checks the item at `position` of the outermost block
    /// (or, with the number of items, after the last): computes every node
    /// of the tree of top-level fields whose point is there or earlier and
    /// that is not computed yet, in the order of the schedule, which puts
    /// what a node needs first. A leaf's initialiser is checked here,
    /// exactly once, after the bodies of the functions it mentions, in the
    /// view of the top level the walk has reached, which is the bindings
    /// above whichever is higher, the point or the leaf's own item; an
    /// interior node's type is the record of its children's. The walk is
    /// driven by the schedule, never by a reference, so it does not nest.
    pub(super) fn compute_fields(&mut self, position: usize) {
        while let Some(&node) = self.schedule.order.get(self.scheduled) {
            let point = self
                .schedule
                .point
                .get(node as usize)
                .copied()
                .unwrap_or(usize::MAX);
            if point > position {
                break;
            }
            self.scheduled += 1;
            self.compute_field(node);
        }
    }

    /// Computes the type of the node `node` of the tree of top-level fields
    /// (see [`Checker::compute_fields`]) and gives it to its root's binding.
    pub(super) fn compute_field(&mut self, node: u32) {
        let Some(entry) = self.mentions.tree.nodes.get(node as usize) else {
            self.compute_view(node);
            return;
        };
        let (leaf, children) = (entry.leaf, entry.children.clone());
        let Some(cell) = self.field_cells.get(node as usize).copied() else {
            return;
        };
        let ty = match leaf {
            // A `def`: its function's binding has its type, which its
            // group's check settles, when something mentions it or after
            // the last item, as for any top-level function. Its body is
            // not checked here: it may read fields computed after this
            // point, since a closure reads them when it runs, not when
            // it is made. A `def` named by a dotted path has no binding:
            // its cell has its type (`Checker::type_dotted_defs`), which
            // its group's check settles likewise.
            Some(leaf) if self.top_fns.iter().any(|top| top.item == leaf.item) => {
                if self.plain_def_binding(Some(leaf)).is_some()
                    && let Some(cell) = self.field_cells.get_mut(node as usize)
                {
                    cell.ty = Some(TyId::ERROR);
                }
                return;
            }
            Some(leaf) => {
                self.check_mentioned(leaf.position);
                let ItemKind::Field {
                    path,
                    ty: annotation,
                    ..
                } = self.ast.item(leaf.item).kind
                else {
                    return;
                };
                let segments = self.ast.path(path);
                // Checked at an item above its own, it reads nothing the
                // top level declares between the two, as a function body
                // checked early does not.
                let early = (self.root_position < leaf.position)
                    .then_some((self.root_position, leaf.position));
                (self.early, self.checking_leaf) = (early, Some(node));
                self.name_callee(segments, leaf.value);
                // TEMPORARY (alias rule): a top-level field of one name, not annotated, may bind a
                // built-in function whose typing is special (see the [inference
                // limits](https://learn.peios.org/gcsl/language/inference-limits/)).
                let alias = segments.len() == 1 && annotation.is_none();
                let outer = alias.then(|| self.alias_site.replace(leaf.value));
                // An annotated leaf has its annotation's type, which is what
                // every reader sees, in this file or another.
                let ty = self.annotated_value(annotation, leaf.value);
                if let Some(outer) = outer {
                    self.alias_site = outer;
                }
                self.callee_name = None;
                (self.early, self.checking_leaf) = (None, None);
                let function = self.direct_function(segments, leaf.value);
                if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
                    binding.function = function;
                }
                if let Some(cell) = self.field_cells.get_mut(node as usize) {
                    let blocked = cell
                        .blocker
                        .is_some_and(|blocker| blocker.kind != BlockerKind::Mutable);
                    if blocked || cell.cyclic {
                        cell.ready = usize::MAX;
                    }
                }
                if cell.cyclic { TyId::ERROR } else { ty }
            }
            None => self.interior_type(node, &children, cell.cyclic),
        };
        if let Some(cell) = self.field_cells.get_mut(node as usize) {
            cell.ty = Some(ty);
        }
        if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
            binding.ty = ty;
        }
    }

    /// The type of the interior node `node` of the tree of top-level
    /// fields, whose children are `children`, `cyclic` if it is on a
    /// cycle: the record of its children's types, with, in a workspace, its
    /// children of other files; `Error` if one of them is erroneous, or it
    /// is on a cycle.
    pub(super) fn interior_type(&mut self, node: u32, children: &[u32], cyclic: bool) -> TyId {
        let mut erroneous = false;
        let mut fields = Vec::with_capacity(children.len());
        for &child in children {
            let ty = self
                .field_cells
                .get(child as usize)
                .and_then(|cell| cell.ty)
                .unwrap_or(TyId::ERROR);
            erroneous |= ty == TyId::ERROR;
            if let Some(name) = self.mentions.tree.nodes.get(child as usize) {
                fields.push((name.name, ty));
            }
        }
        // A shared node's children of other files, computed before this
        // file is checked (see [Directory scope](crate::check::imports#directory-scope)).
        if let Some(workspace) = self.workspace {
            for (name, _, ty) in workspace.foreign_children(node) {
                erroneous |= ty == TyId::ERROR;
                fields.push((name, ty));
            }
        }
        if erroneous || cyclic {
            TyId::ERROR
        } else {
            self.types.record(fields)
        }
    }

    /// Once the group of the `def` named by a dotted path whose leaf is
    /// `leaf` is checked and has changed its leaf's type: gives each node
    /// above it already computed the type its children now have, and the
    /// roots' bindings theirs, as if its group had been checked before
    /// them, and so each node of the file's view with a leaf of the file
    /// under it (see [Top-level functions](#top-level-functions)). A node
    /// computed later reads the leaf's type as it is then.
    pub(super) fn retype_above(&mut self, leaf: u32) {
        let mut changed = vec![leaf];
        let mut at = self
            .mentions
            .tree
            .nodes
            .get(leaf as usize)
            .and_then(|entry| entry.parent);
        while let Some(node) = at {
            let Some(entry) = self.mentions.tree.nodes.get(node as usize) else {
                break;
            };
            let (children, parent) = (entry.children.clone(), entry.parent);
            let Some(cell) = self.field_cells.get(node as usize).copied() else {
                break;
            };
            if cell.ty.is_none() {
                break;
            }
            let ty = self.interior_type(node, &children, cell.cyclic);
            if let Some(cell) = self.field_cells.get_mut(node as usize) {
                cell.ty = Some(ty);
            }
            if let Some(binding) = cell.binding.and_then(|b| self.bindings.get_mut(b)) {
                binding.ty = ty;
            }
            changed.push(node);
            at = parent;
        }
        // The view's nodes come parents first, so children are retyped
        // before their parents here.
        let tree = self.mentions.tree.nodes.len();
        for at in (0..self.mentions.views.len()).rev() {
            let number = u32::try_from(tree + at).unwrap_or(u32::MAX);
            let reads_changed = self
                .mentions
                .views
                .get(at)
                .is_some_and(|view| view.children.iter().any(|child| changed.contains(child)));
            let computed = self
                .field_cells
                .get(number as usize)
                .is_some_and(|cell| cell.ty.is_some());
            if reads_changed && computed {
                self.compute_view(number);
                changed.push(number);
            }
        }
    }

    /// Computes the type of the node `node` of the file's view with a leaf
    /// of the file under it (numbered after the tree's nodes), the record
    /// of its children in the view's order, and the record lowering
    /// assembles at its point: each child's cell and type, the file's own
    /// nodes and view nodes as computed by now (the schedule puts them
    /// first), every other one another file's cell or a record the file
    /// assembled as it started (see [Imports](crate::check::imports#imports)).
    pub(super) fn compute_view(&mut self, node: u32) {
        let Some(workspace) = self.workspace else {
            return;
        };
        let tree = self.mentions.tree.nodes.len();
        let mut fields = Vec::new();
        let mut erroneous = false;
        for (name, child) in workspace.scope.view_children(node) {
            let (slot, ty) = if let Some(cell) = self.field_cells.get(child as usize) {
                (cell.slot, cell.ty.unwrap_or(TyId::ERROR))
            } else if let Some(target) = workspace.scope.target(child) {
                self.import_cell(workspace, target)
                    .unwrap_or((Slot::NONE, TyId::ERROR))
            } else if let Some(global) = workspace.scope.merged(child) {
                (workspace.cell_slot(global), workspace.cell_type(global))
            } else {
                (Slot::NONE, TyId::ERROR)
            };
            erroneous |= ty == TyId::ERROR || slot == Slot::NONE;
            fields.push((name, slot, ty));
        }
        let cyclic = self
            .field_cells
            .get(node as usize)
            .is_some_and(|cell| cell.cyclic);
        let ty = if erroneous || cyclic {
            TyId::ERROR
        } else {
            self.types
                .record(fields.iter().map(|&(name, _, ty)| (name, ty)))
        };
        if let Some(cell) = self.field_cells.get_mut(node as usize) {
            cell.ty = Some(ty);
        }
        if let Some(record) = (node as usize)
            .checked_sub(tree)
            .and_then(|at| self.view_records.get_mut(at))
            && let Some(cell) = self.field_cells.get(node as usize)
        {
            record.0 = cell.slot;
            record.2 = fields;
        }
    }

    /// The type of the reference `id` to `node`, a node of the file's view
    /// with a leaf of the file under it, if `id` is a path through its root
    /// that nothing of this file shadows: read from the private slot the
    /// file assembles it into at its point (see [Imports](crate::check::imports#imports)).
    pub(super) fn view_reference(&mut self, node: u32, id: ExprId) -> Option<TyId> {
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
        let cell = self.field_cells.get(node as usize).copied()?;
        let ty = cell.ty.unwrap_or(TyId::ERROR);
        self.read_cell(id, base, cell.slot, ty);
        Some(ty)
    }

    /// Records that the reference `id`, a path whose variable is `base`,
    /// reads the cell `slot` of type `ty`, as a reference through an import
    /// does: each prefix inside it typed as the record of the path below it.
    pub(super) fn read_cell(&mut self, id: ExprId, base: ExprId, slot: Slot, ty: TyId) {
        self.read_import(id, slot);
        self.expr_vars[base.index()] = self.expr_vars[id.index()];
        let mut at = id;
        let mut inner_ty = ty;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            inner_ty = if inner_ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.record([(name, inner_ty)])
            };
            self.expr_types[expr.index()] = inner_ty;
            at = expr;
        }
    }

    /// The type a field item's value `value` was given when its leaf was
    /// computed, if it is a top-level field's (see
    /// [`Checker::compute_fields`]): the walk does not check it again.
    pub(super) fn computed_value(&self, value: ExprId) -> Option<TyId> {
        if !self.functions.is_empty() {
            return None;
        }
        let node = self
            .mentions
            .tree
            .item_leaf
            .get(self.root_position)
            .copied()
            .flatten()?;
        let entry = self.mentions.tree.nodes.get(node as usize)?;
        if entry.leaf.is_none_or(|leaf| leaf.value != value) {
            return None;
        }
        self.field_cells.get(node as usize)?.ty
    }

    /// The type of a reference to a top-level field of the input, `id`,
    /// if it is one: a variable naming a field root that nothing shadows,
    /// or a chain of field accesses directly on one, which means the
    /// longest prefix of its path the tree declares (see [Top-level
    /// fields](#top-level-fields)). The accesses beyond that prefix are
    /// left to the caller as ordinary accesses: this answers only for the
    /// expression that names the node, recording where its value is read.
    pub(super) fn field_reference(&mut self, id: ExprId) -> Option<TyId> {
        if self.workspace.is_some_and(Context::has_poison) {
            let mut base = id;
            while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
                base = expr;
            }
            if let ExprKind::Name(name) = self.ast.get(base).kind
                && self.reads_poisoned(name, self.lookup(name))
            {
                return Some(TyId::ERROR);
            }
        }
        let (node, at) = match self.workspace {
            Some(workspace) => workspace.scope.reference(self.ast, id)?,
            None => self.mentions.tree.reference(self.ast, id)?,
        };
        if at != id {
            return None;
        }
        if let Some(workspace) = self.workspace
            && node as usize >= self.mentions.tree.nodes.len()
        {
            if (node as usize) < self.mentions.node_count() {
                return self.view_reference(node, id);
            }
            return self.foreign_reference(workspace, node, id);
        }
        let root = self.root_of(node);
        let binding = self.field_cells.get(root as usize)?.binding?;
        let mut base = id;
        while let ExprKind::Field { expr, .. } = self.ast.get(base).kind {
            base = expr;
        }
        let ExprKind::Name(name) = self.ast.get(base).kind else {
            return None;
        };
        let (index, found) = self.lookup(name)?;
        if index != binding || node == root {
            return None;
        }
        let cell = self.field_cells.get(node as usize).copied()?;
        let root_source = self.resolve(index, found);
        let source = match root_source {
            VarRef::Fn(_) => VarRef::Fn(cell.slot),
            _ => VarRef::Local(cell.slot),
        };
        // The reference reads the node's cell; its variable, the root's.
        self.expr_vars[id.index()] = source;
        self.expr_vars[base.index()] = root_source;
        let ty = cell.ty.unwrap_or(TyId::ERROR);
        // The prefixes inside the reference are not read: each is typed as
        // the record of the path below it, down to the node.
        let mut at = id;
        let mut inner_ty = ty;
        while let ExprKind::Field { expr, name } = self.ast.get(at).kind {
            inner_ty = if inner_ty == TyId::ERROR {
                TyId::ERROR
            } else {
                self.types.record([(name, inner_ty)])
            };
            self.expr_types[expr.index()] = inner_ty;
            at = expr;
        }
        Some(ty)
    }

    /// For a field `name` of type `ty` that a top-level spread of the
    /// operand `value`, written at `operand`, defines: if `name` is a
    /// top-level field of the input, defines nothing and writes no slot,
    /// holding the name's place in the record if nothing has it yet, and
    /// returns `true`.
    pub(super) fn spread_over_top_field(
        &mut self,
        (name, ty): (Name, TyId),
        (value, operand): (ExprId, Span),
    ) -> bool {
        let Some(slot) = self.top_declaration_slot(name) else {
            return false;
        };
        // A soft field's name as a dotted root is "already defined" in
        // either order: here the dotted paths came first.
        let tree = &self.mentions.tree;
        if let Some(root) = tree
            .root(name)
            .and_then(|root| tree.nodes.get(root as usize))
            && root.leaf.is_none()
            && let Some(&first) = self.ast.items(self.ast.root()).get(root.first)
            && self.ast.item(first).span.start < operand.start
        {
            let text = self.ast.names().get(name);
            self.errors.push(
                Diagnostic::error(format!("field `{text}` is already defined"), operand)
                    .with_context(self.name_span(first), "first defined here"),
            );
        }
        if self.block_field(name).is_none() {
            self.fields.push(FieldDecl {
                name,
                name_span: operand,
                ty,
                slot,
                open: None,
                spread: Some(value),
                rejected: false,
            });
        }
        self.spread_slots.push(Slot::NONE);
        true
    }

    /// The cell of the input's top-level field named `name`, a `def`
    /// included, if there is one. A private `fn` is not a field: a spread
    /// still defines the record's field of its name (slice 23).
    pub(super) fn top_declaration_slot(&self, name: Name) -> Option<Slot> {
        let root = self.mentions.tree.root(name)?;
        self.field_cells.get(root as usize).map(|cell| cell.slot)
    }

    /// The binding of the field `name` of type `ty` that the dotted item
    /// `id` opens: the top-level root's, `top_root`, declared as the input
    /// started, whose cell the item takes, or a new one.
    pub(super) fn open_binding(
        &mut self,
        top_root: Option<(usize, Slot)>,
        (name, ty): (Name, TyId),
        id: ItemId,
    ) -> usize {
        if let Some((binding, slot)) = top_root {
            self.item_slots[id.index()] = slot;
            return binding;
        }
        self.declare(name, ty, id, BindingKind::Field);
        self.bindings.len() - 1
    }

    /// The binding and cell of the input's top-level field root `name`,
    /// declared as the input started, if it is one and not a `def`.
    pub(super) fn top_root_cell(&self, name: Name) -> Option<(usize, Slot)> {
        let node = self.mentions.tree.root(name)?;
        let cell = self.field_cells.get(node as usize)?;
        Some((cell.binding?, cell.slot))
    }

    /// The root of the node `node` of the tree of top-level fields.
    pub(super) fn root_of(&self, node: u32) -> u32 {
        let mut at = node;
        while let Some(parent) = self
            .mentions
            .tree
            .nodes
            .get(at as usize)
            .and_then(|entry| entry.parent)
        {
            at = parent;
        }
        at
    }

    /// The type `function`'s parameters and return type declare, as its
    /// signature is written, without inferring anything from its body:
    /// what a use of a top-level function sees before its body is checked.
    /// `Error` if the signature is erroneous, which this reports nothing
    /// of: the function's own check does.
    pub(super) fn declared_type(&mut self, function: FnId) -> TyId {
        let (errors, warnings) = (self.errors.len(), self.warnings.len());
        let decl = *self.ast.fn_decl(function);
        let (params_mode, ret_mode) = bareness_of(decl.effect);
        let base = self.fn_params.len();
        for param in self.ast.params(decl.params) {
            let ty = self.resolve_type(param.ty, params_mode);
            self.fn_params.push((Some(param.name), ty));
        }
        let ret = decl
            .ret
            .map_or(TyId::ERROR, |ret| self.resolve_type(ret, ret_mode));
        let ty = self.function_type(function, base, (ret, decl.effect));
        self.fn_params.truncate(base);
        let clean = self.errors.len() == errors && ret != TyId::ERROR;
        self.errors.truncate(errors);
        self.warnings.truncate(warnings);
        if clean { ty } else { TyId::ERROR }
    }

    /// When the walk of the outermost block reaches the item `id`: if it
    /// is a top-level function, which is declared already, records the
    /// bindings above it, which its body sees, gives the item its slot
    /// and, for a `def`, adds the field, and returns `true`; and so for a
    /// definition made with a def-type that is none, which it places in its
    /// namespace, or reports, as such ([`Checker::place_definition`]).
    pub(super) fn reach_top_fn(&mut self, id: ItemId) -> bool {
        let Some(index) = self.top_fns.iter().position(|top| top.item == id) else {
            // A definition made with a def-type, or a definition's meta, that
            // is no top-level function: one whose `DT` is no def-type, or
            // that the rules of dotted fields reject in its namespace (see
            // [Def-types](crate::check::deftypes#def-types)).
            if let ItemKind::DefTypeDef { def, value } | ItemKind::DefMeta { def, value } =
                self.ast.item(id).kind
            {
                self.place_definition(id, def, value);
                return true;
            }
            return false;
        };
        let Some(top) = self.top_fns.get_mut(index) else {
            return false;
        };
        top.cut = Some(self.bindings.len());
        let (name, binding) = (top.name, top.binding);
        let Some(name) = name else {
            self.reach_dotted_def(index, id);
            return true;
        };
        self.check_declared_twice(id, name);
        let Some(&Binding { slot, ty, .. }) = self.bindings.get(binding) else {
            return true;
        };
        self.item_slots[id.index()] = slot;
        if let ItemKind::Field { path, .. } = self.ast.item(id).kind {
            let name_span = self
                .ast
                .path(path)
                .first()
                .map_or(self.ast.item(id).span, |segment| segment.span);
            let soft = match self.spreads {
                BlockSpreads::None => None,
                BlockSpreads::Some | BlockSpreads::Rejected => self
                    .block_field(name)
                    .filter(|&index| self.fields[index].spread.is_some()),
            };
            match soft {
                // The `def` overrides the soft field: the field keeps its
                // place and takes the function's slot.
                Some(index) => {
                    let field = &mut self.fields[index];
                    (field.ty, field.slot, field.name_span) = (ty, slot, name_span);
                    field.spread = None;
                }
                None => self.fields.push(FieldDecl {
                    name,
                    name_span,
                    ty,
                    slot,
                    open: None,
                    spread: None,
                    rejected: false,
                }),
            }
        }
        true
    }
}
