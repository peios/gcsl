//! Invoking definitions and functions of a workspace, as a host does.

use std::mem;
use std::collections::HashMap;
use std::rc::Rc;

use crate::{Outcome, session};
use crate::ast::{
    self, Arg, ArgKind, Ast, AstBuilder, Expr, ExprKind, Item, ItemId, ItemKind, Param, PathSegment,
};
use crate::check::{Checker, KnownParams, Scope, Slot};
use crate::diagnostic::Diagnostic;
use crate::ir::Ir;
use crate::names::{self, Name, Names};
use crate::source::Span;
use crate::strings::Strings;
use crate::types::{Laziness, TyId};
use crate::value::{ContextFrame, Quantity, Value};

use super::{
    Argument, DefType, Definition, FoundDefType, Given, INVOKED_DEFINITION, INVOKED_DEF_TYPE,
    Invocable, InvokeError, Referenced, RequestError, ResolvedDefinition, Scalar, Workspace, count,
    merge_files,
};

impl Workspace {
    /// Every def-type of the workspace's own files, the standard files'
    /// aside, with every definition made with it, a def-type with none
    /// included, in path order of their files, then in source order (see
    /// the module documentation, "Def-types by name"). A definition is
    /// found in whichever file it is, whichever files see it, and listed
    /// under the def-type its `DT` names, through whatever import or alias,
    /// by its path as it writes it; a `#[shadowable]` one that a later one
    /// shadows is found too.
    ///
    /// # Errors
    ///
    /// One error for each chain of definitions of one path of a def-type in
    /// directories that see each other, one through another, "`package
    /// dev.peios.x` is defined 2 times in directories that see each other",
    /// and each path of which two def-types or
    /// more are declared, "def-type `package` is declared 2 times in the
    /// workspace", at the first in path then source order, labelled "also
    /// defined here", or "also declared here", at each other; and one error
    /// for each definition whose path lies under another's of its def-type
    /// in a directory that sees its own, "`package app.core` lies under another
    /// `package` definition", at it, labelled "`app` is defined here" at
    /// each definition above it; all in path order of their files, then in
    /// source order, with the workspace's warnings, as [`Workspace::run`]
    /// returns its errors. None is an error of the language: the workspace
    /// checks and runs regardless. One path in directories that see nothing
    /// of each other is no error: each is named with its directory.
    pub fn def_types(&self) -> Result<Vec<DefType>, Vec<Diagnostic>> {
        let found = self.found_def_types();
        let duplicates = duplicates(&found);
        if !duplicates.is_empty() {
            return Err(merge_files(duplicates, self.warnings.clone()));
        }
        Ok(found.into_iter().map(|found| found.listed).collect())
    }

    /// Invokes the definition of path `path` made with the def-type of path
    /// `def_type`, both as written (`"package"`, `"dev.peios.testpkg"`,
    /// `"org.gnu.gmp-c++"`, `"org.golang.go-stage0-1.24"`), compared with
    /// the listed paths as text, the definition's alone if it is the only
    /// one of its path, else qualified by its directory, `"pkgs/a/main"`, or
    /// `"/main"` for the root's ([`Definition::qualified`]),
    /// with the arguments `args`, each the name of one of its invocation
    /// parameters and the value given for it, as if `DT PATH(NAME = VALUE,
    /// ...)` were called from a file of the definition's directory, each
    /// value a literal of its kind (see the module documentation, "Def-types
    /// by name"): the definition's file and the def-type's run first, each as a
    /// whole, with the files they need, unless they have run, and then the
    /// def-type is called with the definition's path, the definition and
    /// the arguments, each filling the parameter it names, each one given
    /// none given its default, its caller the directory of
    /// the definition's file. Returns the value
    /// the def-type returns, with the warnings of the whole workspace.
    /// Invoked again, the call runs again.
    ///
    /// # Errors
    ///
    /// In this order: the duplicates [`Workspace::def_types`] fails with,
    /// definitions under others among them, with the warnings; no def-type
    /// of path `def_type`, no definition of it of path `path`, `path` above
    /// some, or `path` alone defined in several directories
    /// ([`RequestError::Ambiguous`]), and the errors of the arguments that
    /// [`Workspace::arguments`] finds, a value of another type than its
    /// parameter's among them, each a [`RequestError`]; and the runtime
    /// error of a file that runs or of the call, as
    /// [`Workspace::run`] returns it, a part of it in the call itself,
    /// which has no source, located at the definition's path, labelled
    /// "invoked here".
    pub fn invoke(
        &mut self,
        def_type: &str,
        path: &str,
        args: &[(String, Given)],
    ) -> Result<Outcome, InvokeError> {
        self.invoke_in((def_type, path), args, None)
    }

