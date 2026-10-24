//! The needs among a workspace's cells, the cycles among them, and the order
//! and schedule they are checked and run in.
//!
//! # Order
//!
//! File `A` *needs* file `B` when an item of `A` (an ordered item, a
//! leaf's initialiser, a function's body) mentions a declaration `B` owns:
//! a leaf or a `def` of `B`, or an interior node one of whose leaves is
//! `B`'s. Files are checked and run in an order where each comes after the
//! files it needs, path order breaking ties ([`Scan::order`]); files that
//! need each other are an error, one per cycle. A file runs as a whole
//! before a file that needs it.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::ast::{ImportForm, ImportMember, ItemKind};
use crate::diagnostic::Diagnostic;
use crate::mentions::{self, Need};
use crate::names::Name;
use crate::source::Span;

use super::{Alias, File, FileScope, ROOT, Scan, Target, count};

/// Reports each name of `aliases`, the imports of file `index`, that clashes: a
/// whole import with another import, a top-level `fn`, `let`, `let mut`, `:=`
/// or field root of the file, or a root of its directory chain; a member or a
/// root of a `*`, which merges into the file's view with the file's fields, its
/// directory chain and the other members, with a whole import or a top-level
/// `fn`, `let`, `let mut` or `:=`. The clash family ignores what is shadowable
/// (see the module documentation, "Shadowable declarations"): an alias that is
/// (the implicit prelude's among them), and those a session's input inherits,
/// clash with nothing, and a whole import named like a field root of the file
/// or of its directory chain all of whose leaves are shadowable shadows it
/// silently ([`whole_imports_shadow`](super::views::whole_imports_shadow)).
/// Each name that clashes is added to `poisoned`, read as `Error` in the file
/// (see [Scan errors](crate::workspace::scanning#scan-errors)).
pub(super) fn clashes(
    scan: &Scan,
    index: u32,
    aliases: &[Alias],
    (errors, poisoned): (&mut Vec<Diagnostic>, &mut Vec<Name>),
) {
    let Some(file) = scan.files.get(index as usize) else {
        return;
    };
    // Whether an alias clashes with nothing: inherited, or shadowable, by
    // its import's mark or, for a member, by what it brings.
    let quiet = |alias: &Alias| {
        alias.inherited
            || file.marked(alias)
            || (!alias.whole && scan.target_shadowable(alias.target))
    };
    if aliases.iter().all(quiet) {
        return;
    }
    let ast = &file.ast;
    let tree = &file.mentions.tree;
    // The file's own top-level names, first declaration of each, and
    // whether it is a field's, but a field root all of whose leaves are
    // shadowable, and a declaration a later one shadows.
    let mut declared: Vec<(Name, Span, bool)> = Vec::new();
    let mut seen: HashSet<(Name, bool)> = HashSet::new();
    for (position, &item) in ast.items(ast.root()).iter().enumerate() {
        let (field, path) = match ast.item(item).kind {
            ItemKind::Let { path, .. } | ItemKind::Rebind { path, .. } => (false, path),
            ItemKind::Field { path, .. } => (true, path),
            _ => continue,
        };
        if file.mentions.shadowed.get(position) == Some(&true) {
            continue;
        }
        if let Some(first) = ast.path(path).first()
            && let Some(name) = first.name()
            && !(field
                && tree
                    .root(name)
                    .and_then(|root| tree.nodes.get(root as usize))
                    .is_some_and(|root| root.shadowable))
            && seen.insert((name, field))
        {
            declared.push((name, first.span, field));
        }
    }
    let star = |alias: &Alias| {
        let Some(ItemKind::Import(id)) = alias.item.map(|item| ast.item(item).kind) else {
            return false;
        };
        matches!(ast.import_decl(id).form, ImportForm::Members(range)
            if ast.import_members(range).iter().any(|m| m == &ImportMember::All(alias.span)))
    };
    // A shadowable import clashes with nothing: its names yield to every
    // declaration of theirs (see the module documentation, "Shadowable
    // declarations"). Nor does an inherited import, which the input's
    // declarations shadow ([`inherit_imports`]).
    for (position, alias) in aliases.iter().enumerate() {
        if quiet(alias) {
            continue;
        }
        let shown = scan.names.get(alias.name);
        let earlier = aliases
            .get(..position)
            .and_then(|before| {
                before
                    .iter()
                    .find(|a| !quiet(a) && a.name == alias.name && (alias.whole || a.whole))
            })
            .map(|a| (a.span, "imported here"));
        let other = earlier.or_else(|| {
            declared
                .iter()
                .filter(|&&(n, _, field)| n == alias.name && (alias.whole || !field))
                .min_by_key(|&&(_, span, _)| span.start)
                .map(|&(_, span, _)| (span, "declared here"))
        });
        if let Some((span, label)) = other {
            let message = format!("`{shown}` is imported in this file");
            errors.push(if star(alias) || label == "imported here" {
                Diagnostic::error(message, alias.span).with_context(span, label)
            } else {
                Diagnostic::error(message, span).with_context(alias.span, "imported here")
            });
            poisoned.push(alias.name);
            continue;
        }
        let above = scan
            .chain(file.dir)
            .find_map(|dir| scan.by_name.get(&(dir, ROOT, alias.name)));
        if alias.whole && above.is_some_and(|&node| !scan.target_shadowable(Target::Node(node))) {
            errors.push(Diagnostic::error(
                format!("`{shown}` is a field of this directory"),
                alias.span,
            ));
            poisoned.push(alias.name);
        }
    }
}

