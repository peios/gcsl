//! What of the standard files a step's code may reach, by name, so that a
//! host keys a step by the standard items it may run rather than by every
//! standard file (see [Reach](#reach)).
//!
//! # Reach
//!
//! A host keys a step by the code it runs: each file of its code, and of
//! the standard files only the *items*, the top-level items of their
//! outermost blocks, that its code may reach. An edit of a standard item
//! no step reaches keys no step again.
//!
//! An item *reaches* each item whose declared name appears as an
//! identifier in it, wherever a name of that spelling is declared: in a
//! file its code needs, or in any standard file. An identifier after a `.`
//! names a member, which reaches only each method of its name, as a field
//! call spells it: a module's members are reached through the module's
//! name before the `.`. A module an import binds, a name of the importing
//! file alone, is reached whole where an item of that file reads it as a
//! value; where it reads a member of it, `mod.x`, `x` is
//! reached by its name, as any declaration is, and so is each file of the
//! module's whose path names `x`. One before a `:` or a single `=` names a binding,
//! a parameter, a field or a named argument, which reaches only each
//! context of its name, as a `with` binds it. Its identifiers are its
//! tokens', an interpolation's included, from the end of the item before
//! it through its own end, so that its attributes are its own. An item
//! declares the first segment of the path it binds, or the name of the
//! type, enum, variant or context it declares, or the first segment of the
//! path of what it defines with a def-type; a method declares its
//! method's name, as a member, and a context its name. A name an item reaches also
//! reaches each item that rebinds it, by `:=`, whose value it may then
//! read; a name an import binds, each item the import's target holds; and
//! a name a `from` import binds under another, the items of the name it
//! binds. Reach is followed to a fixpoint from the item that holds the
//! step's own function.
//!
//! Some items are reached always: of every standard file, each
//! declaration of a type, an enum, a variant or a context, and each item
//! that declares no name a reference could spell, an expression, a spread
//! or a file's result; and of every file the code needs, each such item
//! too, since it runs as its file does.
//!
//! Reach by name ignores scope: a name a parameter or a local shadows still
//! reaches each item of its spelling. So it reaches more than the code can,
//! never less, and an edit of an item it reaches without need keys a step
//! again for nothing. A standard file that cannot be lexed is reached whole,
//! by its text, as is every standard file if the item holding the step's
//! function cannot be found.
//!
//! A standard item is keyed by its *identity*: its file's path and the text
//! of its tokens, a run of line breaks as one, so that a comment, or a
//! space, changes no key.

use std::collections::{HashMap, HashSet};

use sha2::{Digest as _, Sha256};

use crate::ast::{Ast, ImportForm, ItemKind, PathSegment};
use crate::lexer::{TokenKind, lex};
use crate::names::{Name, Names};
use crate::source::SourceFile;

use super::files::Target;

/// What reach knows of one top-level item of a file (see [Reach](#reach)).
#[derive(Clone, Debug)]
pub(crate) struct ItemReach {
    /// The offset its own span ends at: it holds every token after the end
    /// of the item before it through this.
    pub(crate) end: u32,
    /// The offset its first token begins at, but a line break.
    pub(crate) first: u32,
    /// The names it declares.
    pub(crate) declares: Vec<Name>,
    /// The names it rebinds, by `:=`.
    pub(crate) rebinds: Vec<Name>,
    /// The identifiers of its tokens that may refer to a declaration, each
    /// once: neither a member's name after a `.` nor a binding's before a
    /// `:` or an `=`.
    pub(crate) idents: Vec<Name>,
    /// The names of the members its tokens read or call, after a `.`, each
    /// once: what a field call of a method spells.
    pub(crate) members: Vec<Name>,
    /// The names its tokens bind, before a `:` or an `=`, each once: a
    /// parameter, a field, a named argument, or a context `with` binds.
    pub(crate) bindings: Vec<Name>,
    /// The names it reads as a value, not before a `.`, each once: a module
    /// so read is reached whole.
    pub(crate) bare: Vec<Name>,
    /// Each name before a `.` and the member's name after it, each pair
    /// once: a module's member, reached by its name.
    pub(crate) qualified: Vec<(Name, Name)>,
    /// The names of the methods it declares.
    pub(crate) methods: Vec<Name>,
    /// Whether what it declares is its file's alone, a `let`'s or a `fn`'s,
    /// which no other file's code can name: a `fn` method's too.
    pub(crate) private: bool,
    /// The names of the contexts it declares.
    pub(crate) contexts: Vec<Name>,
    /// Whether it is reached whenever its file runs: an item that declares
    /// no name a reference could spell.
    pub(crate) ordered: bool,
    /// Whether it declares a type, an enum, a variant or a context.
    pub(crate) declaration: bool,
    /// Its identity: its file's path and its tokens' text.
    pub(crate) identity: [u8; 32],
}