    /// [`Workspace::invoke`], the call in the root context `root`, the
    /// frame of the contexts the host sets ([`Workspace::invoke_with`]).
    pub(super) fn invoke_in(
        &mut self,
        (def_type, path): (&str, &str),
        args: &[(String, Given)],
        root: Option<Rc<ContextFrame>>,
    ) -> Result<Outcome, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        } = self.invoked(def_type, path)?;
        let given = args
            .iter()
            .map(|(name, value)| (name.as_str(), Argument::Given(value)));
        let args = self
            .given(&found, (def_type_cell.1, path), given)
            .map_err(InvokeError::Request)?;
        let Some(definition) = found.listed.definitions.get(at) else {
            return Err(InvokeError::Request(missing_definition(
                &found.listed,
                path,
            )));
        };
        // The files a reference to the definition needs, each run whole.
        self.run_needed(definition.file)
            .map_err(InvokeError::Diagnostics)?;
        self.run_needed(found.listed.file)
            .map_err(InvokeError::Diagnostics)?;
        let known = self.def_type_known(&found);
        self.call_definition(
            (definition, Invocation::Call(&args)),
            (def_type_cell, definition_cell),
            (known, root),
        )
    }

    /// [`Workspace::invoke`], in a context the host sets: each of
    /// `contexts` a context's name and its value, data of its declared type
    /// ([`TypeTable::holds`](super::TypeTable::holds)), bound around the call, as a `with` would
    /// bind it, for every call the definition makes (see the checker's
    /// "Context"). The files the invocation runs, each as a whole, run in
    /// the declared defaults, as they always do, so what a file computes
    /// never depends on an invocation's context. Pekit sets
    /// `host_platform` so for each target it evaluates.
    ///
    /// # Errors
    ///
    /// Before anything else, for each of `contexts` in order: a name no
    /// context of the workspace has ([`RequestError::NoContext`]), one given
    /// twice ([`RequestError::DuplicateContext`]), or a value not of its
    /// context's type ([`RequestError::ContextValue`]); then those of
    /// [`Workspace::invoke`].
    pub fn invoke_with(
        &mut self,
        (def_type, path): (&str, &str),
        args: &[(String, Given)],
        contexts: &[(String, Value)],
    ) -> Result<Outcome, InvokeError> {
        let root = self.root_frame(contexts).map_err(InvokeError::Request)?;
        self.invoke_in((def_type, path), args, root)
    }

    /// The root frame of the context a host sets for an invocation, each of
    /// `contexts` a context's name and its value, checked as
    /// [`Workspace::invoke_with`] says; `None` for none.
    pub(super) fn root_frame(
        &mut self,
        contexts: &[(String, Value)],
    ) -> Result<Option<Rc<ContextFrame>>, RequestError> {
        if contexts.is_empty() {
            return Ok(None);
        }
        let mut binds: Vec<(Name, Value)> = Vec::with_capacity(contexts.len());
        for (name, value) in contexts {
            let found = self
                .scan
                .names
                .lookup(name)
                .and_then(|name| self.scan.contexts.iter().find(|c| c.name == name));
            let Some(declared) = found else {
                let mut names: Vec<String> = self
                    .scan
                    .contexts
                    .iter()
                    .map(|c| self.scan.names.get(c.name).to_owned())
                    .collect();
                names.sort_unstable();
                names.dedup();
                return Err(RequestError::NoContext {
                    name: name.clone(),
                    contexts: names,
                });
            };
            if binds.iter().any(|&(key, _)| key == declared.key) {
                return Err(RequestError::DuplicateContext { name: name.clone() });
            }
            let ty = self.types.context_type(declared.key).unwrap_or(TyId::ERROR);
            if !self.types.holds(value, ty) {
                let ty = self.types.display(ty, &self.scan.names).to_string();
                return Err(RequestError::ContextValue {
                    name: name.clone(),
                    ty,
                });
            }
            binds.push((declared.key, value.clone()));
        }
        Ok(Some(Rc::new(ContextFrame {
            id: self.memo.new_context_frame(),
            binds: binds.into_boxed_slice(),
            outer: None,
        })))
    }

    /// Reads the arguments `args` for the invocation parameters of the
    /// def-type of path `def_type`, to invoke its definition of path `path`
    /// with ([`Workspace::invoke`]), both as written: each the name of a
    /// parameter and the text of its value, read by the parameter's declared
    /// type (an impure def-type's either shape, `String | Future<String>`
    /// of a bare `String`, taken off), which must be one of `String`,
    /// `Int`, `Float` and `Bool` (see [`Given`]): a `String` the text as it
    /// is, an `Int` a decimal integer, with `-` before it if negative, in
    /// range, a `Float` a decimal number, finite, a `Bool` `true` or
    /// `false`. A name with no text is checked as one given, its value not
    /// known yet, and left out of what is returned: so a host can check a
    /// request before it knows every value. Runs nothing: what it finds
    /// wrong, invoking would. Returns each value read, with its parameter's
    /// name, in the order of `args`.
    ///
    /// # Errors
    ///
    /// In this order: the duplicates [`Workspace::def_types`] fails with, with
    /// the warnings; no def-type of path `def_type`, no definition of it of
    /// path `path`, `path` above some, or `path` alone defined in several
    /// directories; an invocation parameter that has
    /// no default and is not one of the four types, the first; then, for
    /// each argument in order, a name that is no invocation parameter's, a
    /// parameter named twice, a parameter given an argument that is not one
    /// of the four types, or a text that is not a value of its type; and
    /// every parameter that has no default given none, each a
    /// [`RequestError`].
    pub fn arguments(
        &self,
        def_type: &str,
        path: &str,
        args: &[(&str, Option<&str>)],
    ) -> Result<Vec<(String, Given)>, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            def_type_cell,
            ..
        } = self.invoked(def_type, path)?;
        let texts = args
            .iter()
            .map(|&(name, text)| (name, Argument::Text(text)));
        self.given(&found, (def_type_cell.1, path), texts)
            .map_err(InvokeError::Request)
    }

    /// Reads the field `field` of the meta of the definition of path `path`
    /// made with the def-type of path `def_type` (see [Metas](#metas)), as
    /// `(DT PATH).FIELD` would be read from a file of the definition's
    /// directory: the definition's file runs first, with the files it
    /// needs, unless they have run, and then the meta is read, running its
    /// `defmeta` as its marker says. Returns `None` if the def-type declares
    /// no meta or its meta has no field `field`. What `pekit` reads a
    /// package's versions with, to list them when none is given.
    ///
    /// # Errors
    ///
    /// As [`Workspace::invoke`], but for its arguments' errors: the
    /// duplicates, no such def-type or definition, and the runtime error of
    /// a file that runs or of the read.
    pub fn meta_field(
        &mut self,
        def_type: &str,
        path: &str,
        field: &str,
    ) -> Result<Option<Outcome>, InvokeError> {
        let ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        } = self.invoked(def_type, path)?;
        let Some(definition) = found.listed.definitions.get(at) else {
            return Err(InvokeError::Request(missing_definition(
                &found.listed,
                path,
            )));
        };
        // The definition's parameter's type, its meta's.
        let func = self
            .types
            .fn_sig(self.types.strip(def_type_cell.1))
            .and_then(|sig| sig.params.get(1))
            .map(|&(_, ty)| self.types.strip(ty));
        let Some(func) = func else {
            return Ok(None);
        };
        let meta = self.types.fn_sig(func).and_then(|sig| sig.meta);
        let has_field = meta.is_some_and(|meta| {
            let record = self.types.strip(meta.ty);
            let name = self.scan.names.lookup(field);
            name.is_some_and(|name| self.types.field(record, name).is_some())
        });
        if !has_field {
            return Ok(None);
        }
        self.run_needed(definition.file)
            .map_err(InvokeError::Diagnostics)?;
        self.call_definition(
            (definition, Invocation::MetaField(field)),
            (def_type_cell, (definition_cell.0, func)),
            (None, None),
        )
        .map(Some)
    }

    /// Which definition `value` names, if it is the closure of a def-type
    /// reference, `DT PATH` evaluated (see the module documentation,
    /// "Def-types by name"): its def-type's path as declared, whatever
    /// import or alias the reference names it by, and the definition's path
    /// as it writes it, found by the definition the closure captures, which
    /// is the closure its definition's cell holds. What a host that gets a
    /// reference back, from an invocation or a meta (a package's `build =
    /// build busybox`), reads to invoke it or read its meta by name. `None`
    /// for any other value, a closure that calls a definition included, and
    /// for a reference to no definition of the workspace's def-types. Runs
    /// nothing and changes nothing.
    #[must_use]
    pub fn definition_of(&self, value: &Value) -> Option<Referenced> {
        let Value::Fn(closure) = value else {
            return None;
        };
        if !closure.code.is_reference(closure.function) {
            return None;
        }
        let Some(Value::Fn(definition)) = closure
            .captures
            .get(crate::lower::REFERENCE_DEFINITION as usize)
        else {
            return None;
        };
        let holds = |file: u32, item: ItemId| {
            let cell = self.leaf_cell(file, item);
            let held = cell.and_then(|(slot, _)| self.frame.get(slot.index()));
            matches!(held, Some(Value::Fn(held)) if Rc::ptr_eq(held, definition))
        };
        self.found_def_types().into_iter().find_map(|found| {
            let (_, listed) = found
                .items
                .iter()
                .zip(&found.listed.definitions)
                .find(|&(&item, listed)| holds(listed.file, item))?;
            Some(Referenced {
                def_type: found.listed.path.clone(),
                path: listed.path.clone(),
                dir: listed.dir.clone(),
            })
        })
    }

    /// The top-level field or `def` of file `file` named `name`, a field of
    /// the file's own record, which another file reads as a member of its
    /// module (`pekit.environment` of `pekit.gcsl` at the root): its type
    /// and, if it is a function, what it may do and how many parameters it
    /// takes (see the module documentation, "Top-level functions by name").
    /// `None` if the file declares none such: no item of the name, or only
    /// a private one (`let`, `fn`), a dotted path under the name, or one
    /// shadowed in its own file.
    #[must_use]
    pub fn top_level(&self, file: u32, name: &str) -> Option<TopLevel> {
        let (item, ((_, ty), _)) = self.top_level_cell(file, name)?;
        let function = self.types.fn_sig(self.types.strip(ty));
        Some(TopLevel {
            ty: self.shown_top_level(file, item, ty),
            function: function.map(|sig| (sig.effect, sig.params.len())),
        })
    }

    /// Whether the top-level function of file `file` named `name`, as
    /// [`Workspace::top_level`] finds it, has returned for no arguments: as
    /// a call of it with none written in a file of `file`'s directory would
    /// find, its closure, which the file stored as it ran, holding a value
    /// for them in the workspace's memo tables (see the evaluator's
    /// "Memoised functions"). So only a memoised (`->>`) function's call
    /// that completed counts: one that failed has left nothing, and one
    /// still running nothing yet. The tables are the workspace's, which
    /// every run of a file, every invocation and every call it makes share,
    /// so a call made in any of them counts, once, for the workspace's life.
    /// False if the file has not run, or has no such function. Looks the
    /// tables up as such a call would, its caller the file's directory: a
    /// function whose call read `fileroot.caller` (see the evaluator's
    /// "Ambient inputs") counts only if it was called from that directory.
    /// Runs nothing and changes nothing.
    #[must_use]
    pub fn memoised(&self, file: u32, name: &str) -> bool {
        let Some((_, ((slot, _), _))) = self.top_level_cell(file, name) else {
            return false;
        };
        let Some(Value::Fn(closure)) = self.frame.get(slot.index()) else {
            return false;
        };
        let dir = self.scan.place(file).dir;
        self.memo.completed(closure, &[], dir)
    }

    /// Checks the call [`Workspace::call_composed`] makes of `functions`,
    /// top-level functions of file `file`, running nothing: whether each
    /// takes what the next returns, as the checker checks a call written in
    /// a file of `file`'s directory, `f(g())` for `["f", "g"]`. An empty
    /// `functions` calls nothing, which is well.
    ///
    /// # Errors
    ///
    /// As [`Workspace::call_composed`], but for the runtime errors: a name
    /// that is no top-level function of the file, or the diagnostics of the
    /// call's check.
    pub fn check_composed(&mut self, file: u32, functions: &[&str]) -> Result<(), InvokeError> {
        let Some((cells, at)) = self.composed(file, functions)? else {
            return Ok(());
        };
        self.lower_composed(&cells, at).map(|_| ())
    }

    /// Calls the composition of `functions`, top-level functions of file
    /// `file` as [`Workspace::top_level`] finds them, each given what the
    /// next returns and the last none, `f(g(h()))` for `["f", "g", "h"]`, as
    /// if written in a file of `file`'s directory, its caller: the file runs
    /// first, with the files it needs, unless they have run, as for
    /// [`Workspace::run`], and then the call, a call built rather than parsed
    /// that reads the functions' cells by names no source can write, as the
    /// call [`Workspace::invoke`] makes does (see the module documentation,
    /// "Top-level functions by name"), on the workspace's frame and memo
    /// tables, so a memoised function called before returns what it
    /// returned. Returns the value the first returns, with the warnings of
    /// the whole workspace; for an empty `functions`, `{}`, running nothing.
    /// Called again, the call runs again.
    ///
    /// # Errors
    ///
    /// A name that is no top-level function of the file,
    /// [`RequestError::NoFunction`], the first such, before anything runs;
    /// the runtime error of a file that runs, as [`Workspace::run`] returns
    /// it; and the diagnostics of the call's check, a function that does
    /// not take what the next returns, or its runtime error, each part of
    /// them in the call itself, which has no source, located at the first
    /// function's name, labelled "invoked here".
    pub fn call_composed(&mut self, file: u32, functions: &[&str]) -> Result<Outcome, InvokeError> {
        let Some((cells, at)) = self.composed(file, functions)? else {
            return Ok(Outcome {
                value: Value::Empty,
                warnings: self.warnings.clone(),
            });
        };
        self.run_needed(file).map_err(InvokeError::Diagnostics)?;
        let lowered = self.lower_composed(&cells, at)?;
        self.run_built(lowered, at, None)
    }

    /// The cells of `functions`, top-level functions of file `file`, as
    /// [`Workspace::top_level`] finds them, each with what a call of it
    /// needs to know of its parameters, and where the first's name is
    /// written; `None` for an empty `functions`.
    ///
    /// # Errors
    ///
    /// The first name that is no top-level function of the file.
    pub(super) fn composed(
        &self,
        file: u32,
        functions: &[&str],
    ) -> Result<Option<Composed>, InvokeError> {
        let mut cells = Vec::new();
        let mut at = None;
        for &name in functions {
            let found = self
                .top_level_cell(file, name)
                .filter(|&(_, ((_, ty), _))| self.types.fn_sig(self.types.strip(ty)).is_some());
            let Some((item, (cell, known))) = found else {
                let path = self.scan.files.get(file as usize);
                return Err(InvokeError::Request(RequestError::NoFunction {
                    file: path.map(|path| path.path.clone()).unwrap_or_default(),
                    name: name.to_owned(),
                }));
            };
            at = at.or_else(|| Some((file, self.top_level_span(file, item))));
            cells.push((cell, known));
        }
        Ok(at.map(|at| (cells, at)))
    }

    /// Checks and lowers the call of `cells`, each given what the next
    /// returns, the first's name written at `at` ([`Workspace::lower_built`]).
    ///
    /// # Errors
    ///
    /// The diagnostics of its check or lowering.
    pub(super) fn lower_composed(
        &mut self,
        cells: &[CellOf],
        at: (u32, Span),
    ) -> Result<(Ir, Vec<Diagnostic>), InvokeError> {
        let bound: Vec<BoundCell> = (0..cells.len())
            .zip(cells)
            .map(|(number, (cell, known))| {
                let name = self.scan.names.intern(&format!("<function{number}>"));
                (name, *cell, known.clone())
            })
            .collect();
        let names: Vec<Name> = bound.iter().map(|&(name, _, _)| name).collect();
        self.lower_built(&bound, |all| composition(all, &names, at.1), at)
    }

    /// The top-level field or `def` of file `file` named `name`, as
    /// [`Workspace::top_level`] finds it: its item, its cell and what a call
    /// of it needs to know of its parameters, if it is a `def`'s.
    pub(super) fn top_level_cell(&self, file: u32, name: &str) -> Option<(ItemId, CellOf)> {
        let ast = &self.scan.files.get(file as usize)?.ast;
        ast.items(ast.root()).iter().find_map(|&item| {
            let ItemKind::Field { path, .. } = ast.item(item).kind else {
                return None;
            };
            let [segment] = ast.path(path) else {
                return None;
            };
            let named = segment.name()?;
            if self.scan.names.get(named) != name {
                return None;
            }
            // A declaration a later one shadows in its own file is a leaf of
            // no tree.
            let cell = self.leaf_cell(file, item)?;
            let known = self
                .leaf_node(file, item)
                .and_then(|node| self.cell_known.get(node as usize).cloned().flatten());
            Some((item, (cell, known)))
        })
    }

    /// Where the name of the top-level item `item` of file `file` is
    /// written.
    pub(super) fn top_level_span(&self, file: u32, item: ItemId) -> Span {
        let Some(source) = self.scan.files.get(file as usize) else {
            return Span::new(0, 0);
        };
        let ast = &source.ast;
        match ast.item(item).kind {
            ItemKind::Field { path, .. } => match ast.path(path) {
                [first, .., last] => first.span.join(last.span),
                [only] => only.span,
                [] => ast.item(item).span,
            },
            _ => ast.item(item).span,
        }
    }

    /// The type of the top-level item `item` of file `file`, whose type is
    /// `ty`, as [`TopLevel::ty`] shows it: its annotation as it is written,
    /// if it has one; else, for a function expression (a `def`'s), `fn(`,
    /// its parameters, each `NAME: TYPE`, and its marker and return type as
    /// they are written, `fn(env: Vm) -> {}`; else its type as a diagnostic
    /// shows it.
    pub(super) fn shown_top_level(&self, file: u32, item: ItemId, ty: TyId) -> String {
        let names = &self.scan.names;
        let shown = || self.types.display(ty, names).to_string();
        let Some(source) = self.scan.files.get(file as usize) else {
            return shown();
        };
        let ast = &source.ast;
        let ItemKind::Field {
            ty: annotation,
            value,
            ..
        } = ast.item(item).kind
        else {
            return shown();
        };
        if let Some(annotation) = annotation {
            return ast.show_type_with(annotation, names);
        }
        let ExprKind::Fn(function) = ast.get(value).kind else {
            return shown();
        };
        let decl = ast.fn_decl(function);
        let Some(ret) = decl.ret else {
            return shown();
        };
        let params: Vec<String> = ast
            .params(decl.params)
            .iter()
            .map(|param| {
                let ty = ast.show_type_with(param.ty, names);
                format!("{}: {ty}", names.get(param.name))
            })
            .collect();
        format!(
            "fn({}{}{}",
            params.join(", "),
            decl.effect.params_end(),
            ast.show_type_with(ret, names)
        )
    }

    /// The def-type of path `def_type` and its definition of path `path`,
    /// as [`Workspace::invoke`] and [`Workspace::meta_field`] find them: the
    /// def-type, the definition's place among its definitions, and the two
    /// cells.
    ///
    /// # Errors
    ///
    /// The duplicates, with the warnings; no such def-type, or definition.
    pub(super) fn invoked(
        &self,
        def_type: &str,
        path: &str,
    ) -> Result<ResolvedDefinition, InvokeError> {
        let mut found = self.found_def_types();
        let duplicates = duplicates(&found);
        if !duplicates.is_empty() {
            let errors = merge_files(duplicates, self.warnings.clone());
            return Err(InvokeError::Diagnostics(errors));
        }
        let Some(at) = found.iter().position(|found| found.listed.path == def_type) else {
            let mut def_types: Vec<String> = found.into_iter().map(|f| f.listed.path).collect();
            def_types.sort_unstable();
            return Err(InvokeError::Request(RequestError::NoDefType {
                name: def_type.to_owned(),
                def_types,
            }));
        };
        let found = found.swap_remove(at);
        let missing = || InvokeError::Request(missing_definition(&found.listed, path));
        let definitions = &found.listed.definitions;
        let at = chosen_definition(&found.listed, path).map_err(InvokeError::Request)?;
        let (Some(definition), Some(&item)) = (definitions.get(at), found.items.get(at)) else {
            return Err(missing());
        };
        let cells = (
            self.leaf_cell(found.listed.file, found.item),
            self.leaf_cell(definition.file, item),
        );
        let (Some(def_type_cell), Some(definition_cell)) = cells else {
            // A definition a later one of another path shadows in its own
            // directory, which is in no tree: no source can reach it either.
            // The later one lies above or under it, so the duplicates have
            // failed already; kept for safety.
            return Err(missing());
        };
        Ok(ResolvedDefinition {
            def_type: found,
            definition_index: at,
            def_type_cell,
            definition_cell,
        })
    }

    /// Every def-type of the workspace's own files, with its definitions,
    /// as [`Workspace::def_types`] lists them, duplicates and all.
    pub(super) fn found_def_types(&self) -> Vec<FoundDefType> {
        let names = &self.scan.names;
        let text = |segments: &[PathSegment]| {
            segments
                .iter()
                .filter_map(PathSegment::name)
                .map(|name| names.get(name))
                .collect::<Vec<_>>()
                .join(".")
        };
        let span = |segments: &[PathSegment]| match (segments.first(), segments.last()) {
            (Some(first), Some(last)) => first.span.join(last.span),
            _ => Span::new(0, 0),
        };
        let mut found = Vec::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        // Each definition, by its namespace's key: the def-type it is made
        // with may be declared in a file after its own.
        let mut definitions: Vec<(Name, Definition, ItemId)> = Vec::new();
        let files = (0u32..).zip(&self.scan.files);
        for (index, file) in files.skip(self.scan.standard as usize) {
            let ast = &file.ast;
            for &item in ast.items(ast.root()) {
                match ast.item(item).kind {
                    ItemKind::Field {
                        path,
                        function: true,
                        ..
                    } if ast.is_def_type(item) => {
                        // One whose definitions are made with a variant is
                        // none: they are reached by references alone.
                        let variant = self
                            .leaf_cell(index, item)
                            .and_then(|(_, ty)| self.types.def_type_shape(ty))
                            .is_some_and(|shape| shape.variant.is_some());
                        if variant {
                            continue;
                        }
                        let segments = ast.path(path);
                        let path = text(segments);
                        let number = u32::try_from(item.index()).unwrap_or(u32::MAX);
                        // Its namespace's key, as the scan made it (see
                        // [Def-types](crate::workspace::declared#def-types)).
                        let key = names::namespace_key(&path, index, number);
                        by_key.insert(key, found.len());
                        let params = invocation_params(ast, item);
                        found.push(FoundDefType {
                            listed: DefType {
                                path,
                                file: index,
                                span: span(segments),
                                parameters: params
                                    .iter()
                                    .map(|param| names.get(param.name).to_owned())
                                    .collect(),
                                optional: params
                                    .iter()
                                    .map(|param| param.default.is_some())
                                    .collect(),
                                definitions: Vec::new(),
                            },
                            item,
                            written: params
                                .iter()
                                .map(|param| ast.show_type_with(param.ty, names))
                                .collect(),
                            items: Vec::new(),
                        });
                    }
                    ItemKind::DefTypeDef { def, .. } => {
                        let def = ast.def_type(def);
                        // Every definition of a workspace that checked has
                        // its def-type's key.
                        let Some(key) = def.key else {
                            continue;
                        };
                        let segments = ast.path(def.path);
                        let dir = self.scan.dirs.get(file.dir as usize);
                        let definition = Definition {
                            path: text(segments),
                            dir: dir.map(|dir| dir.path.clone()).unwrap_or_default(),
                            file: index,
                            span: span(segments),
                        };
                        definitions.push((key, definition, item));
                    }
                    _ => {}
                }
            }
        }
        for (key, definition, item) in definitions {
            let at = by_key.get(names.get(key)).copied();
            if let Some(def_type) = at.and_then(|at| found.get_mut(at)) {
                def_type.listed.definitions.push(definition);
                def_type.items.push(item);
            }
        }
        found
    }

    /// The values of `args`, each the name of an invocation parameter of the
    /// def-type `found`, whose type is `ty`, and an argument for it, for its
    /// definition of path `path`, in the order of `args`, each with its
    /// parameter's name, an argument with no text left out (see
    /// [`Workspace::arguments`]).
    ///
    /// # Errors
    ///
    /// In this order: the first invocation parameter, as it declares them,
    /// that has no default and is not one of the four types an argument can
    /// be given for, its type as it is written; for each argument in order,
    /// a name that is no invocation parameter's, a parameter already named,
    /// a parameter not of the four types, or a text that is not a value of
    /// its type, or a value that is not one, as [`Given`] shows it; then the
    /// invocation parameters that have no default given none.
    pub(super) fn given<'a>(
        &self,
        found: &FoundDefType,
        (ty, path): (TyId, &str),
        args: impl Iterator<Item = (&'a str, Argument<'a>)>,
    ) -> Result<Vec<(String, Given)>, RequestError> {
        let listed = &found.listed;
        let types = self
            .types
            .fn_sig(ty)
            .and_then(|sig| sig.params.get(2..))
            .unwrap_or_default();
        let params: Vec<Invocable> = listed
            .parameters
            .iter()
            .zip(&found.written)
            .zip(&listed.optional)
            .zip(types)
            .map(|(((name, written), &optional), &(_, ty))| Invocable {
                name,
                written,
                optional,
                scalar: self.scalar(ty),
            })
            .collect();
        let def_type = || listed.path.clone();
        let cannot_be_given = |param: &Invocable| RequestError::CannotBeGiven {
            def_type: def_type(),
            parameter: param.name.to_owned(),
            ty: param.written.to_owned(),
        };
        if let Some(param) = params
            .iter()
            .find(|param| !param.optional && param.scalar.is_none())
        {
            return Err(cannot_be_given(param));
        }
        let mut given = vec![false; params.len()];
        let mut values = Vec::new();
        for (name, argument) in args {
            let at = params.iter().position(|param| param.name == name);
            let (Some(param), Some(seen)) = (
                at.and_then(|at| params.get(at)),
                at.and_then(|at| given.get_mut(at)),
            ) else {
                return Err(RequestError::UnknownParameter {
                    def_type: def_type(),
                    parameter: name.to_owned(),
                    parameters: listed.parameters.clone(),
                });
            };
            if mem::replace(seen, true) {
                return Err(RequestError::DuplicateArgument {
                    def_type: def_type(),
                    parameter: name.to_owned(),
                });
            }
            let Some(scalar) = param.scalar else {
                return Err(cannot_be_given(param));
            };
            let (value, text) = match argument {
                Argument::Text(None) => continue,
                Argument::Text(Some(text)) => (scalar.read(text), text.to_owned()),
                Argument::Given(value) => {
                    (scalar.fits(value).then(|| value.clone()), value.to_string())
                }
            };
            let Some(value) = value else {
                return Err(RequestError::CannotParse {
                    def_type: def_type(),
                    path: path.to_owned(),
                    parameter: name.to_owned(),
                    expected: scalar.name(),
                    text,
                });
            };
            values.push((name.to_owned(), value));
        }
        let missing: Vec<String> = params
            .iter()
            .zip(&given)
            .filter(|&(param, &given)| !given && !param.optional)
            .map(|(param, _)| param.name.to_owned())
            .collect();
        if !missing.is_empty() {
            return Err(RequestError::MissingArguments {
                def_type: def_type(),
                parameters: missing,
            });
        }
        Ok(values)
    }

    /// Which of the four types an argument can be given for (see [`Given`])
    /// an invocation parameter of type `ty` is, if one: its declared type,
    /// an impure def-type's either shape (`String | Future<String>` of a
    /// bare `String`) taken off, the same type as one of them, each a
    /// subtype of the other. A union, an explicit `Future`, a record, a
    /// list or a function is none.
    pub(super) fn scalar(&self, ty: TyId) -> Option<Scalar> {
        let ty = match self.types.laziness(ty) {
            Laziness::Either => self.types.strip(ty),
            _ => ty,
        };
        Scalar::ALL.into_iter().find(|scalar| {
            self.types.is_subtype(scalar.ty(), ty) && self.types.is_subtype(ty, scalar.ty())
        })
    }

    /// The cell of the leaf the item `item` of file `index` declares, a
    /// `def` or a definition: the slot it is read from, its function's, and
    /// its type, once its file is checked; `None` if the item is a leaf of
    /// no tree, a later declaration shadowing it in its own directory.
    pub(super) fn leaf_cell(&self, index: u32, item: ItemId) -> Option<(Slot, TyId)> {
        let node = self.leaf_node(index, item)?;
        Some((
            *self.cell_slots.get(node as usize)?,
            *self.cell_types.get(node as usize)?,
        ))
    }

    /// The merged node of the leaf the item `item` of file `index`
    /// declares, as [`Workspace::leaf_cell`] finds it.
    pub(super) fn leaf_node(&self, index: u32, item: ItemId) -> Option<u32> {
        let file = self.scan.files.get(index as usize)?;
        let position = file
            .ast
            .items(file.ast.root())
            .iter()
            .position(|&at| at == item)?;
        let own = (*file.mentions.tree.item_leaf.get(position)?)?;
        file.global.get(own as usize).copied()
    }

    /// What a call of the def-type `found` needs to know of its parameters
    /// (see [`Context::known`](super::Context::known)), its defaults among them, once its file is
    /// checked: so the call that invokes one of its definitions by name
    /// gives an invocation parameter it leaves out its default.
    pub(super) fn def_type_known(&self, found: &FoundDefType) -> Option<Rc<KnownParams>> {
        let node = self.leaf_node(found.listed.file, found.item)?;
        self.cell_known.get(node as usize)?.clone()
    }

    /// Calls the def-type whose cell is `def_type` with the path of
    /// `definition`, the definition, whose cell is `definition_cell`, and the
    /// arguments `invoked` gives, their files run: the input
    /// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`, or the read of a
    /// field of the definition's meta, its names no source can write bound to
    /// the two cells, the def-type's with what a call of it needs to know of
    /// its parameters, `known`, so that the call is of a known callee and gives
    /// an invocation parameter it leaves out its default, checked from the
    /// workspace's slots, as an input of no workspace, and lowered as one
    /// placed in the definition's directory, its caller, numbered after every
    /// file (see [Def-types by
    /// name](crate::workspace::requests#def-types-by-name)); run in the root
    /// context `root`, if the host sets one.
    pub(super) fn call_definition(
        &mut self,
        (definition, invoked): (&Definition, Invocation),
        (def_type, definition_cell): ((Slot, TyId), (Slot, TyId)),
        (known, root): (Option<Rc<KnownParams>>, Option<Rc<ContextFrame>>),
    ) -> Result<Outcome, InvokeError> {
        let at = (definition.file, definition.span);
        let bound = (
            self.scan.names.intern(INVOKED_DEF_TYPE),
            self.scan.names.intern(INVOKED_DEFINITION),
        );
        let cells = [(bound.0, def_type, known), (bound.1, definition_cell, None)];
        let path = &definition.path;
        let lowered = self.lower_built(
            &cells,
            |names| invocation(names, (path, invoked), bound, at.1),
            at,
        )?;
        self.run_built(lowered, at, root)
    }

    /// Checks and lowers a call built rather than parsed, the input `build`
    /// makes, given the workspace's names, which it reads the cells of
    /// `bound` by, each by its name, one no source can write, bound in a
    /// scope of the workspace's slots as a field of the cell's slot, with its
    /// type and what a call of it needs to know of its parameters, so that a
    /// call of it is of a known callee and gives each optional parameter
    /// given none its default: checked as an input of no workspace, so that
    /// it looks nothing up in any file's view and needs no alias, and
    /// lowered as one placed in the directory of file `at.0`, its caller,
    /// numbered after every file (see [Def-types by
    /// name](crate::workspace::requests#def-types-by-name)). Returns its IR, with its own warnings,
    /// located as [`invoked_here`] locates them. It takes no cell and
    /// commits nothing, so it changes nothing any file sees. What
    /// [`Workspace::invoke`], [`Workspace::meta_field`] and
    /// [`Workspace::call_composed`] run, and [`Workspace::check_composed`]
    /// checks.
    ///
    /// # Errors
    ///
    /// The diagnostics of its check or lowering, each part of them in the
    /// call itself, which has no source, located at `at.1` of file `at.0`,
    /// labelled "invoked here" ([`invoked_here`]), with the workspace's
    /// warnings.
    pub(super) fn lower_built(
        &mut self,
        bound: &[BoundCell],
        build: impl FnOnce(Names) -> Ast,
        at: (u32, Span),
    ) -> Result<(Ir, Vec<Diagnostic>), InvokeError> {
        let call = count(self.scan.files.len());
        let ast = build(mem::take(&mut self.scan.names));
        let fields = bound
            .iter()
            .map(|(name, (slot, ty), known)| (*name, *slot, *ty, known.clone()));
        let scope = Scope::of_slots(self.slot_names.clone()).with_fields(fields);
        let place = self.scan.place(at.0);
        let lowered = Checker::with_scope(&ast, scope, &mut self.types)
            .check()
            .and_then(|mut output| {
                output.checked.resolution.set_place(place);
                session::lower_checked(&ast, output, &self.types, call)
            });
        let mut ast = ast;
        self.scan.names = ast.replace_names(Names::new());
        let (ir, output) = lowered.map_err(|errors| {
            let errors = invoked_here(errors, call, at);
            InvokeError::Diagnostics(merge_files(errors, self.warnings.clone()))
        })?;
        Ok((ir, invoked_here(output.warnings, call, at)))
    }

    /// Runs `ir`, a call [`Workspace::lower_built`] lowered, whose own
    /// warnings are `own`, on the workspace's frame and memo tables, as
    /// [`Workspace::run`] runs a file, and returns its value, with the
    /// warnings of the whole workspace and its own, in the root context
    /// `root`, if the host sets one. Run again, it runs again, a call like
    /// any other.
    ///
    /// # Errors
    ///
    /// Its runtime error, a part of it in the call itself, which has no
    /// source, located at `at.1` of file `at.0`, labelled "invoked here"
    /// ([`invoked_here`]), with the warnings.
    pub(super) fn run_built(
        &mut self,
        (mut ir, own): (Ir, Vec<Diagnostic>),
        at: (u32, Span),
        root: Option<Rc<ContextFrame>>,
    ) -> Result<Outcome, InvokeError> {
        let call = count(self.scan.files.len());
        // Numbered after every file, its slots end after theirs; kept
        // nonetheless, as a file's are (see [`Ir::reserve_frame`]).
        ir.reserve_frame(count(self.frame.len()));
        let context = (
            &mut self.memo,
            Some(&self.exec_result),
            &self.scan.dir_texts[..],
            &self.temp_dirs,
        );
        let mut warnings = self.warnings.clone();
        warnings.extend(own);
        match session::run_input_in(ir, &mut self.frame, context, root) {
            Ok(value) => Ok(Outcome { value, warnings }),
            Err(error) => {
                let errors = invoked_here(vec![error], call, at);
                Err(InvokeError::Diagnostics(merge_files(errors, warnings)))
            }
        }
    }
}