/// The error that file `index` reaches `target`, which it computes itself,
/// through an import at `at`: the one-file form of a cycle.
pub(super) fn needs_itself(scan: &Scan, index: u32, target: Target, at: Span) -> Diagnostic {
    let label = format!(
        "`{}` is computed when this file runs",
        scan.show_target(target)
    );
    needs_itself_because(scan, index, label, at)
}

/// The error that file `index` needs itself through the reference at `at`,
/// with `label` saying why.
pub(super) fn needs_itself_because(scan: &Scan, index: u32, label: String, at: Span) -> Diagnostic {
    let path = scan
        .files
        .get(index as usize)
        .map_or("", |f| f.path.as_str());
    Diagnostic::error(format!("`{path}` needs itself"), at)
        .with_label(label)
        .with_note(
            "a file runs as a whole before a file that needs it, so a file cannot need itself",
        )
        .in_input(index)
}

/// Reports each whole read of a directory through an import (`whole_dirs`:
/// the reading file, the directory's files, the target, the reference) one
/// of whose files needs the reading file back, in the needs `all` found so
/// far: the one-file form of a cycle too, "`app.gcsl` needs itself", at the
/// read, once per file (`itselves` saying which files have had theirs). The
/// needs that would close a cycle of two files with it are dropped, so that
/// the cycle is reported once.
pub(super) fn whole_reads_needing_back(
    scan: &Scan,
    whole_dirs: Vec<(u32, Vec<u32>, Target, Span)>,
    all: &mut [HashMap<u32, (Span, String)>],
    itselves: &mut [bool],
    errors: &mut Vec<Diagnostic>,
) {
    for (index, owners, target, span) in whole_dirs {
        let back: Vec<u32> = owners
            .into_iter()
            .filter(|&o| all.get(o as usize).is_some_and(|f| f.contains_key(&index)))
            .collect();
        if back.is_empty() {
            continue;
        }
        if let Some(itself) = itselves.get_mut(index as usize)
            && !*itself
        {
            *itself = true;
            let file = back
                .first()
                .and_then(|&o| scan.files.get(o as usize))
                .map_or("", |f| f.path.as_str());
            let label = format!(
                "`{}` is read whole, and its file `{file}` needs this file",
                scan.show_target(target)
            );
            errors.push(needs_itself_because(scan, index, label, span));
        }
        if let Some(first) = all.get_mut(index as usize) {
            for owner in back {
                first.remove(&owner);
            }
        }
    }
}