/// A call of a method, as its check chose the method: the offset of the
/// call's callee, and the input and offset of the method's declaration, if
/// known.
pub(crate) type MethodEdge = (u32, Option<(u32, u32)>);

/// What reach knows of a file (see [Reach](#reach)).
#[derive(Clone, Debug, Default)]
pub(crate) struct FileReach {
    /// Its top-level items, in source order.
    pub(crate) items: Vec<ItemReach>,
    /// The names of the directories of its path and its stem, by which a
    /// module of a directory names it as a member.
    pub(crate) components: Vec<Name>,
    /// Whether it could be lexed: else it is reached whole, by `whole`.
    pub(crate) analysable: bool,
    /// The digest of its whole text.
    pub(crate) whole: [u8; 32],
    /// Each name a `from` import binds under another, with the name it
    /// binds.
    pub(crate) renamed: Vec<(Name, Name)>,
    /// Each call of a method in it, once it is checked, as its check chose
    /// the method: the offset of the call's callee, and the input and offset
    /// of the method's declaration, if known. `None` for a file not checked,
    /// whose calls reach every method of the name they spell.
    pub(crate) method_edges: Option<Vec<MethodEdge>>,
}

impl FileReach {
    /// The item that holds the offset `at`, if one does.
    pub(crate) fn item_at(&self, at: u32) -> Option<usize> {
        let mut start = 0;
        for (index, item) in self.items.iter().enumerate() {
            if at >= start && at <= item.end {
                return Some(index);
            }
            start = item.end;
        }
        None
    }
}

/// What reach knows of the file `source`, of the syntax tree `ast`, at
/// `path`, its names interned into `names`.
pub(crate) fn index(source: &SourceFile, ast: &Ast, names: &mut Names, path: &str) -> FileReach {
    let whole: [u8; 32] = Sha256::digest(source.text().as_bytes()).into();
    let lexed = lex(source, names);
    if !lexed.diagnostics.is_empty() {
        return FileReach {
            whole,
            ..FileReach::default()
        };
    }
    let mut items = Vec::new();
    let mut renamed = Vec::new();
    let mut tokens = lexed.tokens.iter().peekable();
    for &id in ast.items(ast.root()) {
        let item = ast.item(id);
        let end = item.span.end;
        let mut held = Vec::new();
        while let Some(token) = tokens.next_if(|token| token.span.end <= end) {
            held.push(token);
        }
        let (kinds, begins, identity) = tokens_of(&held, source.text(), path);
        let (idents, members, bindings) = classify(&kinds);
        let (bare, qualified) = qualify(&kinds);
        let declared = declared_by(ast, id, &mut renamed);
        let (methods, private) = methods_of(ast, id, &declared);
        items.push(ItemReach {
            end,
            first: begins.unwrap_or(item.span.start),
            declares: declared.declares,
            rebinds: declared.rebinds,
            idents,
            members,
            bindings,
            bare,
            qualified,
            methods,
            private,
            contexts: declared.contexts,
            ordered: declared.ordered,
            declaration: declared.declaration,
            identity,
        });
    }
    let components = path
        .trim_end_matches(".gcsl")
        .split('/')
        .filter(|part| !part.is_empty() && !part.starts_with('<'))
        .map(|part| names.intern(part))
        .collect();
    FileReach {
        items,
        components,
        analysable: true,
        whole,
        renamed,
        method_edges: None,
    }
}