/// A cell a call built rather than parsed reads (see
/// [`Workspace::lower_built`]): the name, no source can write it, it is read
/// by, its slot and type, and what a call of it needs to know of its
/// parameters, if it is a `def`'s.
pub(super) type BoundCell = (Name, (Slot, TyId), Option<Rc<KnownParams>>);

/// A top-level function's cell, its slot and type, with what a call of it
/// needs to know of its parameters, if it is a `def`'s (see
/// [`Workspace::top_level`]).
pub(super) type CellOf = ((Slot, TyId), Option<Rc<KnownParams>>);

/// The functions of a composed call (see [`Workspace::call_composed`]): the
/// cell of each, in order, and where the first's name is written, its file
/// and span.
pub(super) type Composed = (Vec<CellOf>, (u32, Span));

/// A top-level field or `def` of a file of a workspace, as
/// [`Workspace::top_level`] finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopLevel {
    /// Its type as it is written: its annotation, or for a `def`, `fn(`,
    /// its parameters, each `NAME: TYPE`, and its marker and return type,
    /// `fn() ->> Vm`; or, for a field with neither, as a diagnostic shows
    /// it.
    pub ty: String,
    /// If its type is a function type, what the function may do, as it is
    /// declared, pure (`:`), memoised (`->>`) or impure (`->`), and how many
    /// parameters it takes.
    pub function: Option<(ast::Effect, usize)>,
}

