//! The parts of a workspace: its files and their declarations, directories,
//! nodes, targets, aliases and views.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Ast, DefTypeId, ItemId, TypeExprId};
use crate::check::{CheckOutput, Fileroot, KnownParams, Slot};
use crate::mentions::{self, Mentions, Schedule};
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;

use super::{Scan, checked_contexts, checked_methods};

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
    /// holds only the committed declarations of these (see [`check`](super::check)).
    pub spelled: Vec<Name>,
    /// Its declarations and what each of its items mentions, resolved
    /// against its view ([`FileScope`](super::FileScope)).
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
    /// source order (see [Types](crate::workspace::declared#types)).
    pub types: Vec<u32>,
    /// The types it imports by member or `*`, in source order.
    pub type_imports: Vec<TypeImport>,
    /// Its view of types: the declaration each name of a type names in it,
    /// by its number in [`Scan::type_store`].
    pub type_view: HashMap<Name, u32>,
    /// Its definitions made with a def-type whose directory declares no
    /// meta of their path and def-type, by their uses of their def-types,
    /// in source order: an error of each whose def-type declares a meta (see
    /// [Def-types](crate::workspace::declared#def-types)).
    pub unpaired: Vec<DefTypeId>,
    /// The positions among its items of the declarations the scan dropped,
    /// recovering from an error of the scan: a later definition of a field
    /// another file defined first, a meta declared twice or with no
    /// definition. Each is in no tree, as a shadowed declaration is, and
    /// still checked as written (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub dropped: Vec<usize>,
    /// The names it reads as `Error`, silently, recovering from an error of
    /// the scan: what an import that failed binds, a name an import clashes
    /// on, and the root of a conflict of its view or its view of types, in
    /// [`Name`] order (see [Scan errors](crate::workspace::scanning#scan-errors)).
    pub poisoned: Vec<Name>,
    /// Whether it imports names the scan cannot know, by a `*` import that
    /// failed, so that a name that names nothing in it is no error (see
    /// [Scan errors](crate::workspace::scanning#scan-errors)).
    pub blind: bool,
    /// Its `import` items the scan reported an error of, in source order,
    /// each with the module its path names, if it names one.
    pub failed_imports: Vec<(ItemId, Option<Target>)>,
    /// The modules its imports name, a directory or a file each, in source
    /// order, the implicit prelude's last: whose `def` methods it sees (see
    /// [Methods](crate::workspace::declared#methods)).
    pub method_modules: Vec<Target>,
    /// Its view of methods: the `def` methods of other files it sees, by
    /// their names (see [Methods](crate::workspace::declared#methods)).
    pub method_view: HashMap<Name, Vec<MethodRef>>,
    /// Its view of variants: the variants it sees, its own and other
    /// files', by the key of their enum and their name, each by its index
    /// in [`Scan::variants`], or, of a session's earlier input, its key
    /// alone (see [Enums](crate::workspace::declared#enums)).
    pub variant_view: HashMap<(Name, Name), Vec<SeenVariant>>,
}

/// A variant a file sees (see [Enums](crate::workspace::declared#enums)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeenVariant {
    /// Its key, its identity ([`names::variant_key`](crate::names::variant_key)).
    pub key: Name,
    /// Its declaration, by its index in [`Scan::variants`]; `None` for an
    /// earlier input's of a session, whose source is not this one.
    pub declared: Option<u32>,
}

/// What a session's input inherits of the earlier inputs' types: each type
/// with its declaration's number in the store, and each variant.
pub(super) type Inheriting<'a> = (
    &'a [(Name, u32)],
    &'a [InheritedVariant],
    &'a [InheritedContext],
);

/// A variant a session's input inherits from the earlier inputs: its enum's
/// key, its name and its own key (see [Enums](crate::workspace::declared#enums)).
pub(crate) type InheritedVariant = (Name, Name, Name);

/// A variant one of a scan's files declares (see [Enums](crate::workspace::declared#enums)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclaredVariant {
    /// Its file.
    pub file: u32,
    /// Its `enum` item.
    pub item: ItemId,
    /// Its name.
    pub name: Name,
    /// Where its name is written.
    pub span: Span,
    /// Its enum, as written, in its file's tree.
    pub of: TypeExprId,
    /// Its enum, by its key and its name, if what it writes is one.
    pub enum_of: Option<(Name, Name)>,
    /// Its key, its identity ([`names::variant_key`](crate::names::variant_key)).
    pub key: Name,
}

/// A context one of a scan's files declares (see [Context](crate::workspace::declared#context)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclaredContext {
    /// Its file.
    pub file: u32,
    /// Its position among the file's contexts ([`Ast::contexts`]).
    pub decl: u32,
    /// Its name.
    pub name: Name,
    /// Where its name is written.
    pub span: Span,
    /// Its declaration's key, its identity ([`names::context_key`](crate::names::context_key)).
    pub key: Name,
    /// Its default's binding, if it has a default.
    pub binding: Option<Name>,
}

/// A context a session's input inherits from the earlier inputs: its name,
/// its declaration's key and its default's binding, if it has a default
/// (see [Context](crate::workspace::declared#context)).
pub(crate) type InheritedContext = (Name, Name, Option<Name>);