/// Of an item's tokens `held`, of the file `text` at `path`, their kinds
/// but line breaks', where the first begins, and the item's identity: the
/// path and the tokens' text, a run of line breaks as one.
fn tokens_of(
    held: &[&crate::lexer::Token],
    text: &str,
    path: &str,
) -> (Vec<TokenKind>, Option<u32>, [u8; 32]) {
    let mut hasher = Sha256::new();
    hasher.update(b"gcsl-item-1\0");
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    let mut broken = false;
    let mut begins = None;
    let mut kinds = Vec::new();
    for token in held {
        match token.kind {
            TokenKind::Newline => {
                if !broken {
                    hasher.update(b"\n");
                }
                broken = true;
                continue;
            }
            TokenKind::Eof => continue,
            _ => {}
        }
        kinds.push(token.kind);
        broken = false;
        begins.get_or_insert(token.span.start);
        let piece = text
            .get(token.span.start as usize..token.span.end as usize)
            .unwrap_or("");
        hasher.update(piece.as_bytes());
        hasher.update(b"\0");
    }
    (kinds, begins, hasher.finalize().into())
}

/// What an item declares (see [Reach](#reach)).
struct Declared {
    /// The names it declares.
    declares: Vec<Name>,
    /// The names it rebinds.
    rebinds: Vec<Name>,
    /// The names of the contexts it declares.
    contexts: Vec<Name>,
    /// Whether it declares no name a reference could spell.
    ordered: bool,
    /// Whether it declares a type, an enum, a variant or a context.
    declaration: bool,
}

/// What the item `id` of `ast` declares, each name a `from` import of it
/// binds under another added to `renamed`, with the name it binds.
fn declared_by(ast: &Ast, id: crate::ast::ItemId, renamed: &mut Vec<(Name, Name)>) -> Declared {
    let item = ast.item(id);
    let first = |range| ast.path(range).first().and_then(PathSegment::name);
    let (mut declares, rebinds, ordered, mut declaration) = match item.kind {
        ItemKind::Let { path, .. } | ItemKind::Field { path, .. } => {
            (first(path).into_iter().collect(), Vec::new(), false, false)
        }
        ItemKind::Rebind { path, .. } => {
            (Vec::new(), first(path).into_iter().collect(), true, false)
        }
        ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. } => {
            let declared: Vec<Name> = first(ast.def_type(def).path).into_iter().collect();
            let ordered = declared.is_empty();
            (declared, Vec::new(), ordered, false)
        }
        ItemKind::Type { name, .. }
        | ItemKind::Enum { name, .. }
        | ItemKind::Context { name, .. }
        | ItemKind::Variant { name, .. } => (vec![name], Vec::new(), false, true),
        ItemKind::Import(import) => {
            if let ImportForm::Members(members) = ast.import_decl(import).form {
                for member in ast.import_members(members) {
                    if let crate::ast::ImportMember::Named {
                        name,
                        alias: Some(alias),
                    } = *member
                    {
                        renamed.push((alias.name, name.name));
                    }
                }
            }
            (Vec::new(), Vec::new(), false, false)
        }
        ItemKind::Spread(_)
        | ItemKind::Result(_)
        | ItemKind::Expr(_)
        | ItemKind::Break(_)
        | ItemKind::Continue(_)
        | ItemKind::Return(_)
        | ItemKind::Error => (Vec::new(), Vec::new(), true, false),
    };
    // A context with a default is the `let` of its default under a binding
    // no source spells: it declares the context's name too.
    let mut contexts = Vec::new();
    for context in ast.contexts().iter().filter(|context| context.item == id) {
        declares.push(context.name);
        contexts.push(context.name);
        declaration = true;
    }
    if let ItemKind::Context { name, .. } = item.kind {
        contexts.push(name);
    }
    Declared {
        declares,
        rebinds,
        contexts,
        ordered,
        declaration,
    }
}

/// The names of the methods the item `id` of `ast` declares, under a
/// binding of their own, which a field call reaches by the method's name,
/// and whether what it declares, `declared`, is its file's alone: a `let`'s
/// or a `fn`'s, or a `fn` method's, but not a context's default, which is
/// the `let` of a binding.
fn methods_of(ast: &Ast, id: crate::ast::ItemId, declared: &Declared) -> (Vec<Name>, bool) {
    let declared_methods: Vec<_> = ast
        .methods()
        .iter()
        .filter(|method| method.item == id)
        .collect();
    let methods = declared_methods.iter().map(|method| method.name).collect();
    let private = if declared_methods.is_empty() {
        matches!(ast.item(id).kind, ItemKind::Let { .. }) && declared.contexts.is_empty()
    } else {
        declared_methods.iter().all(|method| !method.public)
    };
    (methods, private)
}