/// The tree of the input of a composed call (see
/// [`Workspace::call_composed`]), its names `names`: `<function0>(
/// <function1>( ... ))`, its result, each name one of `functions`, in
/// order, the last called with no argument and each other with what the
/// next returns, each node at `span`. An empty `functions` is the empty
/// block.
pub(super) fn composition(names: Names, functions: &[Name], span: Span) -> Ast {
    let mut builder = AstBuilder::with_capacity(2 * functions.len() + 2);
    let mut inner = None;
    for &function in functions.iter().rev() {
        let callee = builder.alloc(Expr {
            kind: ExprKind::Name(function),
            span,
        });
        let arguments: Vec<Arg> = inner
            .into_iter()
            .map(|value| Arg {
                kind: ArgKind::Positional,
                value,
            })
            .collect();
        let args = builder.push_args(&arguments);
        inner = Some(builder.alloc(Expr {
            kind: ExprKind::Call { callee, args },
            span,
        }));
    }
    let items = match inner {
        Some(call) => vec![builder.alloc_item(Item {
            kind: ItemKind::Result(call),
            span,
        })],
        None => Vec::new(),
    };
    let root = builder.push_item_list(&items);
    builder.finish(root, names, Strings::new())
}

/// The index among the definitions made with the def-type `listed` of the
/// one `asked` names (see the module documentation, "Def-types by name"):
/// a path qualified by a directory, `dir/sub/name` or `/name` for the root,
/// names the definition of that path in that directory; a path alone, the
/// definition of that path, if it is the only one.
///
/// # Errors
///
/// [`missing_definition`]'s, if none is of the path, in the directory if
/// one is given; and if a path alone is several definitions' path, in
/// directories that cannot see each other, [`RequestError::Ambiguous`].
pub(super) fn chosen_definition(listed: &DefType, asked: &str) -> Result<usize, RequestError> {
    let (dir, path) = split_qualified(asked);
    let mut matching = (0usize..)
        .zip(&listed.definitions)
        .filter(|(_, definition)| {
            definition.path == path && dir.is_none_or(|dir| definition.dir == dir)
        });
    let first = matching.next();
    let others: Vec<usize> = matching.map(|(at, _)| at).collect();
    match first {
        None => Err(missing_definition(listed, asked)),
        Some((at, _)) if others.is_empty() => Ok(at),
        Some((at, _)) => {
            let mut candidates: Vec<String> = std::iter::once(at)
                .chain(others)
                .filter_map(|at| listed.definitions.get(at))
                .map(Definition::qualified)
                .collect();
            candidates.sort_unstable();
            Err(RequestError::Ambiguous {
                def_type: listed.path.clone(),
                path: path.to_owned(),
                candidates,
            })
        }
    }
}

