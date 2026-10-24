//! A file's symbols, and the workspace's (see the documentation of
//! [`crate::index`], "Symbols").

use crate::ast::{Ast, ExprId, ExprKind, ImportForm, ImportMember, ItemId, ItemKind};
use crate::source::Span;
use crate::types::TyId;

use super::{Index, Symbol, known, path_span, unparen};

/// The longest a symbol's detail is, in characters: a longer one is cut
/// short with `…`.
const DETAIL: usize = 60;

/// How deep a file's symbols go: its top-level declarations, and the fields
/// of record blocks written as their values, and theirs.
const DEPTH: usize = 3;

/// What a [`DocumentSymbol`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// A field: top-level, or of a record block.
    Field,
    /// A `let`.
    Variable,
    /// A `fn` or a `def`.
    Function,
    /// A method, `fn (x: T) name(...)` or `def (x: T) name(...)`.
    Method,
    /// A `type` item.
    Type,
    /// The file's imports, and each of them.
    Module,
    /// A definition, `def package x.y`, or a meta, `defmeta package x.y`.
    Definition,
}

/// A symbol of a file, as [`Index::document_symbols`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSymbol {
    /// Its name: a field's path, `a.b`; a definition's, `package x.y`.
    pub name: String,
    /// Its type, on one line, cut short.
    pub detail: Option<String>,
    /// What it is.
    pub kind: SymbolKind,
    /// The whole of what declares it.
    pub span: Span,
    /// Its name as written.
    pub selection: Span,
    /// The fields of the record block written as its value.
    pub children: Vec<DocumentSymbol>,
}

/// A symbol of a file of the workspace, as [`Index::workspace_symbols`]
/// finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundSymbol {
    /// Its file's input number.
    pub file: u32,
    /// Its name, as [`DocumentSymbol::name`].
    pub name: String,
    /// What it is.
    pub kind: SymbolKind,
    /// Its name as written.
    pub span: Span,
    /// The name of the symbol it is a field of, if it is one.
    pub container: Option<String>,
}

/// A symbol of the workspace, with its name lowered for matching.
#[derive(Debug)]
pub(super) struct Flat {
    /// The symbol.
    found: FoundSymbol,
    /// Its name in lower case.
    lower: String,
}

impl Index {
    /// The symbols of the file of input number `file`, in source order
    /// (see the module documentation, "Symbols"): its top-level fields,
    /// `let`s, `fn`s, `def`s, definitions, metas and `type` items, each
    /// field whose value is written as a record block with that block's
    /// fields as children, three levels at most, and its imports as one
    /// group, `imports`.
    #[must_use]
    pub fn document_symbols(&self, file: u32) -> Vec<DocumentSymbol> {
        let Some(ast) = self.scan.files.get(file as usize).map(|f| &f.ast) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut imports = Vec::new();
        for &item in ast.items(ast.root()) {
            if matches!(ast.item(item).kind, ItemKind::Import(_)) {
                imports.extend(self.import_symbol(ast, item));
            } else {
                found.extend(self.item_symbol(file, ast, item));
            }
        }
        if let (Some(first), Some(last)) = (imports.first(), imports.last()) {
            let span = first.span.join(last.span);
            let selection = first.selection;
            let at = found.partition_point(|symbol| symbol.span.start < span.start);
            let group = DocumentSymbol {
                name: "imports".to_owned(),
                detail: None,
                kind: SymbolKind::Module,
                span,
                selection,
                children: imports,
            };
            found.insert(at, group);
        }
        found
    }

