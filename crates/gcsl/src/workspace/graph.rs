//! Invocations as units of an evaluation in graph mode.
//!
//! # Prepared calls
//!
//! A host evaluates a target, an invocation of a definition, in graph mode
//! as a unit that runs, stopping where it waits and taking up there, until
//! it is done (see [Graph mode](crate::eval::graph#graph-mode)). Its call is lowered once
//! ([`Workspace::prepare_call`]), as [`Workspace::invoke_with`] lowers it,
//! checked and placed alike, so that every attempt runs the same code,
//! whose closures' keys are then the same; and it is numbered, after every
//! file, as a unit of its own. Each attempt
//! ([`Workspace::run_prepared`]) runs, in graph mode, the files the call
//! needs, then the call, in the root context the host set: a file or the
//! call that stops for a step's result is [`Progress::Paused`], to run
//! again once the host has given it.

use std::mem;
use std::rc::Rc;

use crate::diagnostic::{Diagnostic, Mark};
use crate::eval::graph::{
    EffectJob, Graph, GraphHost, GraphRun, NodeId, NodeKind, RunExtras, STEP_UNITS, StepCall,
    StepReads, run_step_call,
};
use crate::ir::Ir;
use crate::source::Span;
use crate::session;
use crate::value::ContextFrame;

use super::invoke::{FieldCell, Invocation, invoked_here, missing_definition};
use super::{Argument, Given, InvokeError, Progress, Workspace, count, merge_files};

/// A root context a host sets for a prepared call from a top-level field of
/// the workspace ([`Workspace::prepare_call_with`]): the context `context`,
/// set to the value of the top-level field `field` of file `file`, read
/// through the fields `path`, `pekitconf.toolchains` for `toolchains`.
#[derive(Clone, Copy, Debug)]
pub struct RootField<'a> {
    /// The context's name.
    pub context: &'a str,
    /// The file, by its number.
    pub file: u32,
    /// Its top-level field.
    pub field: &'a str,
    /// The fields read of the field's value, in order.
    pub path: &'a [&'a str],
}

/// A top-level function a prepared call gives its invocation to
/// ([`Workspace::prepare_call_wrapped`]): the function `function` of file
/// `file`, given the invocation's value and then `args`.
#[derive(Clone, Copy, Debug)]
pub struct RootWrap<'a> {
    /// The file, by its number.
    pub file: u32,
    /// Its top-level function's name.
    pub function: &'a str,
    /// The arguments it is given after the invocation's value, in order.
    pub args: &'a [Given],
}

/// An invocation of a definition, prepared to run in graph mode, as many
/// times as it takes (see [Prepared calls](#prepared-calls)).
#[derive(Debug)]
pub struct PreparedCall {
    /// The call's code, lowered once.
    ir: Rc<Ir>,
    /// Its own warnings.
    warnings: Vec<Diagnostic>,
    /// Where it is located: the definition's file and span.
    at: (u32, Span),
    /// The root context the host set for it.
    root: Option<Rc<ContextFrame>>,
    /// The files it needs, which run before it: the definition's, its
    /// def-type's, and each whose field it sets a root context from.
    files: Vec<u32>,
    /// Its number as a unit, after every file's.
    unit: u32,
    /// Where its own top-level slots begin: after every file's, at the
    /// same place as every other prepared call's, which is why a suspended
    /// run keeps them (see [Suspension](crate::eval::machine#suspension)).
    own: usize,
    /// The slot of its definition's cell, whose closure it binds to the
    /// context of [`DEFINITION_CONTEXT`](crate::lower::DEFINITION_CONTEXT)
    /// around its call, as a reference's call does.
    definition: crate::check::Slot,
    /// The root context it runs in, `root` with its definition bound, made
    /// once its definition's file has run, the first time it runs, and kept,
    /// so that a frame is one frame however often it runs.
    running: std::cell::OnceCell<Option<Rc<ContextFrame>>>,
}

impl PreparedCall {
    /// Its number as a unit, which the nodes it makes carry.
    #[must_use]
    pub const fn unit(&self) -> u32 {
        self.unit
    }

    /// The files it needs, by number, which run before it: its
    /// definition's, its def-type's, and each whose field it sets a root
    /// context from. What each of them needs in turn is
    /// [`Workspace::files_needed`]'s.
    #[must_use]
    pub fn files(&self) -> &[u32] {
        &self.files
    }
}