/// The files a whole read by file `index` of its view node `view` needs:
/// every owner of every leaf under it, in whichever directory or import,
/// but `index` itself, whose own leaves are computed in its block; with the
/// node of an import that reaches into `index`, if one does, which is a
/// cycle of one file, as a reach through a whole import is.
pub(super) fn view_owners(scan: &Scan, index: u32, view: u32) -> (Vec<u32>, Option<Target>) {
    let mut owners = Vec::new();
    let mut reached = None;
    let parts = scan.views.get(view as usize).map_or(&[][..], |v| &v.parts);
    for &part in parts {
        let files = scan.part_owners(part);
        if !part.chain && files.contains(&index) {
            reached.get_or_insert(part.target);
        } else {
            owners.extend(files.into_iter().filter(|&file| file != index));
        }
    }
    (owners, reached)
}

/// The field nodes each item of `file`'s outermost block mentions, with its
/// position, but a shadowed declaration's, which needs nothing and reads
/// nothing whole computed for it (see the module documentation,
/// "Shadowable declarations").
pub(super) fn needing_items(file: &File) -> impl Iterator<Item = (usize, &Vec<u32>)> {
    let shadowed = &file.mentions.shadowed;
    file.mentions
        .fields
        .iter()
        .enumerate()
        .filter(move |&(position, _)| shadowed.get(position) != Some(&true))
}

/// Fills [`Scan::needs`], and returns for each file the first mention, in
/// source order, of a declaration of each file it needs, with what it
/// shows.
pub(super) fn find_needs(
    scan: &mut Scan,
    errors: &mut Vec<Diagnostic>,
) -> Vec<HashMap<u32, (Span, String)>> {
    let mut all = Vec::with_capacity(scan.files.len());
    let mut itselves = Vec::with_capacity(scan.files.len());
    // Each whole read of a directory through an import: the reading file,
    // the directory's files, the target and the reference.
    let mut whole_dirs: Vec<(u32, Vec<u32>, Target, Span)> = Vec::new();
    for (index, file) in (0u32..).zip(&scan.files) {
        let scope = FileScope::new(scan, file);
        let mut itself = false;
        let mut first = method_needs(scan, index, file);
        context_needs(scan, index, file, &mut first);
        let mut record = |owner: u32, span: Span, shown: &dyn Fn() -> String| {
            if owner == index {
                return;
            }
            let entry = first.entry(owner).or_insert_with(|| (span, shown()));
            if span.start < entry.0.start {
                *entry = (span, shown());
            }
        };
        for (position, fields) in needing_items(file) {
            let spans = file.mentions.field_spans.get(position);
            let foreign = file.mentions.foreign.get(position).into_iter().flatten();
            let mut nodes: Vec<(u32, Span)> = Vec::new();
            // The nodes of the view it reads whole, each with the reference.
            let mut views: Vec<(u32, Span)> = Vec::new();
            for (&n, &span) in fields.iter().zip(spans.into_iter().flatten()) {
                if let Some(&global) = file.global.get(n as usize) {
                    nodes.push((global, span));
                } else if let Some(view) = scope.own_view(n) {
                    views.push((view, span));
                }
            }
            for &(need, span) in foreign {
                match need {
                    // Through an import: what it reaches needs its files,
                    // and reaching into this file is a cycle of one file.
                    Need::Field(n) => {
                        let target = match scope.target(n) {
                            None => {
                                nodes.extend(scope.merged(n).map(|merged| (merged, span)));
                                continue;
                            }
                            Some(Target::View(view)) => {
                                views.push((view, span));
                                continue;
                            }
                            Some(target) => target,
                        };
                        let owners = scan.target_files(target);
                        if owners.contains(&index) {
                            if !itself {
                                itself = true;
                                errors.push(needs_itself(scan, index, target, span));
                            }
                            continue;
                        }
                        for &owner in &owners {
                            record(owner, span, &|| scan.show_target(target));
                        }
                        if matches!(target, Target::Dir(_)) {
                            whole_dirs.push((index, owners, target, span));
                        }
                    }
                    Need::Function(f) => {
                        if let Some(function) = scope
                            .foreign_function(f)
                            .and_then(|id| scan.functions.get(id as usize))
                        {
                            record(function.file, span, &|| scan.show_function(*function));
                        }
                    }
                }
            }
            for (merged, span) in nodes {
                for owner in scan.target_files(Target::Node(merged)) {
                    record(owner, span, &|| scan.show(merged));
                }
            }
            for (view, span) in views {
                let (owners, reached) = view_owners(scan, index, view);
                if let Some(target) = reached
                    && !itself
                {
                    itself = true;
                    errors.push(needs_itself(scan, index, target, span));
                }
                for owner in owners {
                    record(owner, span, &|| scan.show_view(view));
                }
            }
        }
        all.push(first);
        itselves.push(itself);
    }
    whole_reads_needing_back(scan, whole_dirs, &mut all, &mut itselves, errors);
    // A file that needs itself is not checked (see [Scan
    // errors](crate::workspace::scanning#scan-errors)).
    scan.cyclic = itselves;
    scan.needs = all
        .iter()
        .map(|first| {
            let mut needs: Vec<u32> = first.keys().copied().collect();
            needs.sort_unstable();
            needs
        })
        .collect();
    all
}