/// The identifiers of the tokens `kinds`, each once, in three: those that
/// may refer to a declaration; members' names, after a `.`, which a field
/// call of a method spells; and bindings' names, before a `:`, as a
/// parameter's or a field's type is written, or before a single `=`, as a
/// field, a named argument or a context a `with` binds is.
fn classify(kinds: &[TokenKind]) -> (Vec<Name>, Vec<Name>, Vec<Name>) {
    let mut sets: [(Vec<Name>, HashSet<Name>); 3] = Default::default();
    for (at, kind) in kinds.iter().enumerate() {
        let TokenKind::Ident(name) = *kind else {
            continue;
        };
        let before = at.checked_sub(1).and_then(|before| kinds.get(before));
        let after = kinds.get(at + 1);
        let which = if matches!(before, Some(TokenKind::Dot)) {
            1
        } else if matches!(after, Some(TokenKind::Colon | TokenKind::Eq)) {
            2
        } else {
            0
        };
        let (list, seen) = &mut sets[which];
        if seen.insert(name) {
            list.push(name);
        }
    }
    let [(idents, _), (members, _), (bindings, _)] = sets;
    (idents, members, bindings)
}

/// Of the tokens `kinds`, the identifiers read as a value, not before a
/// `.`, and each identifier before a `.` with the member's name after it,
/// each once: how an item reads a module.
fn qualify(kinds: &[TokenKind]) -> (Vec<Name>, Vec<(Name, Name)>) {
    let mut bare = Vec::new();
    let mut qualified = Vec::new();
    let (mut seen_bare, mut seen_pair) = (HashSet::new(), HashSet::new());
    for (at, kind) in kinds.iter().enumerate() {
        let TokenKind::Ident(name) = *kind else {
            continue;
        };
        if matches!(
            at.checked_sub(1).and_then(|before| kinds.get(before)),
            Some(TokenKind::Dot)
        ) {
            continue;
        }
        match (kinds.get(at + 1), kinds.get(at + 2)) {
            (Some(TokenKind::Dot), Some(&TokenKind::Ident(member))) => {
                if seen_pair.insert((name, member)) {
                    qualified.push((name, member));
                }
            }
            _ => {
                if seen_bare.insert(name) {
                    bare.push(name);
                }
            }
        }
    }
    (bare, qualified)
}

/// An item, by its file's input number and its index among the file's.
pub(crate) type At = (u32, usize);

/// Where each name is declared, rebound, imported and renamed among some
/// files (see [Reach](#reach)).
#[derive(Debug, Default)]
pub(crate) struct NameIndex {
    /// The items declaring each name.
    pub(crate) declared: HashMap<Name, Vec<At>>,
    /// The items rebinding each name.
    pub(crate) rebound: HashMap<Name, Vec<At>>,
    /// The files each name an import binds holds, by the importing file:
    /// an import binds a name of its own file alone.
    pub(crate) imported: HashMap<(u32, Name), Vec<u32>>,
    /// The names each name a `from` import binds under another binds.
    pub(crate) renamed: HashMap<Name, Vec<Name>>,
    /// The items declaring a method of each name.
    pub(crate) methods: HashMap<Name, Vec<At>>,
    /// The items declaring a context of each name.
    pub(crate) contexts: HashMap<Name, Vec<At>>,
}

/// Whether the item `found` is one an item of file `file` can name: its own
/// file's, or not its file's alone.
fn visible(files: &[FileReach], file: u32, found: At) -> bool {
    found.0 == file
        || !files
            .get(found.0 as usize)
            .and_then(|f| f.items.get(found.1))
            .is_some_and(|item| item.private)
}