impl Workspace {
    /// Runs the step that the deferred node `id` of `graph` is as a unit of
    /// its own in graph mode, numbered [`STEP_UNITS`] plus the node's number,
    /// as the host's engine runs a hermetic step (see pekit2's
    /// `design/engine.md`, E1): its callee's body, given its arguments, in the
    /// context it was made in, its effects nodes of its unit, which the host
    /// runs ([`Workspace::effect_job`]), deferred calls in it as `host`
    /// says, and its own `#[retry]` and `#[timeout]` not applied, which are
    /// the host's. [`Progress::Paused`] if it needs a result the host has yet
    /// to give, to take up where it stopped once it is given. With it, what
    /// the run read of the context the step was made in ([`StepReads`]): a
    /// run that ends gives every read the step made, across its runs, so a
    /// host keys the step by those; each first read is told
    /// to `host`'s effects as it is made ([`Effects::context_read`](crate::eval::Effects::context_read)).
    ///
    /// # Errors
    ///
    /// The step's failure, located as a call's is: where it was raised, or
    /// at the call for a `#[track_caller]` callee's own; and an internal error
    /// if the node is no deferred call's.
    pub fn run_step(
        &mut self,
        graph: &mut Graph,
        id: NodeId,
        host: GraphHost<'_>,
    ) -> Result<(Progress, StepReads), Diagnostic> {
        let call = graph.node(id).and_then(StepCall::of);
        let Some(call) = call else {
            return Err(internal("a step that is no deferred call's"));
        };
        let unit = STEP_UNITS.saturating_add(id.number());
        let run = GraphRun::new(mem::take(graph), unit, host.defers)
            .watching(call.context(), self.context_texts())
            .resuming(self.suspended.remove(&unit));
        let names = std::cell::RefCell::new(std::mem::take(&mut self.scan.names));
        let extras = RunExtras {
            exec_result: Some(&self.exec_result),
            dirs: &self.scan.dir_texts[..],
            effects: host.effects,
            names: Some(&names),
            profile: self.profile.as_ref(),
        };
        let outcome = run_step_call(&call, (&mut self.frame, &mut self.memo), extras, &run);
        self.scan.names = names.into_inner();
        if let Some(suspended) = run.take_suspended() {
            self.suspended.insert(unit, suspended);
        }
        let reads = run.reads();
        let (left, paused) = run.finish();
        *graph = left;
        match outcome {
            Ok(value) => Ok((Progress::Done(value), reads)),
            Err(error) if error.mark == Mark::PENDING => match Progress::stopped(&paused) {
                Some(progress) => Ok((progress, reads)),
                None => Err(internal("a step stopped for no step")),
            },
            Err(error) => Err(error),
        }
    }

