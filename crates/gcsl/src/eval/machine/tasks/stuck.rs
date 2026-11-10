//! Units stuck in a circle: when no unit a host has can go on, the waits
//! between their tasks, and a circle among them that a call needing its
//! own result closes (see [Across units](super#across-units)).
//!
//! Every task of every suspended unit is a point of a graph, and each wait
//! an edge to what it waits for: a call or `lazy` another task runs, in
//! its unit or another (*data* edges), its live children, or, for its
//! turn, each earlier task of its unit that the turn rule makes it wait
//! for. A wait for a node's result is the host's, and leads nowhere. A
//! circle with a data edge in it is a call that depends on its own result:
//! under the rules of [Across units](super#across-units), each turn waited
//! for stays inside a run, and the circle comes back into it through what
//! that run needs.

use std::collections::HashMap;
use std::rc::Rc;

use crate::source::Span;
use crate::value::Future;

use super::super::super::graph::Wait;
use super::super::super::memo::{MemoTables, Running};
use super::super::suspend::Suspended;
use super::super::{Activation, CallState, Frame, Parked};
use super::{Meta, Run, Status, Wound, path_of, site_of};

/// A data wait in a circle: the call or await that waits, by its unit, its
/// input and span, what it needs, and the unit running that.
#[derive(Clone, Debug)]
pub(crate) struct Need {
    /// The unit that waits.
    pub(crate) unit: u32,
    /// The input of the code the call or await is in.
    pub(crate) input: u32,
    /// Where the call or await is.
    pub(crate) span: Span,
    /// What it needs.
    pub(crate) what: Needed,
    /// The unit running it.
    pub(crate) holder: u32,
}

/// What a call or await in a circle needs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Needed {
    /// A memoised call of the function of this name, if it has one.
    Call(Option<crate::names::Name>),
    /// A `lazy`.
    Lazy,
}

/// A task of a suspended unit, as the graph reads it.
struct Point<'a> {
    /// Its unit.
    unit: u32,
    /// Its place.
    meta: &'a Meta,
    /// Its frames.
    frames: &'a [Frame],
    /// Its calls.
    calls: &'a [CallState],
    /// Its activations.
    activations: &'a [Activation],
    /// Its running memoised calls.
    running: &'a [Running],
    /// The `lazy`s it is running.
    futures: &'a [Rc<Future>],
    /// What it waits for.
    status: Option<&'a Status>,
}

impl Point<'_> {
    /// Its path (see [Tasks](super#tasks)).
    fn path(&self) -> Vec<u32> {
        path_of(self.meta, self.frames, self.calls)
    }

    /// Whether it runs `target`'s call or `lazy`: its frame is among its
    /// frames.
    fn runs(&self, target: Wound) -> bool {
        site_of(target, (self.meta, self.frames, self.calls)).is_some()
    }

    /// The demanded runs it holds, by identity.
    fn demanded(&self, memo: &MemoTables) -> (Vec<u64>, Vec<*const Future>) {
        (
            self.running
                .iter()
                .filter(|running| memo.is_demanded(running.at()))
                .map(|running| running.entry)
                .collect(),
            self.futures
                .iter()
                .filter(|future| future.is_demanded())
                .map(Rc::as_ptr)
                .collect(),
        )
    }

    /// Whether it holds every run of `held`.
    fn holds(&self, (entries, futures): &(Vec<u64>, Vec<*const Future>)) -> bool {
        entries
            .iter()
            .all(|entry| self.running.iter().any(|running| running.entry == *entry))
            && futures.iter().all(|future| {
                self.futures
                    .iter()
                    .any(|run| std::ptr::eq(Rc::as_ptr(run), *future))
            })
    }

    /// Where the call or await it waits to take up is, and what it needs:
    /// its input, span and what (see [`Need::what`]).
    fn site(&self, run: &Run) -> Option<(u32, Span, Needed)> {
        let ir = &self.activations.last()?.ir;
        match run {
            Run::Entry(_) => {
                let call = self.calls.last()?;
                let name = call
                    .closure
                    .code
                    .find_function(call.closure.function)
                    .and_then(|code| code.name);
                Some((ir.input(), ir.get(call.node).span, Needed::Call(name)))
            }
            Run::Future(_) => {
                let (Status::Blocked(_, resume) | Status::Sibling(resume)) = self.status? else {
                    return None;
                };
                Some((ir.input(), resume.await_span()?, Needed::Lazy))
            }
        }
    }
}

/// The run `run` is, as a wound would name it.
fn target(run: &Run) -> Wound {
    match run {
        Run::Entry(at) => Wound::Entry(at.2),
        Run::Future(future) => Wound::Future(Rc::as_ptr(future)),
    }
}

