//! Which top-level functions each top-level item of an input mentions, and
//! the groups of mutually recursive top-level functions.
//!
//! A *top-level function* of an input is a named function item (`fn
//! NAME`) or a `def` that is directly an item of the input's outermost
//! block. Its name is in scope in the whole input, so the checker must
//! know, before it checks anything, which functions each top-level item
//! and each function body may use: that is what decides the order the
//! bodies are checked in (see the checker's module documentation, "Top-level
//! functions").
//!
//! An item or a body *mentions* a top-level function when a name in it,
//! read as a variable, is the function's and no declaration the tree shows
//! shadows it there: a parameter, a local `let` or `fn` item, a field of an
//! inner block, a `for` variable or a header variable, each from where it
//! is in scope, and a function's name for itself in its own body. The
//! first segment of a dotted path, and the variable a `:=` rebinds, count
//! as mentions too.
//!
//! The relation over-approximates, and only ever that way: a name that a
//! *spread* inside a nested block defines is not known before checking, so
//! it does not shadow here, and a name read there counts as a mention
//! although it reads the spread's field. So does a dotted `let` that would
//! declare its first segment. A mention that is not a use only makes a
//! body be checked earlier, or an item be held to the rule that a function
//! it mentions exists by then; it never hides a use.
//!
//! The groups are the strongly connected components of "mentions" between
//! the top-level functions, found by Tarjan's algorithm. They come out
//! callee first: every group a group mentions, other than itself, comes
//! before it. The order is deterministic, a function of the tree alone.

use crate::ast::{
    Ast, ExprId, ExprKind, FnId, ForHead, ForId, ItemId, ItemKind, ItemRange, PathRange,
    SegmentKind, StrPart,
};
use crate::names::Name;
use crate::source::Span;

/// What [`find`] found: the input's top-level functions, what each
/// top-level item mentions, and the groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mentions {
    /// The top-level functions, in source order. A function is referred to
    /// by its index here.
    pub functions: Vec<TopLevel>,
    /// For each item of the outermost block, by its position among them,
    /// the functions it mentions, each once, in index order. For a
    /// top-level function, those its body mentions.
    pub items: Vec<Vec<u32>>,
    /// Parallel to `items`: where each item first mentions each function
    /// it mentions, in source order the first identifier naming it.
    pub spans: Vec<Vec<Span>>,
    /// The groups, callee first, each one's members in source order.
    pub groups: Vec<Vec<u32>>,
    /// The group of each function, an index in `groups`.
    pub group_of: Vec<u32>,
}

/// One top-level function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopLevel {
    /// Its item, `fn NAME ...` or `def NAME ...`.
    pub item: ItemId,
    /// The item's position among the outermost block's items.
    pub position: usize,
    /// Its name.
    pub name: Name,
    /// Its function expression.
    pub expr: ExprId,
    /// Its function.
    pub function: FnId,
}

/// Finds the top-level functions of `ast`'s outermost block, what each of
/// its items mentions, and the groups (see the module documentation).
#[must_use]
pub fn find(ast: &Ast) -> Mentions {
    let root = ast.items(ast.root());
    let mut functions = Vec::new();
    for (position, &id) in root.iter().enumerate() {
        if let Some((name, expr, function)) = top_level_function(ast, id) {
            functions.push(TopLevel {
                item: id,
                position,
                name,
                expr,
                function,
            });
        }
    }
    let mut walker = Walker {
        ast,
        functions: &functions,
        shadows: Vec::new(),
        found: Vec::new(),
    };
    let (items, spans): (Vec<_>, Vec<_>) = root
        .iter()
        .map(|&id| {
            // A top-level item's own declaration shadows nothing for the
            // items after it: the names of the top level that clash with a
            // function's are errors of their own.
            walker.found.clear();
            walker.shadows.clear();
            walker.item(id);
            let mut found = std::mem::take(&mut walker.found);
            // Each function once, where it is first mentioned.
            found.sort_unstable_by_key(|&(function, span)| (function, span.start));
            found.dedup_by_key(|&mut (function, _)| function);
            found.into_iter().unzip::<_, _, Vec<_>, Vec<_>>()
        })
        .unzip();
    let edges: Vec<&[u32]> = root
        .iter()
        .zip(&items)
        .filter(|&(&id, _)| top_level_function(ast, id).is_some())
        .map(|(_, found)| found.as_slice())
        .collect();
    let (groups, group_of) = components(&edges);
    Mentions {
        functions,
        items,
        spans,
        groups,
        group_of,
    }
}