    /// The job of the effect node `id` of `graph`: its built-in and its
    /// arguments as plain data, which the host runs on any thread
    /// ([`EffectJob::run`]) and gives back ([`Graph::resolve`],
    /// [`Workspace::from_plain`]).
    ///
    /// # Errors
    ///
    /// What of its arguments is no data, or that `graph` has no such effect
    /// node.
    pub fn effect_job(&self, graph: &Graph, id: NodeId) -> Result<EffectJob, String> {
        let node = graph.node(id).ok_or("an effect the graph lacks")?;
        let NodeKind::Effect(builtin) = node.kind() else {
            return Err("a step that is no effect".to_owned());
        };
        let args = node
            .args()
            .iter()
            .map(|value| self.to_plain(value))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EffectJob::new(builtin, args, node.span()))
    }

    /// Prepares the invocation of the definition of path `path` of the
    /// def-type of path `def_type` with the arguments `args`, in the root
    /// context `contexts`, as [`Workspace::invoke_with`] reads and checks
    /// them, to run in graph mode (see [Prepared calls](#prepared-calls)):
    /// lowered, numbered, nothing run.
    ///
    /// # Errors
    ///
    /// Those of [`Workspace::invoke_with`] before it runs anything, and the
    /// call's check's.
    pub fn prepare_call(
        &mut self,
        call: (&str, &str),
        args: &[(String, Given)],
        contexts: &[(String, crate::Value)],
    ) -> Result<PreparedCall, InvokeError> {
        self.prepare_call_with(call, args, contexts, &[])
    }

    /// [`Workspace::prepare_call`], with root contexts set from top-level
    /// fields of the workspace too, as if the call were written in `with {
    /// CONTEXT = FIELD.PATH } { ... }`: each field read in the call's own
    /// unit, once its file has run in it, checked against its context's
    /// type as a `with`'s value is, as pekit sets `^toolchains` from
    /// `workspace.gcsl`'s `pekitconf.toolchains` (see [Prepared
    /// calls](#prepared-calls)).
    ///
    /// # Errors
    ///
    /// Those of [`Workspace::prepare_call`], and a field that is no
    /// top-level field of its file, [`RequestError::NoField`](super::RequestError::NoField).
    pub fn prepare_call_with(
        &mut self,
        call: (&str, &str),
        args: &[(String, Given)],
        contexts: &[(String, crate::Value)],
        fields: &[RootField<'_>],
    ) -> Result<PreparedCall, InvokeError> {
        self.prepare_call_wrapped(call, args, contexts, (fields, None))
    }

    /// [`Workspace::prepare_call_with`], the invocation given, if `wrap`
    /// says so, to a top-level function of a file as its first argument, the
    /// wrapper's others after it, as if the call were written
    /// `FUNCTION(DT PATH(...), ARGS...)`, in the root contexts too: as pekit
    /// packs a package, `packaging.pack(package NAME())`.
    ///
    /// # Errors
    ///
    /// Those of [`Workspace::prepare_call_with`], and a function that is no
    /// top-level field of its file, [`RequestError::NoField`](super::RequestError::NoField), or
    /// one the invocation's value does not fit, its check's.
    pub fn prepare_call_wrapped(
        &mut self,
        (def_type, path): (&str, &str),
        args: &[(String, Given)],
        contexts: &[(String, crate::Value)],
        (fields, wrap): (&[RootField<'_>], Option<RootWrap<'_>>),
    ) -> Result<PreparedCall, InvokeError> {
        let no_field = |workspace: &Self, file: u32, name: &str| {
            let file = workspace
                .scan
                .files
                .get(file as usize)
                .map(|file| file.path.clone())
                .unwrap_or_default();
            InvokeError::Request(super::RequestError::NoField {
                file,
                name: name.to_owned(),
            })
        };
        let wrapper = match wrap {
            None => None,
            Some(wrap) => {
                let Some((_, (cell, known))) = self.top_level_cell(wrap.file, wrap.function) else {
                    return Err(no_field(self, wrap.file, wrap.function));
                };
                Some((
                    (self.scan.names.intern("<root wrapper>"), cell, known),
                    wrap.args,
                ))
            }
        };
        let mut cells = Vec::with_capacity(fields.len());
        for (number, field) in fields.iter().enumerate() {
            let Some((_, (cell, _))) = self.top_level_cell(field.file, field.field) else {
                return Err(no_field(self, field.file, field.field));
            };
            cells.push(FieldCell {
                context: self.scan.names.intern(field.context),
                bound: self.scan.names.intern(&format!("<root field {number}>")),
                cell,
                path: field
                    .path
                    .iter()
                    .map(|name| self.scan.names.intern(name))
                    .collect(),
            });
        }
        let root = self.root_frame(contexts).map_err(InvokeError::Request)?;
        let resolved = self.invoked(def_type, path)?;
        let given = args
            .iter()
            .map(|(name, value)| (name.as_str(), Argument::Given(value)));
        let args = self
            .given(&resolved.def_type, (resolved.def_type_cell.1, path), given)
            .map_err(InvokeError::Request)?;
        let found = &resolved.def_type;
        let Some(definition) = found.listed.definitions.get(resolved.definition_index) else {
            return Err(InvokeError::Request(missing_definition(
                &found.listed,
                path,
            )));
        };
        let definition = definition.clone();
        let mut files = vec![definition.file, found.listed.file];
        files.extend(fields.iter().map(|field| field.file));
        files.extend(wrap.map(|wrap| wrap.file));
        let known = self.def_type_known(found);
        let (mut ir, warnings) = self.lower_definition_call(
            (&definition, Invocation::Call(&args)),
            (resolved.def_type_cell, resolved.definition_cell),
            (known, root.as_ref()),
            (&cells, wrapper),
        )?;
        ir.reserve_frame(count(self.frame.len()));
        let unit = count(self.scan.files.len()).saturating_add(self.calls_prepared);
        self.calls_prepared = self.calls_prepared.saturating_add(1);
        Ok(PreparedCall {
            ir: Rc::new(ir),
            warnings,
            at: (definition.file, definition.span),
            root,
            files,
            unit,
            // Checked in a scope of every slot the workspace has named, its
            // own come after them.
            own: self.slot_names.len(),
            definition: resolved.definition_cell.0,
            running: std::cell::OnceCell::new(),
        })
    }

    /// How many memoised calls have ended in the workspace's runs, returned
    /// or failed, or been given up for a run elsewhere (see
    /// [`MemoTables::ended`](crate::eval::MemoTables::ended)): by it a host
    /// tells a unit's run that ended a call another unit waits on, and so
    /// progressed though it stopped again, from units stuck waiting for
    /// each other (see [Graph mode](crate::eval::graph#graph-mode)).
    #[must_use]
    pub fn memo_ended(&self) -> u64 {
        self.memo.ended()
    }

    /// Runs `call`, a prepared invocation, in graph mode, against `graph`,
    /// `host` saying which deferring calls are steps and how other effects
    /// reach the system (see
    /// [Prepared calls](#prepared-calls)): the files it needs, then the call.
    /// [`Progress::Paused`] if one of them stopped for a step's result; else
    /// [`Progress::Done`] with the call's value, whose holes [`Graph::holes`]
    /// finds.
    ///
    /// # Errors
    ///
    /// The runtime error of a file or of the call, as
    /// [`Workspace::invoke_with`] gives it.
    pub fn run_prepared(
        &mut self,
        call: &PreparedCall,
        graph: &mut Graph,
        host: GraphHost<'_>,
    ) -> Result<Progress, InvokeError> {
        for &file in &call.files {
            if let Some(progress) = self
                .run_files(file, Some((&mut *graph, host)))
                .map_err(InvokeError::Diagnostics)?
            {
                return Ok(progress);
            }
        }
        // Its root context, its definition bound as a reference's call binds
        // it, so that every node it makes knows the definition it is of,
        // whichever other targets make the same calls.
        let root = call
            .running
            .get_or_init(|| {
                let key = self.scan.names.lookup(crate::lower::DEFINITION_CONTEXT);
                match (key, self.frame.get(call.definition.index())) {
                    (Some(key), Some(definition @ crate::Value::Fn(_))) => {
                        Some(Rc::new(ContextFrame {
                            id: self.memo.new_context_frame(),
                            binds: Box::new([(key, definition.clone())]),
                            outer: call.root.clone(),
                        }))
                    }
                    _ => call.root.clone(),
                }
            })
            .clone();
        let names = std::cell::RefCell::new(std::mem::take(&mut self.scan.names));
        let context = (
            &mut self.memo,
            RunExtras {
                exec_result: Some(&self.exec_result),
                dirs: &self.scan.dir_texts[..],
                effects: host.effects,
                names: Some(&names),
                profile: self.profile.as_ref(),
            },
        );
        let run = GraphRun::new(mem::take(graph), call.unit, host.defers)
            .owning(call.own)
            .resuming(self.suspended.remove(&call.unit));
        let outcome = session::run_input_in(&call.ir, &mut self.frame, context, (root, Some(&run)));
        self.scan.names = names.into_inner();
        if let Some(suspended) = run.take_suspended() {
            self.suspended.insert(call.unit, suspended);
        }
        let (left, paused) = run.finish();
        *graph = left;
        match outcome {
            Ok(value) => Ok(Progress::Done(value)),
            Err(error) if error.mark == Mark::PENDING => match Progress::stopped(&paused) {
                Some(progress) => Ok(progress),
                None => Err(InvokeError::Diagnostics(vec![
                    crate::diagnostic::InternalError {
                        message: "a call stopped for no step",
                        span: error.span,
                    }
                    .into(),
                ])),
            },
            Err(error) => {
                let call_input = count(self.scan.files.len());
                let errors = invoked_here(vec![error], call_input, call.at);
                let mut warnings = self.warnings.clone();
                warnings.extend(call.warnings.iter().cloned());
                Err(InvokeError::Diagnostics(merge_files(errors, warnings)))
            }
        }
    }
}

/// The internal error `message`, of no source.
fn internal(message: &'static str) -> Diagnostic {
    crate::diagnostic::InternalError {
        message,
        span: Span::new(0, 0),
    }
    .into()
}