    /// The symbol of the top-level item `item` of file `file`, whose tree
    /// is `ast`, if it declares one.
    fn item_symbol(&self, file: u32, ast: &Ast, item: ItemId) -> Option<DocumentSymbol> {
        let span = ast.item(item).span;
        match ast.item(item).kind {
            ItemKind::Field { .. } => self.field_symbol(file, ast, item, 1),
            ItemKind::Let {
                path,
                value,
                function,
                ..
            } => {
                let [first] = ast.path(path) else {
                    return None;
                };
                // A method by its name, not its binding's.
                if let Some(method) = ast.method(item) {
                    let ty = self.expr_type(file, unparen(ast, value));
                    return Some(DocumentSymbol {
                        name: self.scan.names.get(method.name).to_owned(),
                        detail: ty.and_then(|ty| self.short(ty)),
                        kind: SymbolKind::Method,
                        span,
                        selection: method.name_span,
                        children: Vec::new(),
                    });
                }
                let name = first.name()?;
                let function =
                    function || matches!(ast.get(unparen(ast, value)).kind, ExprKind::Fn(_));
                let ty = if function {
                    self.expr_type(file, unparen(ast, value))
                } else {
                    self.local_type(file, first.span)
                        .or_else(|| self.expr_type(file, value))
                };
                Some(DocumentSymbol {
                    name: self.scan.names.get(name).to_owned(),
                    detail: ty.and_then(|ty| self.short(ty)),
                    kind: if function {
                        SymbolKind::Function
                    } else {
                        SymbolKind::Variable
                    },
                    span,
                    selection: first.span,
                    children: Vec::new(),
                })
            }
            ItemKind::DefTypeDef { def, .. } | ItemKind::DefMeta { def, .. } => {
                let used = ast.def_type(def);
                let segments = ast.path(used.path);
                let selection = path_span(segments)?;
                let dt = match ast.get(used.dt).kind {
                    ExprKind::Name(name) => self.scan.names.get(name),
                    _ => "",
                };
                let meta = matches!(ast.item(item).kind, ItemKind::DefMeta { .. });
                // A meta's path is one occurrence; a definition's last
                // segment is its node's.
                let at = match segments.last() {
                    Some(last) if !meta => last.span,
                    _ => selection,
                };
                let path = self.show_segments(segments);
                Some(DocumentSymbol {
                    name: if meta {
                        format!("defmeta {dt} {path}")
                    } else {
                        format!("{dt} {path}")
                    },
                    detail: self.node_type(file, at).and_then(|ty| self.short(ty)),
                    kind: SymbolKind::Definition,
                    span,
                    selection,
                    children: Vec::new(),
                })
            }
            ItemKind::Type {
                name, name_span, ..
            } => {
                let expansion = self
                    .files
                    .get(file as usize)
                    .and_then(|f| f.declared.iter().find(|&&(at, _)| at == item))
                    .map(|&(_, ty)| ty)
                    .and_then(known);
                Some(DocumentSymbol {
                    name: self.scan.names.get(name).to_owned(),
                    detail: expansion.and_then(|ty| self.short(ty)),
                    kind: SymbolKind::Type,
                    span,
                    selection: name_span,
                    children: Vec::new(),
                })
            }
            _ => None,
        }
    }

    /// The symbol of the field item `item` of file `file`, whose tree is
    /// `ast`, at `depth` (the top level's is 1): its children the fields of
    /// the record block its value is written as, if it is one.
    fn field_symbol(
        &self,
        file: u32,
        ast: &Ast,
        item: ItemId,
        depth: usize,
    ) -> Option<DocumentSymbol> {
        let ItemKind::Field {
            path,
            value,
            function,
            ..
        } = ast.item(item).kind
        else {
            return None;
        };
        let segments = ast.path(path);
        let selection = path_span(segments)?;
        // A top-level field's type is its node's; a record block's field's
        // its local's.
        let declared = match segments {
            _ if depth == 1 => segments
                .last()
                .and_then(|last| self.node_type(file, last.span)),
            [first] => self.local_type(file, first.span),
            _ => None,
        };
        let ty = declared.or_else(|| self.expr_type(file, value));
        let mut children = Vec::new();
        if !function
            && depth < DEPTH
            && let ExprKind::Block { items, .. } = ast.get(unparen(ast, value)).kind
        {
            for &field in ast.items(items) {
                children.extend(self.field_symbol(file, ast, field, depth + 1));
            }
        }
        Some(DocumentSymbol {
            name: self.show_segments(segments),
            detail: ty.and_then(|ty| self.short(ty)),
            kind: if function {
                SymbolKind::Function
            } else {
                SymbolKind::Field
            },
            span: ast.item(item).span,
            selection,
            children,
        })
    }