/// Reports each cycle of files that need each other, once, and recovers
/// from it (see [Scan errors](crate::workspace::scanning#scan-errors)): each file of it is marked
/// cyclic ([`Scan::is_cyclic`]), not to be checked, and its needs of the
/// other files of it are forgotten, so that the order has every file.
pub(super) fn report_cycles(
    scan: &mut Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) {
    let groups = cycle_groups(scan, mentioned, errors);
    for group in groups {
        for &file in &group {
            if let Some(entry) = scan.cyclic.get_mut(file as usize) {
                *entry = true;
            }
            if let Some(needs) = scan.needs.get_mut(file as usize) {
                needs.retain(|need| !group.contains(need));
            }
        }
    }
}

/// Reports each cycle of `scan`'s files that need each other, once, and
/// returns the files of each.
pub(super) fn cycle_groups(
    scan: &Scan,
    mentioned: &[HashMap<u32, (Span, String)>],
    errors: &mut Vec<Diagnostic>,
) -> Vec<Vec<u32>> {
    let edges: Vec<&[u32]> = scan.needs.iter().map(Vec::as_slice).collect();
    let (mut groups, _) = mentions::components(&edges);
    for group in &mut groups {
        group.sort_unstable();
    }
    groups.sort();
    for group in groups.iter().filter(|group| group.len() > 1) {
        let path = |file: u32| {
            scan.files
                .get(file as usize)
                .map_or_else(String::new, |f| format!("`{}`", f.path))
        };
        let shown: Vec<String> = group.iter().map(|&f| path(f)).collect();
        let listed = match shown.split_last() {
            Some((last, rest)) if rest.len() > 1 => format!("{} and {last}", rest.join(", ")),
            Some((last, rest)) => format!("{} and {last}", rest.join("")),
            None => String::new(),
        };
        // Each file's first mention of another file of the cycle.
        let uses = group.iter().filter_map(|&file| {
            let first = mentioned.get(file as usize)?;
            let (other, (at, what)) = group
                .iter()
                .filter_map(|other| Some((*other, first.get(other)?)))
                .min_by_key(|(_, (at, _))| at.start)?;
            let text = match (
                what.strip_prefix(METHOD_NEED),
                what.strip_prefix(CONTEXT_NEED),
            ) {
                (Some(name), _) => format!(
                    "{} may call the method `{name}` of {} here",
                    path(file),
                    path(other)
                ),
                (None, Some(name)) => format!(
                    "{} reads the context `{name}` of {} here",
                    path(file),
                    path(other)
                ),
                (None, None) => format!("{} uses `{what}` here", path(file)),
            };
            Some((file, *at, text))
        });
        let mut diagnostic: Option<Diagnostic> = None;
        for (file, span, text) in uses {
            diagnostic = Some(match diagnostic {
                None => Diagnostic::error(format!("{listed} need each other"), span)
                    .with_label(text)
                    .in_input(file),
                Some(diagnostic) => diagnostic.with_context_in(file, span, text),
            });
        }
        if let Some(diagnostic) = diagnostic {
            errors.push(diagnostic.with_note(
                "a file runs as a whole before a file that needs it, so two files cannot need \
                 each other",
            ));
        }
    }
    groups.retain(|group| group.len() > 1);
    groups
}

