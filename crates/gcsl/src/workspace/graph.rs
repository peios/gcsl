//! Invocations as units of an evaluation in graph mode.
//!
//! # Prepared calls
//!
//! A host evaluates a target, an invocation of a definition, in graph mode
//! as a unit that runs again from its start until it is done (see [Graph
//! mode](crate::eval::graph#graph-mode)). Its call is lowered once
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
use crate::eval::graph::{Defers, Graph, GraphRun};
use crate::eval::{Direct, Effects};
use crate::ir::Ir;
use crate::source::Span;
use crate::session;
use crate::value::ContextFrame;

use super::invoke::{Invocation, invoked_here, missing_definition};
use super::{Argument, Given, InvokeError, Progress, Workspace, count, merge_files};

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
    /// The files it needs, which run before it: the definition's and its
    /// def-type's.
    files: [u32; 2],
    /// Its number as a unit, after every file's.
    unit: u32,
}

impl PreparedCall {
    /// Its number as a unit, which the nodes it makes carry.
    #[must_use]
    pub const fn unit(&self) -> u32 {
        self.unit
    }
}

impl Workspace {
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
        (def_type, path): (&str, &str),
        args: &[(String, Given)],
        contexts: &[(String, crate::Value)],
    ) -> Result<PreparedCall, InvokeError> {
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
        let files = [definition.file, found.listed.file];
        let known = self.def_type_known(found);
        let (mut ir, warnings) = self.lower_definition_call(
            (&definition, Invocation::Call(&args)),
            (resolved.def_type_cell, resolved.definition_cell),
            (known, root.as_ref()),
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
        })
    }

    /// Runs `call`, a prepared invocation, in graph mode, against `graph`,
    /// `defers` saying which deferring calls the host takes as steps (see
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
        defers: Defers<'_>,
    ) -> Result<Progress, InvokeError> {
        for file in call.files {
            if let Some(node) = self
                .run_files(file, Some((&mut *graph, defers)))
                .map_err(InvokeError::Diagnostics)?
            {
                return Ok(Progress::Paused(node));
            }
        }
        let direct = Direct {
            temp_dirs: Some(&self.temp_dirs),
        };
        let context = (
            &mut self.memo,
            Some(&self.exec_result),
            &self.scan.dir_texts[..],
            &direct as &dyn Effects,
        );
        let run = GraphRun::new(mem::take(graph), call.unit, defers);
        let outcome = session::run_input_in(
            &call.ir,
            &mut self.frame,
            context,
            (call.root.clone(), Some(&run)),
        );
        let (left, paused) = run.finish();
        *graph = left;
        match outcome {
            Ok(value) => Ok(Progress::Done(value)),
            Err(error) if error.mark == Mark::PENDING => match paused {
                Some(node) => Ok(Progress::Paused(node)),
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