/// The methods the item `(file, item)` calls, among `files`: those its
/// calls chose, by its receivers' types, if its file was checked and each
/// was found; else each method of each member's name it can name.
fn methods_called(files: &[FileReach], index: &NameIndex, (file, item): At) -> Vec<At> {
    let Some(f) = files.get(file as usize) else {
        return Vec::new();
    };
    let Some(at) = f.items.get(item) else {
        return Vec::new();
    };
    let start = item
        .checked_sub(1)
        .map_or(0, |before| f.items.get(before).map_or(0, |item| item.end));
    let chosen: Option<Vec<At>> = f.method_edges.as_ref().and_then(|edges| {
        edges
            .iter()
            .filter(|(call, _)| *call >= start && *call <= at.end)
            .map(|(_, declared)| {
                let (target, offset) = (*declared)?;
                let found = files.get(target as usize)?.item_at(offset)?;
                Some((target, found))
            })
            .collect()
    });
    chosen.unwrap_or_else(|| {
        at.members
            .iter()
            .flat_map(|name| index.methods.get(name).into_iter().flatten().copied())
            .filter(|&found| visible(files, file, found))
            .collect()
    })
}

/// The items the item `at` of file `file` reaches of the modules it reads,
/// among `files`: each module it reads as a value, whole, and each file of a
/// module named as the member it reads of it; each member's name added to
/// `names`, to be reached as any declaration is, which may be one the
/// module imports.
fn modules_read(
    files: &[FileReach],
    index: &NameIndex,
    (file, at): (u32, &ItemReach),
    names: &mut Vec<Name>,
) -> Vec<At> {
    let mut wholes: Vec<u32> = Vec::new();
    for name in &at.bare {
        wholes.extend(
            index
                .imported
                .get(&(file, *name))
                .into_iter()
                .flatten()
                .copied(),
        );
    }
    for (module, member) in &at.qualified {
        let Some(targets) = index.imported.get(&(file, *module)) else {
            continue;
        };
        names.push(*member);
        wholes.extend(targets.iter().copied().filter(|&target| {
            files
                .get(target as usize)
                .is_some_and(|f| f.components.contains(member))
        }));
    }
    wholes
        .into_iter()
        .flat_map(|whole| {
            let count = files.get(whole as usize).map_or(0, |f| f.items.len());
            (0..count).map(move |item| (whole, item))
        })
        .collect()
}

/// The items reached from `seeds` among `files`, by `index`, to a fixpoint.
pub(crate) fn reach(files: &[FileReach], index: &NameIndex, seeds: &[At]) -> HashSet<At> {
    let mut reached: HashSet<At> = seeds.iter().copied().collect();
    let mut work: Vec<At> = seeds.to_vec();
    // By the file reading it, which decides what of its file alone it sees.
    let mut names_seen: HashSet<(u32, Name)> = HashSet::new();
    while let Some((file, item)) = work.pop() {
        let Some(at) = files.get(file as usize).and_then(|f| f.items.get(item)) else {
            continue;
        };
        let mut next = methods_called(files, index, (file, item));
        for name in &at.bindings {
            next.extend(index.contexts.get(name).into_iter().flatten().copied());
        }
        let mut names: Vec<Name> = at.idents.clone();
        next.extend(modules_read(files, index, (file, at), &mut names));
        let mut name_at = 0;
        while let Some(&name) = names.get(name_at) {
            name_at += 1;
            if !names_seen.insert((file, name)) {
                continue;
            }
            names.extend(index.renamed.get(&name).into_iter().flatten().copied());
            next.extend(
                index
                    .declared
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&found| visible(files, file, found)),
            );
            next.extend(index.rebound.get(&name).into_iter().flatten().copied());
        }
        for found in next {
            if reached.insert(found) {
                work.push(found);
            }
        }
    }
    reached
}

/// What reach has found so far in a workspace, kept so that each item's
/// reach is found once.
#[derive(Debug, Default)]
pub(crate) struct Reached {
    /// The name index of the files each file's code needs, by its input
    /// number.
    indices: HashMap<u32, std::rc::Rc<NameIndex>>,
    /// The digest of the standard items each item reaches, by the item.
    digests: HashMap<At, [u8; 32]>,
}

impl super::Workspace {
    /// Whether the file `file` is a standard file.
    #[must_use]
    pub fn is_standard_file(&self, file: u32) -> bool {
        file < self.scan.standard
    }