/// The directory and the path of `asked`, a definition's path as asked for:
/// the text before its last `/`, without a leading `/` (so `""`, the root,
/// for `/name`), and the text after it; `None` and `asked` for a path with
/// no `/`, which a definition's path never has.
pub(super) fn split_qualified(asked: &str) -> (Option<&str>, &str) {
    match asked.rsplit_once('/') {
        Some((dir, path)) => (Some(dir.strip_prefix('/').unwrap_or(dir)), path),
        None => (None, asked),
    }
}

/// The names the definitions made with the def-type `listed` are asked for
/// by, sorted: each path alone, but those several definitions have, each
/// qualified by its directory ([`Definition::qualified`]).
pub(super) fn definition_names(listed: &DefType) -> Vec<String> {
    let definitions = &listed.definitions;
    let shared = |definition: &Definition| {
        definitions
            .iter()
            .filter(|other| other.path == definition.path)
            .count()
            > 1
    };
    let mut names: Vec<String> = definitions
        .iter()
        .map(|definition| {
            if shared(definition) {
                definition.qualified()
            } else {
                definition.path.clone()
            }
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The error of `asked`, which names no definition made with the def-type
/// `listed` (see [`chosen_definition`]): "`dev.peios` is not a `package`
/// definition" if it is above some of them, in its directory if it gives
/// one, else "`package` has no definition `dev.peios.nope`".
pub(super) fn missing_definition(listed: &DefType, asked: &str) -> RequestError {
    let definitions = definition_names(listed);
    let (dir, path) = split_qualified(asked);
    let above = listed.definitions.iter().any(|definition| {
        lies_under(&definition.path, path) && dir.is_none_or(|dir| definition.dir == dir)
    });
    let (def_type, path) = (listed.path.clone(), asked.to_owned());
    if above {
        RequestError::NotADefinition {
            def_type,
            path,
            definitions,
        }
    } else {
        RequestError::NoDefinition {
            def_type,
            path,
            definitions,
        }
    }
}

/// The duplicates among `found` (see [`Workspace::def_types`]): an error for
/// each path of which two def-types or more are declared, for each path of
/// which a def-type has two definitions or more in directories that see
/// each other ([`sees`]), and for each definition that lies under another
/// of its def-type in such a directory ([`overlaps`]), in path then source
/// order of the first, or of the definition below.
pub(super) fn duplicates(found: &[FoundDefType]) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let declared = found.iter().map(|found| {
        (
            found.listed.path.as_str(),
            found.listed.file,
            found.listed.span,
        )
    });
    for group in groups(declared) {
        errors.extend(duplicate(
            &group,
            |path, times| format!("def-type `{path}` is declared {times} times in the workspace"),
            "also declared here",
            "a def-type invoked by name must be the only one of its name in the workspace",
        ));
    }
    for found in found {
        let definitions = &found.listed.definitions;
        let defined = definitions
            .iter()
            .map(|definition| (definition.path.as_str(), definition.file, definition.span));
        let def_type = &found.listed.path;
        for group in groups(defined) {
            // The group's definitions, by where they are, gathered into those
            // whose directories see each other, one through another.
            let dir_of = |(_, file, span): &Declared| {
                definitions
                    .iter()
                    .find(|definition| definition.file == *file && definition.span == *span)
                    .map_or("", |definition| definition.dir.as_str())
            };
            for chain in chains(&group, |a, b| sees(dir_of(a), dir_of(b))) {
                errors.extend(duplicate(
                    &chain,
                    |path, times| {
                        format!(
                            "`{def_type} {path}` is defined {times} times in directories that \
                             see each other"
                        )
                    },
                    "also defined here",
                    "a definition invoked by name must be the only one of its path in its \
                     directory, the directories above it and those below it",
                ));
            }
        }
        errors.extend(overlaps(&found.listed));
    }
    errors.sort_by_key(|error| (error.input, error.span.start));
    errors
}

/// The errors of the definitions made with the def-type `listed` that lie
/// under others of it (see [`Workspace::def_types`]): for each definition
/// whose path is below another's, `app.core` below `app`, "`package
/// app.core` lies under another `package` definition", at it, labelled
/// "`app` is defined here" at each definition above it, in the order of
/// `listed`'s.
pub(super) fn overlaps(listed: &DefType) -> Vec<Diagnostic> {
    let definitions = &listed.definitions;
    let def_type = &listed.path;
    let mut errors = Vec::new();
    for definition in definitions {
        let mut above = definitions
            .iter()
            .filter(|other| {
                lies_under(&definition.path, &other.path) && sees(&definition.dir, &other.dir)
            })
            .peekable();
        if above.peek().is_none() {
            continue;
        }
        let path = &definition.path;
        let mut error = Diagnostic::error(
            format!("`{def_type} {path}` lies under another `{def_type}` definition"),
            definition.span,
        )
        .in_input(definition.file)
        .with_note(
            "a definition invoked by name must not lie under another in its directory, the \
             directories above it or those below it",
        );
        for other in above {
            let text = format!("`{}` is defined here", other.path);
            error = error.with_context_in(other.file, other.span, text);
        }
        errors.push(error);
    }
    errors
}

/// Whether the directories of root-relative paths `a` and `b` see each
/// other's definitions, one through another: whether they are one, or one
/// is above the other (the root, `""`, is above every directory). Two
/// directories neither of which is above the other, siblings or cousins,
/// see nothing of each other's.
pub(super) fn sees(a: &str, b: &str) -> bool {
    let above = |upper: &str, lower: &str| {
        upper.is_empty()
            || lower
                .strip_prefix(upper)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    a == b || above(a, b) || above(b, a)
}

/// `group`, the declarations of one path, gathered into the chains of those
/// `related`, one through others: each chain in the order of `group`, the
/// chains in the order of their first.
pub(super) fn chains<'a>(
    group: &[Declared<'a>],
    related: impl Fn(&Declared<'a>, &Declared<'a>) -> bool,
) -> Vec<Vec<Declared<'a>>> {
    let mut chains: Vec<Vec<Declared<'a>>> = Vec::new();
    for &entry in group {
        let (joined, apart): (Vec<_>, Vec<_>) = chains
            .into_iter()
            .partition(|chain| chain.iter().any(|other| related(other, &entry)));
        let mut merged: Vec<Declared<'a>> = joined.into_iter().flatten().collect();
        merged.push(entry);
        chains = apart;
        chains.push(merged);
    }
    chains.sort_by_key(|chain| chain.first().map(|&(_, file, span)| (file, span.start)));
    for chain in &mut chains {
        chain.sort_by_key(|&(_, file, span)| (file, span.start));
    }
    chains
}

/// Whether the path `path` lies under the path `above`, its segments
/// following all of `above`'s: `app.core` under `app`, but not `apple`.
pub(super) fn lies_under(path: &str, above: &str) -> bool {
    path.strip_prefix(above)
        .is_some_and(|rest| rest.starts_with('.'))
}

/// A declaration of a path by [`duplicates`]: its path, its file and where.
pub(super) type Declared<'a> = (&'a str, u32, Span);

/// `declared`, grouped by path, each group in the order of `declared`, the
/// groups in the order of their first.
pub(super) fn groups<'a>(declared: impl Iterator<Item = Declared<'a>>) -> Vec<Vec<Declared<'a>>> {
    let mut groups: Vec<Vec<Declared<'a>>> = Vec::new();
    let mut by_path: HashMap<&str, usize> = HashMap::new();
    for entry in declared {
        let at = *by_path.entry(entry.0).or_insert(groups.len());
        match groups.get_mut(at) {
            Some(group) => group.push(entry),
            None => groups.push(vec![entry]),
        }
    }
    groups
}