/// The slot of each context's default's closure, once its file is checked,
/// by its declaration's key (see [Context](crate::workspace::declared#context)).
pub(crate) type CheckedContexts = HashMap<Name, Slot>;

/// What the checks of a workspace's files found that later files read:
/// their methods (see [Methods](crate::workspace::declared#methods)) and their contexts' defaults
/// (see [Context](crate::workspace::declared#context)).
#[derive(Clone, Debug, Default)]
pub(crate) struct ForLater {
    /// The methods.
    pub(crate) methods: CheckedMethods,
    /// The contexts' defaults.
    pub(crate) contexts: CheckedContexts,
}

impl ForLater {
    /// Adds what the check `output` of file `index` of `scan` found.
    pub(super) fn extend(&mut self, scan: &Scan, index: u32, output: &CheckOutput) {
        self.methods.extend(checked_methods(scan, index, output));
        self.contexts.extend(checked_contexts(scan, index, output));
    }
}

/// Where a read of a context that no `with` binds finds its default (see
/// [Context](crate::workspace::declared#context)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextDefault {
    /// It has none.
    None,
    /// The binding of its function, of the file being checked or of a
    /// session's earlier input.
    Binding(Name),
    /// The slot of its function, of another file of the workspace, checked
    /// and run before the file; `None` if that file failed its check.
    Foreign(Option<Slot>),
}

/// A context a file sees, as its checker is given it
/// ([`Context::context`](super::Context::context)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SeenContext {
    /// Its declaration's key, by which the type table has its type.
    pub(crate) key: Name,
    /// Where its default is.
    pub(crate) default: ContextDefault,
    /// Where its name is declared, its file and its span, if that is known.
    pub(crate) site: Option<(u32, Span)>,
}

/// A method of a file of a scan, by the file and its position among the
/// file's methods ([`Ast::methods`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MethodRef {
    /// Its file.
    pub file: u32,
    /// Its position among the file's methods.
    pub method: u32,
}

/// A method of a file, checked: where another file reads its closure, its
/// type, and what a call of it needs to know of its parameters (see
/// [Methods](crate::workspace::declared#methods)).
#[derive(Clone, Debug)]
pub(crate) struct CheckedMethod {
    /// The slot its closure is in once its file has run.
    pub(crate) slot: Slot,
    /// Its function's type.
    pub(crate) ty: TyId,
    /// What a call of it needs to know of its parameters.
    pub(crate) known: Option<Rc<KnownParams>>,
}

/// The checked methods of the files of a workspace, by file and position.
pub(crate) type CheckedMethods = HashMap<MethodRef, CheckedMethod>;

/// A method of another file that a file sees, checked, as its checker is
/// given it ([`Context::methods`](super::Context::methods)).
#[derive(Clone, Debug)]
pub(crate) struct ForeignMethod {
    /// Its file, the input its diagnostics' labels are in.
    pub(crate) file: u32,
    /// Where its name is declared, in its file.
    pub(crate) span: Span,
    /// What its check found.
    pub(crate) checked: CheckedMethod,
}

impl File {
    /// Whether `alias`, one of the file's, is imported by an import marked
    /// `#[shadowable]` (the implicit one of the prelude among them), which
    /// makes every leaf it brings shadowable (see the module documentation,
    /// "Shadowable declarations"); an import a session's input inherits is
    /// not, which only the input's declarations shadow
    /// ([`inherit_imports`](super::imports::inherit_imports)).
    #[must_use]
    pub fn marked(&self, alias: &Alias) -> bool {
        alias.item.is_some_and(|item| self.ast.is_shadowable(item))
    }

    /// The file at `path` in directory `dir`, parsed to `ast`, spelling
    /// `spelled`, with its declarations found and nothing else yet.
    pub(super) fn new(path: &str, dir: u32, ast: Ast, spelled: Vec<Name>) -> Self {
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
            method_modules: Vec::new(),
            method_view: HashMap::new(),
            variant_view: HashMap::new(),
        }
    }

    /// Drops the declarations at `positions` among its items (see
    /// [`File::dropped`]): its declarations found again with them shadowed,
    /// as well as those already are.
    pub(super) fn drop_items(&mut self, positions: &[usize]) {
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
    pub(super) fn poison(&mut self, name: Name) {
        if let Err(at) = self.poisoned.binary_search(&name) {
            self.poisoned.insert(at, name);
        }
    }
}

/// A `type` declaration of a file of a scan (see [Types](crate::workspace::declared#types)).
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
    /// The type it stands for, in its file's tree; `None` for an enum,
    /// which is a type of its own (see the checker's "Enums").
    pub ty: Option<TypeExprId>,
}

/// A type a file imports by member, `import NAME from PATH`, or by `*`
/// (see [Types](crate::workspace::declared#types)).
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
    /// (see [The standard library](crate::workspace#the-standard-library)).
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
    /// [`inherit_imports`](super::imports::inherit_imports)).
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
    pub(super) const fn chain(source: u32, node: u32) -> Self {
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
    /// under ([`ROOT`](super::ROOT) for a root, else its index in [`Scan::views`]) and
    /// the name.
    pub(super) merged: HashMap<(u32, Name), Merged>,
}