/// The files file `index`, `file`, needs for its calls of fields, each with its
/// first such call and what it is shown as: those of every method of the called
/// name it sees, which the call may mean (see
/// [Methods](crate::workspace::declared#methods)).
pub(super) fn method_needs(scan: &Scan, index: u32, file: &File) -> HashMap<u32, (Span, String)> {
    let mut first: HashMap<u32, (Span, String)> = HashMap::new();
    if file.method_view.is_empty() {
        return first;
    }
    for (name, span) in file.ast.field_calls() {
        for method in file.method_view.get(&name).into_iter().flatten() {
            if method.file == index {
                continue;
            }
            let entry = first
                .entry(method.file)
                .or_insert_with(|| (span, format!("{METHOD_NEED}{}", scan.names.get(name))));
            if span.start < entry.0.start {
                *entry = (span, format!("{METHOD_NEED}{}", scan.names.get(name)));
            }
        }
    }
    first
}

/// Adds to `first` the files file `index`, `file`, needs for its reads of
/// contexts, each with its first such read and what it is shown as, unless an
/// earlier need of the file is: the file declaring the context each reads,
/// whose default the read may call (see
/// [Context](crate::workspace::declared#context)), but a session's earlier
/// input's, and its own.
pub(super) fn context_needs(
    scan: &Scan,
    index: u32,
    file: &File,
    first: &mut HashMap<u32, (Span, String)>,
) {
    for (name, span) in file.ast.context_reads() {
        // Its own declaration, or an earlier input's, is read in place.
        let own = scan
            .contexts
            .iter()
            .any(|c| c.name == name && c.file == index);
        let inherited = index >= scan.standard
            && scan
                .inherited_contexts
                .iter()
                .any(|&(earlier, ..)| earlier == name);
        if own || inherited {
            continue;
        }
        let Some(declared) = scan
            .contexts
            .iter()
            .find(|c| c.name == name && c.binding.is_some())
        else {
            continue;
        };
        let shown = format!("{CONTEXT_NEED}{}", scan.names.get(name));
        let entry = first
            .entry(declared.file)
            .or_insert_with(|| (span, shown.clone()));
        if span.start < entry.0.start {
            *entry = (span, shown);
        }
    }
}

/// What a need of a context's default is shown as, before the context's
/// name, among the needs a cycle of files is reported with (see
/// [Context](crate::workspace::declared#context)): no field's path holds `<`.
pub(super) const CONTEXT_NEED: &str = "<context> ";

/// What a need of the methods of a name is shown as, before the name, among
/// the needs a cycle of files is reported with (see [Methods](crate::workspace::declared#methods)):
/// no field's path holds `<`.
pub(super) const METHOD_NEED: &str = "<method> ";

/// The files in the order they run: repeatedly the path-smallest file all
/// of whose needs are placed. `needs` has no cycles.
pub(super) fn order(needs: &[Vec<u32>]) -> Vec<u32> {
    let mut waiting: Vec<usize> = needs.iter().map(Vec::len).collect();
    let mut users: Vec<Vec<u32>> = vec![Vec::new(); needs.len()];
    for (file, list) in (0u32..).zip(needs) {
        for &need in list {
            if let Some(entry) = users.get_mut(need as usize) {
                entry.push(file);
            }
        }
    }
    let mut ready: BinaryHeap<Reverse<u32>> = (0u32..)
        .zip(&waiting)
        .filter(|&(_, &n)| n == 0)
        .map(|(file, _)| Reverse(file))
        .collect();
    let mut order = Vec::with_capacity(needs.len());
    while let Some(Reverse(file)) = ready.pop() {
        order.push(file);
        for &user in users.get(file as usize).into_iter().flatten() {
            if let Some(entry) = waiting.get_mut(user as usize) {
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    ready.push(Reverse(user));
                }
            }
        }
    }
    order
}