/// The error of `group`, the declarations of one path, if there are two or
/// more: `message` of the path and their number, at the first, labelled
/// `label` at each other, with `note`.
pub(super) fn duplicate(
    group: &[Declared],
    message: impl Fn(&str, usize) -> String,
    label: &str,
    note: &str,
) -> Option<Diagnostic> {
    let ([(path, file, span), rest @ ..], true) = (group, group.len() > 1) else {
        return None;
    };
    let mut error = Diagnostic::error(message(path, group.len()), *span)
        .in_input(*file)
        .with_note(note);
    for &(_, file, span) in rest {
        error = error.with_context_in(file, span, label);
    }
    Some(error)
}

/// The invocation parameters of the def-type whose `def` is the item `item`
/// of `ast`: its parameters after its second, the definition's path's and
/// the definition's, as it declares them; none if it declares none, or if
/// the item is no `def` of a function.
pub(super) fn invocation_params(ast: &Ast, item: ItemId) -> &[Param] {
    let ItemKind::Field { value, .. } = ast.item(item).kind else {
        return &[];
    };
    let ExprKind::Fn(function) = ast.get(value).kind else {
        return &[];
    };
    ast.params(ast.fn_decl(function).params)
        .get(2..)
        .unwrap_or_default()
}

/// The tree of the input of the call that invokes a definition by name
/// (see [Def-types by name](crate::workspace::requests#def-types-by-name)), its names `names`:
/// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`, its result, PATH
/// `path`, each named argument one of `args`, in order, its value a literal
/// of its kind, the two names those of `bound`, each node at `span`.
pub(super) fn invocation(
    mut names: Names,
    (path, invoked): (&str, Invocation),
    bound: (Name, Name),
    span: Span,
) -> Ast {
    let args = match invoked {
        Invocation::Call(args) => args,
        Invocation::MetaField(field) => {
            return meta_read(names.intern(field), names, bound.1, span);
        }
    };
    let mut builder = AstBuilder::with_capacity(4 + args.len());
    let mut strings = Strings::new();
    let callee = builder.alloc(Expr {
        kind: ExprKind::Name(bound.0),
        span,
    });
    let name = builder.alloc(Expr {
        kind: ExprKind::Str(strings.push(path)),
        span,
    });
    let definition = builder.alloc(Expr {
        kind: ExprKind::Name(bound.1),
        span,
    });
    // The path and the definition positionally, then the arguments by name.
    let mut arguments = vec![
        Arg {
            kind: ArgKind::Positional,
            value: name,
        },
        Arg {
            kind: ArgKind::Positional,
            value: definition,
        },
    ];
    for (name, given) in args {
        let kind = match given {
            Given::Str(text) => ExprKind::Str(strings.push(text)),
            Given::Int(value) => ExprKind::Int(*value),
            Given::Float(value) => ExprKind::Float(*value),
            Given::Bool(value) => ExprKind::Bool(*value),
            Given::Quantity(Quantity::Duration, amount) => ExprKind::Duration(*amount),
            Given::Quantity(Quantity::Size, amount) => ExprKind::Size(*amount),
        };
        let value = builder.alloc(Expr { kind, span });
        arguments.push(Arg {
            kind: ArgKind::Named {
                name: names.intern(name),
                span,
                shorthand: false,
            },
            value,
        });
    }
    let args = builder.push_args(&arguments);
    let call = builder.alloc(Expr {
        kind: ExprKind::Call { callee, args },
        span,
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Result(call),
        span,
    });
    let root = builder.push_item_list(&[item]);
    builder.finish(root, names, strings)
}