/// The name, function expression and function of the item `id`, if it is
/// a named function item or a `def`.
#[must_use]
pub fn top_level_function(ast: &Ast, id: ItemId) -> Option<(Name, ExprId, FnId)> {
    match ast.item(id).kind {
        ItemKind::Let {
            path,
            value,
            function: true,
            ..
        }
        | ItemKind::Field {
            path,
            value,
            function: true,
        } => match (ast.path(path), ast.get(value).kind) {
            ([single], ExprKind::Fn(function)) => Some((single.name()?, value, function)),
            _ => None,
        },
        _ => None,
    }
}

/// The strongly connected components of the graph whose node `n` has an
/// edge to each node of `edges[n]`, callee first, with each node's
/// component.
fn components(edges: &[&[u32]]) -> (Vec<Vec<u32>>, Vec<u32>) {
    /// Tarjan's state for one node: its visit index and low link, while it
    /// is on the stack.
    #[derive(Clone, Copy)]
    struct Visit {
        index: u32,
        low: u32,
        on_stack: bool,
    }
    let count = edges.len();
    let mut visits: Vec<Option<Visit>> = vec![None; count];
    let mut stack: Vec<u32> = Vec::new();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut group_of = vec![0; count];
    let mut next = 0u32;
    // An explicit work list of (node, next edge to follow), so deep chains
    // of functions do not overflow the native stack.
    let mut work: Vec<(u32, usize)> = Vec::new();
    for start in 0..count {
        if visits[start].is_some() {
            continue;
        }
        work.push((index_u32(start), 0));
        while let Some(&mut (node, ref mut edge)) = work.last_mut() {
            let n = node as usize;
            if *edge == 0 && visits[n].is_none() {
                visits[n] = Some(Visit {
                    index: next,
                    low: next,
                    on_stack: true,
                });
                next += 1;
                stack.push(node);
            }
            let targets = edges.get(n).copied().unwrap_or_default();
            if let Some(&target) = targets.get(*edge) {
                *edge += 1;
                match visits.get(target as usize).copied().flatten() {
                    None if (target as usize) < count => work.push((target, 0)),
                    Some(visit) if visit.on_stack => {
                        if let Some(Some(own)) = visits.get_mut(n) {
                            own.low = own.low.min(visit.index);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            work.pop();
            let Some(own) = visits[n] else { continue };
            if let Some(&(parent, _)) = work.last()
                && let Some(Some(up)) = visits.get_mut(parent as usize)
            {
                up.low = up.low.min(own.low);
            }
            if own.low == own.index {
                let mut group = Vec::new();
                while let Some(member) = stack.pop() {
                    if let Some(Some(visit)) = visits.get_mut(member as usize) {
                        visit.on_stack = false;
                    }
                    group_of[member as usize] = index_u32(groups.len());
                    group.push(member);
                    if member == node {
                        break;
                    }
                }
                group.sort_unstable();
                groups.push(group);
            }
        }
    }
    (groups, group_of)
}

/// `index` as a `u32`, saturating: no input has that many functions.
fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Walks a tree for the mentions of the top-level functions.
struct Walker<'a> {
    ast: &'a Ast,
    functions: &'a [TopLevel],
    /// The names declared where the walk is, innermost last.
    shadows: Vec<Name>,
    /// The functions mentioned so far, each with where, possibly repeated.
    found: Vec<(u32, Span)>,
}

impl Walker<'_> {
    /// Records `name`, read as a variable at `span`, if it is a top-level
    /// function's and nothing shadows it.
    fn name(&mut self, name: Name, span: Span) {
        if self.shadows.contains(&name) {
            return;
        }
        if let Some(index) = self.functions.iter().position(|f| f.name == name) {
            self.found.push((index_u32(index), span));
        }
    }

    /// Walks the items of a block in a scope of its own.
    fn block(&mut self, items: ItemRange) {
        let depth = self.shadows.len();
        for &id in self.ast.items(items) {
            self.item(id);
        }
        self.shadows.truncate(depth);
    }

    /// Walks one item, declaring what it declares for the items after it.
    fn item(&mut self, id: ItemId) {
        match self.ast.item(id).kind {
            ItemKind::Let { path, value, .. } | ItemKind::Field { path, value, .. } => {
                self.binding(path, value);
            }
            ItemKind::Rebind { path, value } => {
                let segments = self.ast.path(path);
                if let Some(first) = segments.first()
                    && let Some(name) = first.name()
                {
                    self.name(name, first.span);
                }
                for segment in segments {
                    if let SegmentKind::Index { position, .. } = segment.kind {
                        self.expr(position);
                    }
                }
                self.expr(value);
            }
            ItemKind::Spread(value) | ItemKind::Result(value) | ItemKind::Expr(value) => {
                self.expr(value);
            }
            ItemKind::Break(jump) | ItemKind::Continue(jump) | ItemKind::Return(jump) => {
                if let Some(value) = jump.value {
                    self.expr(value);
                }
            }
        }
    }

    /// Walks a `let` or a field of the path `path` initialised with
    /// `value`, declaring a single name for the items after it. Kept out of
    /// [`Walker::item`], as [`Walker::function`] is out of
    /// [`Walker::expr`].
    #[inline(never)]
    fn binding(&mut self, path: PathRange, value: ExprId) {
        let segments = self.ast.path(path);
        let single = match segments {
            [single] => single.name(),
            _ => None,
        };
        for segment in segments {
            if let SegmentKind::Index { position, .. } = segment.kind {
                self.expr(position);
            }
        }
        match (single, self.ast.get(value).kind) {
            // A function sees itself by the name it is bound to.
            (Some(name), ExprKind::Fn(_)) => {
                self.shadows.push(name);
                self.expr(value);
            }
            (Some(name), _) => {
                self.expr(value);
                self.shadows.push(name);
            }
            (None, _) => {
                if let Some(first) = segments.first()
                    && let Some(name) = first.name()
                {
                    self.name(name, first.span);
                }
                self.expr(value);
            }
        }
    }

    /// Walks an expression.
    fn expr(&mut self, id: ExprId) {
        let ast = self.ast;
        match ast.get(id).kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) => {}
            ExprKind::Interp(parts) => {
                for &part in ast.parts(parts) {
                    if let StrPart::Expr(expr) = part {
                        self.expr(expr);
                    }
                }
            }
            ExprKind::If {
                cond, then, else_, ..
            } => {
                self.expr(cond);
                self.expr(then);
                self.expr(else_);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Paren(expr)
            | ExprKind::Field { expr, .. }
            | ExprKind::Ascribe { expr, .. }
            | ExprKind::Is { expr, .. }
            | ExprKind::As { expr, .. } => self.expr(expr),
            ExprKind::Name(name) => self.name(name, ast.get(id).span),
            ExprKind::Block { items, .. } => self.block(items),
            ExprKind::Fn(function) => self.function(function),
            ExprKind::Lazy(function) => self.expr(ast.fn_decl(function).body),
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                for arg in ast.args(args) {
                    self.expr(arg.value);
                }
            }
            ExprKind::List(elements) => {
                for element in ast.elements(elements) {
                    self.expr(element.value);
                }
            }
            ExprKind::Index { expr, index } => {
                self.expr(expr);
                self.expr(index);
            }
            ExprKind::For(id) => self.for_loop(id),
        }
    }

    /// Walks a function expression, whose parameters shadow names in its
    /// body. Kept out of [`Walker::expr`], so that the frame each level of
    /// nesting takes on the native stack stays small.
    #[inline(never)]
    fn function(&mut self, function: FnId) {
        let decl = *self.ast.fn_decl(function);
        let depth = self.shadows.len();
        self.shadows
            .extend(self.ast.params(decl.params).iter().map(|param| param.name));
        self.expr(decl.body);
        self.shadows.truncate(depth);
    }

    /// Walks a `for` loop, whose variables shadow names in it. Kept out of
    /// [`Walker::expr`], as [`Walker::function`] is.
    #[inline(never)]
    fn for_loop(&mut self, id: ForId) {
        let ast = self.ast;
        let decl = *ast.for_decl(id);
        let depth = self.shadows.len();
        match decl.head {
            ForHead::Each {
                name,
                iterable,
                guard,
                ..
            } => {
                self.expr(iterable);
                self.shadows.push(name);
                if let Some(guard) = guard {
                    self.expr(guard);
                }
                self.expr(decl.body);
            }
            ForHead::While {
                before,
                cond,
                after,
            } => {
                for &item in ast.items(before) {
                    self.item(item);
                }
                if let Some(cond) = cond {
                    self.expr(cond);
                }
                self.expr(decl.body);
                for &item in ast.items(after) {
                    self.item(item);
                }
            }
            ForHead::Do { cond } => {
                self.expr(decl.body);
                self.expr(cond);
            }
        }
        self.shadows.truncate(depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::source::SourceFile;

    /// What the items of `text`'s outermost block mention, by the names of
    /// the functions, and the groups, by their members' names.
    fn mentions(text: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names, strings) = tokenize(&source).unwrap();
        let ast = parse(&tokens, names, strings, &source).unwrap();
        let found = find(&ast);
        let by_name = |indices: &[u32]| {
            indices
                .iter()
                .map(|&index| {
                    let name = found.functions[index as usize].name;
                    ast.names().get(name).to_owned()
                })
                .collect::<Vec<_>>()
        };
        let items = found.items.iter().map(|item| by_name(item)).collect();
        let groups = found.groups.iter().map(|group| by_name(group)).collect();
        (items, groups)
    }

    #[test]
    fn items_mention_the_functions_they_name() {
        let (items, groups) =
            mentions("x = f()\nfn f(): Int { g() }\nfn g(): Int { f() }\ndef h(): Int { 1 }");
        assert_eq!(items, [vec!["f"], vec!["g"], vec!["f"], vec![]]);
        // `f` and `g` are one group; `h` is a group of its own; a function
        // naming itself is not an edge of its own.
        assert_eq!(groups, [vec!["f", "g"], vec!["h"]]);
        // Mentioned anywhere: in a function expression or a `lazy` the
        // item only builds, in a loop body, a labelled block.
        let (items, _) = mentions(
            "a = fn(): Int { f() }\nb = lazy { f() }\nc = for i in [1] { f() }\nd = {'l f() }\n\
             fn f(): Int { 1 }",
        );
        assert_eq!(items[..4], [vec!["f"], vec!["f"], vec!["f"], vec!["f"]]);
    }

    #[test]
    fn a_nearer_declaration_shadows_a_function() {
        let (items, _) = mentions(
            "fn g(f: Int): Int { f }\nx = { let f = 1; = f }\ny = { f = 1; g = f }\n\
             z = for f in [1] { f }\nfn f(): Int { 1 }",
        );
        assert!(items[..4].iter().all(Vec::is_empty), "{items:?}");
        // From where the declaration is in scope only.
        let (items, _) = mentions("x = { a = f(); f = 1 }\nfn f(): Int { 1 }");
        assert_eq!(items[0], ["f"]);
        // A local function sees itself by its own name.
        let (items, _) = mentions("x = { fn f(): Int { f() }; = 1 }\nfn f(): Int { 1 }");
        assert_eq!(items[0], Vec::<String>::new());
    }

    #[test]
    fn a_name_a_nested_spread_defines_still_counts_as_a_mention() {
        // The spread defines `f` in the inner block, so the `f` read there
        // is the spread's field; it is not known before checking, so it is
        // counted as a mention of the function: an over-approximation.
        let (items, _) = mentions("x = { ...{f = 1}; g = f }\nfn f(): Int { 1 }");
        assert_eq!(items[0], ["f"]);
    }

    #[test]
    fn groups_come_callee_first() {
        let (_, groups) = mentions(
            "fn a(): Int { b() }\nfn b(): Int { c() }\nfn c(): Int { b() + d() }\n\
             fn d(): Int { 1 }",
        );
        assert_eq!(groups, [vec!["d"], vec!["b", "c"], vec!["a"]]);
        // Three in a ring are one group.
        let (_, groups) = mentions("fn a(): Int { b() }\nfn b(): Int { c() }\nfn c(): Int { a() }");
        assert_eq!(groups, [vec!["a", "b", "c"]]);
    }
}