/// The circle the suspended units `suspended` wait in, if there is one
/// that a call or `lazy` needing its own result closes: its data waits, in
/// order round the circle, beginning with the earliest unit's. `memo` says
/// which runs are demanded.
pub(crate) fn circle<'a>(
    suspended: impl Iterator<Item = (u32, &'a Suspended)>,
    memo: &MemoTables,
) -> Option<Vec<Need>> {
    let mut points: Vec<Point<'_>> = Vec::new();
    let mut units: Vec<(u32, &Suspended)> = suspended.collect();
    units.sort_by_key(|(unit, _)| *unit);
    for (unit, suspended) in &units {
        let (parked, running, futures): (&Parked, &[Running], &[Rc<Future>]) = suspended.parts();
        let (tasks, meta, status) = &parked.tasks;
        points.push(Point {
            unit: *unit,
            meta,
            frames: &parked.frames,
            calls: &parked.calls,
            activations: &parked.activations,
            running,
            futures,
            status: status.as_ref(),
        });
        let mut set_aside: Vec<&super::Task> = tasks.iter().collect();
        set_aside.sort_by_key(|task| task.meta.id);
        points.extend(set_aside.into_iter().map(|task| Point {
            unit: *unit,
            meta: &task.meta,
            frames: &task.frames,
            calls: &task.calls,
            activations: &task.activations,
            running: &task.own.running,
            futures: &task.own.futures,
            status: Some(&task.status),
        }));
    }
    let at: HashMap<(u32, u32), usize> = points
        .iter()
        .enumerate()
        .map(|(index, point)| ((point.unit, point.meta.id), index))
        .collect();
    let edges: Vec<Vec<(usize, Option<Need>)>> = points
        .iter()
        .map(|point| waits(point, &points, &at, memo))
        .collect();
    let found = find_circle(&edges)?;
    let needs: Vec<Need> = found
        .iter()
        .filter_map(|&(from, to)| edges.get(from)?.iter().find(|edge| edge.0 == to)?.1.clone())
        .collect();
    (!needs.is_empty()).then_some(needs)
}

/// The waits of `point`, each the index among `points` of what it waits
/// for, `at` finding a task by its unit and number, and, for a data wait,
/// what it needs.
fn waits(
    point: &Point<'_>,
    points: &[Point<'_>],
    at: &HashMap<(u32, u32), usize>,
    memo: &MemoTables,
) -> Vec<(usize, Option<Need>)> {
    let Some(status) = point.status else {
        return Vec::new();
    };
    let holder_in = |unit: u32, run: &Run| -> Vec<(usize, Option<Need>)> {
        let Some(holder) = points
            .iter()
            .position(|other| other.unit == unit && other.runs(target(run)))
        else {
            return Vec::new();
        };
        let need = point.site(run).map(|(input, span, what)| Need {
            unit: point.unit,
            input,
            span,
            what,
            holder: unit,
        });
        vec![(holder, need)]
    };
    match status {
        Status::Blocked(Wait::Unit(unit), resume) => resume
            .waits_for()
            .map(|run| holder_in(*unit, &run))
            .unwrap_or_default(),
        Status::Sibling(resume) => resume
            .waits_for()
            .map(|run| holder_in(point.unit, &run))
            .unwrap_or_default(),
        Status::Joining => point
            .frames
            .last()
            .and_then(Frame::join)
            .map(|join| {
                join.borrow()
                    .children
                    .iter()
                    .filter(|child| child.3.is_none())
                    .filter_map(|child| at.get(&(point.unit, child.2)))
                    .map(|&child| (child, None))
                    .collect()
            })
            .unwrap_or_default(),
        Status::Waiting(_) => {
            let mine = point.path();
            let demanded = point.demanded(memo);
            points
                .iter()
                .enumerate()
                .filter(|(_, other)| {
                    other.unit == point.unit
                        && other.meta.id != point.meta.id
                        && other.holds(&demanded)
                        && other.path() < mine
                })
                .map(|(index, _)| (index, None))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// A circle among `edges`, each point's waits, as its edges in order, each
/// a point and the next; the first found from the earliest point.
fn find_circle(edges: &[Vec<(usize, Option<Need>)>]) -> Option<Vec<(usize, usize)>> {
    // 0 unvisited, 1 on the path being walked, 2 done.
    let mut state = vec![0u8; edges.len()];
    for start in 0..edges.len() {
        if state[start] != 0 {
            continue;
        }
        // The path walked, each point with the next of its edges to try.
        let mut path: Vec<(usize, usize)> = vec![(start, 0)];
        state[start] = 1;
        while let Some(&mut (point, ref mut next)) = path.last_mut() {
            let Some(&(to, _)) = edges[point].get(*next) else {
                state[point] = 2;
                path.pop();
                continue;
            };
            *next += 1;
            match state[to] {
                0 => {
                    state[to] = 1;
                    path.push((to, 0));
                }
                1 => {
                    let from = path.iter().position(|&(on, _)| on == to)?;
                    let circle: Vec<usize> = path[from..].iter().map(|&(on, _)| on).collect();
                    let mut pairs: Vec<(usize, usize)> =
                        circle.windows(2).map(|pair| (pair[0], pair[1])).collect();
                    pairs.push((point, to));
                    return Some(pairs);
                }
                _ => {}
            }
        }
    }
    None
}