/// Finds each file's schedule, its shared interior nodes that a later
/// file in the order computes left out, and the nodes of its view with a
/// leaf of the file under them that it reads whole in: those mentioned and
/// the view nodes under them, each needing its own children (see
/// [`Mentions::views`](crate::mentions::Mentions::views)).
pub(super) fn schedule(scan: &mut Scan) {
    let edges: Vec<(Vec<mentions::ViewNode>, Vec<bool>)> = scan
        .files
        .iter()
        .map(|file| view_edges(scan, file))
        .collect();
    let mut needs = Vec::with_capacity(edges.len());
    for (file, (views, needed)) in scan.files.iter_mut().zip(edges) {
        file.mentions.views = views;
        needs.push(needed);
    }
    let rank = &scan.rank;
    let mut schedules = Vec::with_capacity(scan.files.len());
    for ((index, file), needed) in (0u32..).zip(&scan.files).zip(needs) {
        let mut done: Vec<bool> = file
            .global
            .iter()
            .map(|&merged| {
                scan.nodes.get(merged as usize).is_some_and(|node| {
                    node.leaf.is_none()
                        && node
                            .contributors
                            .iter()
                            .max_by_key(|&&f| rank.get(f as usize).copied())
                            != Some(&index)
                })
            })
            .collect();
        done.extend(needed.iter().map(|&needed| !needed));
        schedules.push(file.mentions.schedule_except(&done));
    }
    for (file, schedule) in scan.files.iter_mut().zip(schedules) {
        file.schedule = schedule;
    }
}

/// The nodes of `file`'s view with a leaf of the file under them, as the nodes
/// of its graph of what needs what
/// ([`Mentions::views`](crate::mentions::Mentions::views)), placed by the
/// file's own node of their path, with whether the file reads each whole: those
/// it mentions, and the view nodes under them. Only those have their children,
/// the file's own nodes or such view nodes, as edges: finding every one's would
/// cost every file of a directory the children of its directory's nodes, which
/// a whole read costs only the file that reads them.
pub(super) fn view_edges(scan: &Scan, file: &File) -> (Vec<mentions::ViewNode>, Vec<bool>) {
    let scope = FileScope::new(scan, file);
    let own = count(file.global.len());
    let start = file.view.start;
    let nodes = scan
        .views
        .get(start as usize..start.saturating_add(file.view.own) as usize)
        .unwrap_or_default();
    let mut views: Vec<mentions::ViewNode> = nodes
        .iter()
        .map(|node| {
            let tree = node
                .own
                .and_then(|own| file.mentions.tree.nodes.get(own as usize));
            mentions::ViewNode {
                name: node.name,
                parent: node
                    .parent
                    .map(|parent| own.saturating_add(parent.saturating_sub(start))),
                children: Vec::new(),
                first: tree.map_or(0, |tree| tree.first),
                last: tree.map_or(0, |tree| tree.last),
            }
        })
        .collect();
    let mut needed = vec![false; views.len()];
    let mut stack: Vec<u32> = needing_items(file)
        .flat_map(|(_, fields)| fields)
        .copied()
        .filter(|&n| scope.own_view(n).is_some())
        .collect();
    while let Some(number) = stack.pop() {
        let at = number.saturating_sub(own) as usize;
        if needed.get(at) != Some(&false) {
            continue;
        }
        let children: Vec<u32> = scope
            .view_children(number)
            .into_iter()
            .map(|(_, child)| child)
            .filter(|&child| child < scope.merged_start())
            .collect();
        stack.extend(children.iter().filter(|&&c| scope.own_view(c).is_some()));
        if let (Some(seen), Some(view)) = (needed.get_mut(at), views.get_mut(at)) {
            *seen = true;
            view.children = children;
        }
    }
    (views, needed)
}