/// What the input [`Workspace::call_definition`] runs does with the
/// definition: call its def-type with it and these arguments, or read this
/// field of its meta.
#[derive(Clone, Copy)]
pub(super) enum Invocation<'a> {
    /// `<def-type>("PATH", <definition>, NAME = VALUE, ...)`.
    Call(&'a [(String, Given)]),
    /// `<definition>.FIELD`.
    MetaField(&'a str),
}

/// The tree of the input that reads the field `field` of a definition's
/// meta (see [`Workspace::meta_field`]), its names `names`:
/// `<definition>.FIELD`, its result, the name that of `definition`, each
/// node at `span`.
pub(super) fn meta_read(field: Name, names: Names, definition: Name, span: Span) -> Ast {
    let mut builder = AstBuilder::with_capacity(2);
    let read = builder.alloc(Expr {
        kind: ExprKind::Name(definition),
        span,
    });
    let field = builder.alloc(Expr {
        kind: ExprKind::Field {
            expr: read,
            name: field,
        },
        span,
    });
    let item = builder.alloc_item(Item {
        kind: ItemKind::Result(field),
        span,
    });
    let root = builder.push_item_list(&[item]);
    builder.finish(root, names, Strings::new())
}

/// `diagnostics` of the call that invokes a definition by name, input
/// number `call`, as its caller can show them: the call has no source, so
/// every diagnostic and label located in it, of input `call` or of no input,
/// is located instead at `at`, the definition's path in its file, labelled
/// "invoked here" (see [Def-types by name](crate::workspace::requests#def-types-by-name)).
pub(super) fn invoked_here(
    mut diagnostics: Vec<Diagnostic>,
    call: u32,
    (file, span): (u32, Span),
) -> Vec<Diagnostic> {
    const INVOKED: &str = "invoked here";
    for diagnostic in &mut diagnostics {
        let own = diagnostic.input.is_none_or(|input| input == call);
        for label in &mut diagnostic.labels {
            if label.input.map_or(own, |input| input == call) {
                label.input = Some(file);
                label.span = span;
                INVOKED.clone_into(&mut label.text);
            }
        }
        if own {
            diagnostic.input = Some(file);
            diagnostic.span = span;
            diagnostic.label = Some(INVOKED.into());
        }
    }
    diagnostics
}

/// `diagnostics` of the file `index` of a scan of one file beside the
/// standard library, as the diagnostics of the input being run name it: by
/// no input, as a label in it does, a label in a standard file keeping that
/// file's number, which is its input number in a session too.
#[must_use]
pub fn being_run(mut diagnostics: Vec<Diagnostic>, index: u32) -> Vec<Diagnostic> {
    for diagnostic in &mut diagnostics {
        if diagnostic.input == Some(index) {
            diagnostic.input = None;
        }
        for label in &mut diagnostic.labels {
            if label.input == Some(index) {
                label.input = None;
            }
        }
    }
    diagnostics
}