    /// The symbol of the `import` item `item` of `ast`, named as written:
    /// `a.b`, `a.b as c`, `x, y from a.b`. None for the implicit one.
    fn import_symbol(&self, ast: &Ast, item: ItemId) -> Option<DocumentSymbol> {
        let ItemKind::Import(id) = ast.item(item).kind else {
            return None;
        };
        if Some(item) == ast.implicit_import() {
            return None;
        }
        let decl = ast.import_decl(id);
        let segments = ast.path(decl.path);
        let selection = path_span(segments)?;
        let module = self.show_segments(segments);
        let name = match decl.form {
            ImportForm::Whole(Some(alias)) => {
                format!("{module} as {}", self.scan.names.get(alias.name))
            }
            ImportForm::Whole(None) => module,
            ImportForm::Members(members) => {
                let shown: Vec<&str> = ast
                    .import_members(members)
                    .iter()
                    .map(|member| match member {
                        ImportMember::Named { name, .. } => self.scan.names.get(name.name),
                        ImportMember::All(_) => "*",
                    })
                    .collect();
                format!("{} from {module}", shown.join(", "))
            }
        };
        Some(DocumentSymbol {
            name,
            detail: None,
            kind: SymbolKind::Module,
            span: ast.item(item).span,
            selection,
            children: Vec::new(),
        })
    }

    /// The type of the cell of the node written at `at` in file `file`, if
    /// the check found it.
    fn node_type(&self, file: u32, at: Span) -> Option<TyId> {
        let found = self.at(file, at.start)?;
        match found.symbol? {
            Symbol::Node(node) => self.cells.get(node as usize).copied().and_then(known),
            _ => None,
        }
    }

    /// The type of the local of file `file` whose name is at `at`.
    fn local_type(&self, file: u32, at: Span) -> Option<TyId> {
        let local = self.files.get(file as usize)?.locals.get(&at)?;
        known(local.ty)
    }

    /// The type of the expression `id` of file `file`, as its check left
    /// it.
    pub(super) fn expr_type(&self, file: u32, id: ExprId) -> Option<TyId> {
        let types = &self.files.get(file as usize)?.types;
        types.get(id.index()).copied().and_then(known)
    }

    /// `ty` on one line, cut to [`DETAIL`] characters.
    pub(super) fn short(&self, ty: TyId) -> Option<String> {
        let shown = self.one_line(ty);
        let line = shown.lines().next()?.trim();
        if line.chars().count() <= DETAIL {
            return Some(line.to_owned());
        }
        let mut cut: String = line.chars().take(DETAIL - 1).collect();
        cut.push('…');
        Some(cut)
    }

    /// The symbols of the files of the workspace whose names have `query`
    /// as a subsequence, ignoring case, `limit` at most, in input then
    /// source order: each of [`Index::document_symbols`], children
    /// included, but the imports.
    #[must_use]
    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<FoundSymbol> {
        let query: Vec<char> = query.to_lowercase().chars().collect();
        let flat = self.flat.get_or_init(|| {
            let mut flat = Vec::new();
            for file in self.standard()..u32::try_from(self.files.len()).unwrap_or(u32::MAX) {
                let symbols = self.document_symbols(file);
                flatten(file, &symbols, None, &mut flat);
            }
            flat
        });
        flat.iter()
            .filter(|symbol| subsequence(&query, &symbol.lower))
            .take(limit)
            .map(|symbol| symbol.found.clone())
            .collect()
    }
}

/// Adds `symbols`, of file `file`, and their children, to `flat`, each of
/// `container` if it is a field of one; imports apart.
fn flatten(file: u32, symbols: &[DocumentSymbol], container: Option<&str>, flat: &mut Vec<Flat>) {
    for symbol in symbols {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        flat.push(Flat {
            found: FoundSymbol {
                file,
                name: symbol.name.clone(),
                kind: symbol.kind,
                span: symbol.selection,
                container: container.map(str::to_owned),
            },
            lower: symbol.name.to_lowercase(),
        });
        flatten(file, &symbol.children, Some(&symbol.name), flat);
    }
}

/// Whether `query`'s characters are in `text` in order.
fn subsequence(query: &[char], text: &str) -> bool {
    let mut wanted = query.iter().peekable();
    for c in text.chars() {
        if wanted.peek() == Some(&&c) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}