    /// The digest of the standard items the code of input `input` at its
    /// offset `at` may reach (see [Reach](#reach)): `None` if no item holds
    /// it, or its file cannot be lexed, when every standard file is the
    /// code's, whole.
    #[must_use]
    pub fn standard_reach(&self, input: u32, at: u32) -> Option<[u8; 32]> {
        let scan = &self.scan;
        let file = scan.reach.get(input as usize)?;
        if !file.analysable {
            return None;
        }
        let item = file.item_at(at)?;
        if let Some(digest) = scan.reached.borrow().digests.get(&(input, item)) {
            return Some(*digest);
        }
        let reached = self.reached_from((input, item));
        let mut identities: Vec<[u8; 32]> = reached
            .iter()
            .filter(|(f, _)| *f < scan.standard)
            .filter_map(|&(f, i)| Some(scan.reach.get(f as usize)?.items.get(i)?.identity))
            .collect();
        identities.sort_unstable();
        let mut hasher = Sha256::new();
        hasher.update(b"gcsl-standard-reach-1\0");
        for identity in &identities {
            hasher.update(identity);
        }
        // A standard file that cannot be lexed is reached whole.
        for f in 0..scan.standard {
            if let Some(file) = scan.reach.get(f as usize)
                && !file.analysable
            {
                hasher.update(b"whole\0");
                hasher.update(file.whole);
            }
        }
        let digest: [u8; 32] = hasher.finalize().into();
        scan.reached
            .borrow_mut()
            .digests
            .insert((input, item), digest);
        Some(digest)
    }

    /// What keys the function whose body begins at the offset `at` of the
    /// standard file `input`, rather than the file's whole text: what it may
    /// reach ([`Workspace::standard_reach`]), and where its body begins in
    /// the item that holds it, which no edit of another item moves. `None`
    /// for a file that is no standard file, or a function reach cannot
    /// place, which the file's whole text keys.
    #[must_use]
    pub fn function_identity(&self, input: u32, at: u32) -> Option<[u8; 32]> {
        if !self.is_standard_file(input) {
            return None;
        }
        let reach = self.standard_reach(input, at)?;
        let file = self.scan.reach.get(input as usize)?;
        let start = file.items.get(file.item_at(at)?)?.first;
        let mut hasher = Sha256::new();
        hasher.update(b"gcsl-function-1\0");
        hasher.update(reach);
        hasher.update((at - start.min(at)).to_le_bytes());
        Some(hasher.finalize().into())
    }

    /// Where the function whose body begins at the offset `at` of the
    /// standard file `input` is, as no edit of another item changes it: the
    /// first name the item holding it declares, else its number among the
    /// file's items, and where its body begins in the item. `None` for a
    /// file that is no standard file, or a function it cannot place.
    #[must_use]
    pub fn function_place(&self, input: u32, at: u32) -> Option<String> {
        if !self.is_standard_file(input) {
            return None;
        }
        let file = self.scan.reach.get(input as usize)?;
        let index = file.item_at(at)?;
        let item = file.items.get(index)?;
        let start = item.first;
        let named = item
            .declares
            .first()
            .map(|&name| self.scan.names.get(name).to_owned());
        Some(format!(
            "{}+{}",
            named.unwrap_or_else(|| format!("#{index}")),
            at - start.min(at)
        ))
    }

    /// [`Workspace::standard_reach`] of the code of `closure`.
    #[must_use]
    pub fn closure_reach(&self, closure: &crate::value::Closure) -> Option<[u8; 32]> {
        let code = &closure.code;
        let body = code.find_function(closure.function)?.body;
        self.standard_reach(code.input(), code.get(body).span.start)
    }

    /// [`Workspace::standard_reach`] of the code of the node `id` of
    /// `graph`: `None` too for a node that runs no code of its own.
    #[must_use]
    pub fn node_standard_reach(
        &self,
        graph: &crate::eval::graph::Graph,
        id: crate::eval::graph::NodeId,
    ) -> Option<[u8; 32]> {
        let closure = graph.node(id)?.closure()?;
        self.closure_reach(closure)
    }

    /// How many of each standard file's items the code of input `input` at
    /// its offset `at` reaches, and how many it has, by the file's path:
    /// for measuring reach.
    #[doc(hidden)]
    #[must_use]
    pub fn standard_reached(&self, input: u32, at: u32) -> Option<Vec<(String, usize, usize)>> {
        let scan = &self.scan;
        let file = scan.reach.get(input as usize)?;
        let item = file.item_at(at)?;
        let reached = self.reached_from((input, item));
        Some(
            (0..scan.standard)
                .map(|f| {
                    let total = scan.reach.get(f as usize).map_or(0, |f| f.items.len());
                    let count = reached.iter().filter(|(at, _)| *at == f).count();
                    let path = scan
                        .files
                        .get(f as usize)
                        .map_or_else(String::new, |f| f.path.clone());
                    (path, count, total)
                })
                .collect(),
        )
    }

    /// The items reached from the item `item` (see [Reach](#reach)): from
    /// it, from every item of the files its code needs that declares no
    /// name, and from each declaration of a type, an enum, a variant or a
    /// context of every standard file.
    fn reached_from(&self, item: At) -> HashSet<At> {
        let scan = &self.scan;
        let needed = scan.needed(item.0);
        let index = self.name_index(item.0, &needed);
        let mut seeds = vec![item];
        for &f in &needed {
            let standard = f < scan.standard;
            let items = scan.reach.get(f as usize).map_or(&[][..], |f| &f.items[..]);
            for (i, at) in items.iter().enumerate() {
                if at.ordered || (standard && at.declaration) {
                    seeds.push((f, i));
                }
            }
        }
        reach(&scan.reach, &index, &seeds)
    }

    /// The standard items the code of input `input` at its offset `at`
    /// reaches, each by its file's path and its number among the file's
    /// items: for tests of reach.
    #[cfg(any(test, feature = "coverage"))]
    #[must_use]
    pub fn standard_items_reached(&self, input: u32, at: u32) -> Option<HashSet<(String, usize)>> {
        let scan = &self.scan;
        let item = scan.reach.get(input as usize)?.item_at(at)?;
        Some(
            self.reached_from((input, item))
                .into_iter()
                .filter(|(f, _)| *f < scan.standard)
                .map(|(f, i)| (scan.files[f as usize].path.clone(), i))
                .collect(),
        )
    }

    /// [`Workspace::standard_items_reached`] of the code of `closure`: for
    /// tests of reach.
    #[cfg(any(test, feature = "coverage"))]
    #[must_use]
    pub fn closure_items_reached(
        &self,
        closure: &crate::value::Closure,
    ) -> Option<HashSet<(String, usize)>> {
        let code = &closure.code;
        let body = code.find_function(closure.function)?.body;
        self.standard_items_reached(code.input(), code.get(body).span.start)
    }

    /// The standard items `used` used, each by its file's path and its
    /// number among the file's items, and what of the standard files it
    /// used that no item could be found for: for tests of reach (see
    /// [`crate::coverage`]).
    #[cfg(any(test, feature = "coverage"))]
    #[must_use]
    pub fn standard_items_used(
        &self,
        used: &[crate::coverage::Used],
    ) -> (HashSet<(String, usize)>, Vec<crate::coverage::Used>) {
        let mut items = HashSet::new();
        let mut unplaced = Vec::new();
        for &one in used {
            let Some(found) = self.standard_items_of(one) else {
                continue;
            };
            if found.is_empty() {
                unplaced.push(one);
            }
            for (f, i) in found {
                items.insert((self.scan.files[f as usize].path.clone(), i));
            }
        }
        (items, unplaced)
    }

    /// The standard items a run's use `one` used: `None` if it used none of
    /// a standard file's, and none if no item could be found for it.
    #[cfg(any(test, feature = "coverage"))]
    fn standard_items_of(&self, one: crate::coverage::Used) -> Option<Vec<At>> {
        use crate::coverage::Used;
        let scan = &self.scan;
        let holding = |file: u32, holds: &dyn Fn(&ItemReach) -> bool| -> Vec<At> {
            scan.reach.get(file as usize).map_or_else(Vec::new, |f| {
                f.items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| holds(item))
                    .map(|(i, _)| (file, i))
                    .collect()
            })
        };
        Some(match one {
            Used::Function { input, at } => {
                if input >= scan.standard {
                    return None;
                }
                let found = scan.reach.get(input as usize).and_then(|f| f.item_at(at));
                found.map(|i| (input, i)).into_iter().collect()
            }
            Used::Context { key } => {
                let declared: Vec<_> = scan.contexts.iter().filter(|c| c.key == key).collect();
                // One of no standard file's is the workspace's own.
                if !declared.is_empty() && declared.iter().all(|c| c.file >= scan.standard) {
                    return None;
                }
                declared
                    .into_iter()
                    .filter(|c| c.file < scan.standard)
                    .flat_map(|c| holding(c.file, &|item| item.contexts.contains(&c.name)))
                    .collect()
            }
            Used::Top { input, slot } => {
                if let Some(node) = scan.nodes.get(slot as usize) {
                    // A cell: of the files with a leaf under it, the items
                    // declaring its root's name.
                    let mut root = slot;
                    while let Some(parent) = scan.nodes.get(root as usize).and_then(|n| n.parent) {
                        root = parent;
                    }
                    let name = scan.nodes[root as usize].name;
                    let files: Vec<u32> = node
                        .contributors
                        .iter()
                        .copied()
                        .filter(|&f| f < scan.standard)
                        .collect();
                    if files.is_empty() {
                        return None;
                    }
                    files
                        .into_iter()
                        .flat_map(|f| holding(f, &|item| item.declares.contains(&name)))
                        .collect()
                } else {
                    // A slot of the reading code's own file.
                    if input >= scan.standard {
                        return None;
                    }
                    let Some(&name) = self.slot_names.get(slot as usize) else {
                        return Some(Vec::new());
                    };
                    holding(input, &|item| item.declares.contains(&name))
                }
            }
        })
    }

    /// The name index of `needed`, the files the code of input `input`
    /// needs, found once.
    fn name_index(&self, input: u32, needed: &[u32]) -> std::rc::Rc<NameIndex> {
        let scan = &self.scan;
        if let Some(index) = scan.reached.borrow().indices.get(&input) {
            return std::rc::Rc::clone(index);
        }
        let mut index = NameIndex::default();
        for &f in needed {
            let Some(file) = scan.reach.get(f as usize) else {
                continue;
            };
            for (i, item) in file.items.iter().enumerate() {
                for &name in &item.declares {
                    index.declared.entry(name).or_default().push((f, i));
                }
                for &name in &item.rebinds {
                    index.rebound.entry(name).or_default().push((f, i));
                }
                for &name in &item.methods {
                    index.methods.entry(name).or_default().push((f, i));
                }
                for &name in &item.contexts {
                    index.contexts.entry(name).or_default().push((f, i));
                }
            }
            for &(alias, name) in &file.renamed {
                index.renamed.entry(alias).or_default().push(name);
            }
            for alias in scan
                .files
                .get(f as usize)
                .map_or(&[][..], |f| &f.aliases[..])
            {
                // A declaration, by the first name of its path, which is
                // what its item declares; anything wider, by its files.
                let leaf = match alias.target {
                    Target::Node(node) => scan
                        .nodes
                        .get(node as usize)
                        .filter(|at| at.leaf.is_some() && at.children.is_empty())
                        .map(|_| node),
                    Target::Through { leaf, .. } => Some(leaf),
                    _ => None,
                };
                match leaf {
                    Some(leaf) => {
                        let mut root = leaf;
                        while let Some(parent) =
                            scan.nodes.get(root as usize).and_then(|at| at.parent)
                        {
                            root = parent;
                        }
                        if let Some(at) = scan.nodes.get(root as usize)
                            && at.name != alias.name
                        {
                            index.renamed.entry(alias.name).or_default().push(at.name);
                        }
                    }
                    None => index
                        .imported
                        .entry((f, alias.name))
                        .or_default()
                        .extend(scan.target_files(alias.target)),
                }
            }
        }
        let index = std::rc::Rc::new(index);
        scan.reached
            .borrow_mut()
            .indices
            .insert(input, std::rc::Rc::clone(&index));
        index
    }
}
