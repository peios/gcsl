//! Profiling an evaluation: how often each function and built-in ran, and
//! for how long.
//!
//! # Profiling
//!
//! A host that sets a [`Profile`] on a workspace
//! ([`crate::workspace::Workspace::start_profiling`]) has each run of its
//! code tallied by *site*: a function, by its code, or a built-in. A run
//! without one pays one test of an `Option` per call.
//!
//! A site's tally counts its *calls*: each call of a closure of the
//! function that runs it, memoised or not, the body of a step a host runs
//! from its node, a function's default argument and a context's default
//! when evaluated, and each run of a `lazy`'s body. A deferred call, which
//! makes a node and runs nothing, is no call here; the run of its step is
//! (see [Graph mode](crate::eval::graph#graph-mode)). A memoised call that
//! finds its entry is a call, counted as *found* too. An effect a host runs
//! as a job on a thread of its own ([`crate::eval::graph::EffectJob::run`])
//! is the host's to time, and is in no profile.
//!
//! A call's time runs from when its arguments are evaluated, which is its
//! caller's code running, and a call that fails in its arguments never
//! ran, to when it ends, by any road, an error or a pause
//! for a step's result among them: its *total*, which for a memoised call
//! includes looking its entry up; and its *own* time, its total less the
//! totals of the calls made inside it. A site's total adds only
//! the calls of it that no call of it is running around, so that a
//! recursive function's time is not counted once for each level. The time
//! of calls that no call is running around is the profile's *outermost*
//! time: a host that knows how long its runs took has the time spent
//! outside every call, at the top level of its files, as the difference.
//!
//! Graph mode runs a unit again from its start after each pause (see
//! [Graph mode](crate::eval::graph#graph-mode)), so a profile of a build
//! counts every run of each unit, as the build spends its time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::ast::FnId;
use crate::check::Builtin;
use crate::ir::{FnCode, FnKind, Ir};
use crate::names::{Name, Names};
use crate::source::Span;

use super::Evaluator;

/// What a call runs, as a profile tallies it (see [Profiling](#profiling)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Site {
    /// A function of the code of session input `input`, numbered `function`
    /// in its IR.
    Code {
        /// The input of its code.
        input: u32,
        /// The function's number in its IR.
        function: FnId,
    },
    /// A built-in function.
    Builtin(Builtin),
}

/// A function's code, as a profile names it: its IR and its entry there.
pub(crate) type Code<'c> = (&'c Ir, &'c FnCode);

/// A profile being taken of an evaluation (see [Profiling](#profiling)).
#[derive(Debug, Default)]
pub struct Profile {
    /// Its tallies so far, borrowed only briefly, never across a call.
    tallies: RefCell<Tallies>,
}

/// What a [`Profile`] holds.
#[derive(Debug, Default)]
struct Tallies {
    /// Each site's tally.
    sites: HashMap<Site, Tally>,
    /// The calls running, innermost last.
    open: Vec<Open>,
    /// The time of the calls no call was running around.
    outermost: Duration,
}

/// A site's tally (see [Profiling](#profiling)).
#[derive(Clone, Debug)]
struct Tally {
    /// Where the function's body is; nowhere for a built-in.
    span: Span,
    /// The function's name, if it has one.
    name: Option<Name>,
    /// What kind of function it is; `None` for a built-in.
    kind: Option<FnKind>,
    /// Its calls that ended.
    calls: u64,
    /// Those of them that found a memo entry.
    found: u64,
    /// Their own time.
    own: Duration,
    /// Their total time, but for calls inside a call of the same site.
    total: Duration,
    /// How many calls of it are running past their arguments.
    running: u32,
}

/// A call running.
#[derive(Debug)]
struct Open {
    /// Its site.
    site: Site,
    /// When it began.
    began: Instant,
    /// The total time of the calls inside it that ended.
    inner: Duration,
    /// Whether it is running past its arguments, which are its caller's
    /// code: only then is its time its own.
    running: bool,
}

impl Profile {
    /// A profile of nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begins a call of `site`, a function whose code is `code`, or a
    /// built-in if `code` is `None`, its arguments evaluated already if
    /// `running`, or to be evaluated first ([`Profile::arguments_done`]).
    fn begin(&self, site: Site, code: Option<Code<'_>>, running: bool) {
        let mut tallies = self.tallies.borrow_mut();
        let tally = tallies.sites.entry(site).or_insert_with(|| Tally {
            span: code.map_or(Span::new(0, 0), |(ir, function)| ir.get(function.body).span),
            name: code.and_then(|(_, function)| function.name),
            kind: code.map(|(_, function)| function.kind),
            calls: 0,
            found: 0,
            own: Duration::ZERO,
            total: Duration::ZERO,
            running: 0,
        });
        tally.running += u32::from(running);
        tallies.open.push(Open {
            site,
            began: Instant::now(),
            inner: Duration::ZERO,
            running,
        });
    }

    /// Adds `elapsed`, the time of a call that ended, or of calls its
    /// arguments made, to the call running around it, or to the outermost
    /// time if none is.
    fn to_outer(tallies: &mut Tallies, elapsed: Duration) {
        match tallies.open.last_mut() {
            Some(outer) => outer.inner += elapsed,
            None => tallies.outermost += elapsed,
        }
    }

    /// Ends the innermost call running.
    fn end(&self) {
        let mut tallies = self.tallies.borrow_mut();
        let Some(open) = tallies.open.pop() else {
            return;
        };
        // One that failed in its arguments never ran: its time, and its
        // arguments' calls, are its caller's.
        if !open.running {
            Self::to_outer(&mut tallies, open.inner);
            return;
        }
        let elapsed = open.began.elapsed();
        if let Some(tally) = tallies.sites.get_mut(&open.site) {
            tally.calls += 1;
            tally.own += elapsed.saturating_sub(open.inner);
            tally.running = tally.running.saturating_sub(1);
            if tally.running == 0 {
                tally.total += elapsed;
            }
        }
        Self::to_outer(&mut tallies, elapsed);
    }

    /// Begins the innermost call running, now that its arguments are
    /// evaluated: the time it has taken so far, and the calls made inside
    /// it, are its caller's, as evaluating the arguments is the caller's
    /// code.
    fn arguments_done(&self) {
        let mut tallies = self.tallies.borrow_mut();
        let Some(open) = tallies.open.pop() else {
            return;
        };
        if open.running {
            tallies.open.push(open);
            return;
        }
        Self::to_outer(&mut tallies, open.inner);
        if let Some(tally) = tallies.sites.get_mut(&open.site) {
            tally.running += 1;
        }
        tallies.open.push(Open {
            began: Instant::now(),
            inner: Duration::ZERO,
            running: true,
            ..open
        });
    }

    /// Counts the innermost call running as one that found its memo entry.
    fn found(&self) {
        let mut tallies = self.tallies.borrow_mut();
        let Some(site) = tallies.open.last().map(|open| open.site) else {
            return;
        };
        if let Some(tally) = tallies.sites.get_mut(&site) {
            tally.found += 1;
        }
    }

    /// Runs `run` as a call of `site`, a function whose code is `code`, or
    /// a built-in if `code` is `None`, its arguments evaluated already if
    /// `running`, or by `run`, which says when ([`Profile::arguments_done`]).
    fn call<T>(
        &self,
        (site, code): (Site, Option<Code<'_>>),
        running: bool,
        run: impl FnOnce() -> T,
    ) -> T {
        self.begin(site, code, running);
        let result = run();
        self.end();
        result
    }

    /// What the profile found, its functions' names read in `names`: each
    /// site's tally, the longest own time first, and the outermost time.
    #[must_use]
    pub fn report(&self, names: &Names) -> ProfileReport {
        let tallies = self.tallies.borrow();
        let mut rows: Vec<ProfileRow> = tallies
            .sites
            .iter()
            .map(|(site, tally)| ProfileRow {
                site: match *site {
                    Site::Code { input, .. } => Profiled::Function {
                        input,
                        span: tally.span,
                        name: tally.name.map(|name| names.get(name).to_owned()),
                        kind: tally.kind.unwrap_or(FnKind::Function),
                    },
                    Site::Builtin(builtin) => Profiled::Builtin(builtin.name()),
                },
                calls: tally.calls,
                found: tally.found,
                own: tally.own,
                total: tally.total,
            })
            .collect();
        rows.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| b.calls.cmp(&a.calls)));
        ProfileReport {
            rows,
            outermost: tallies.outermost,
        }
    }
}

/// What a [`Profile`] found (see [Profiling](#profiling)).
#[derive(Clone, Debug, Default)]
pub struct ProfileReport {
    /// Each site's tally, the longest own time first.
    pub rows: Vec<ProfileRow>,
    /// The time of the calls no call was running around.
    pub outermost: Duration,
}

/// A site's tally (see [Profiling](#profiling)).
#[derive(Clone, Debug)]
pub struct ProfileRow {
    /// What ran.
    pub site: Profiled,
    /// How many calls of it ended.
    pub calls: u64,
    /// How many of those found a memo entry.
    pub found: u64,
    /// Their own time.
    pub own: Duration,
    /// Their total time, but for calls inside a call of the same site.
    pub total: Duration,
}

/// What a [`ProfileRow`] tallies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Profiled {
    /// A function.
    Function {
        /// The session input of its code.
        input: u32,
        /// Where its body is in that input.
        span: Span,
        /// The name it refers to itself by, if it has one: the `let`,
        /// field or function item it initialises.
        name: Option<String>,
        /// Whether it is a function, a memoised one, or a `lazy`'s body.
        kind: FnKind,
    },
    /// A built-in function, by its name in the `builtins` module.
    Builtin(&'static str),
}

impl Evaluator<'_> {
    /// Runs `run` on this frame as a call of the function `function` of
    /// `code`, whose entry there is `found`, tallied if the run is profiled
    /// (see [Profiling](#profiling)): one whose arguments are evaluated
    /// already if `running`, or that evaluates them first and says so
    /// ([`Evaluator::arguments_done`]).
    pub(super) fn profiled<T>(
        &mut self,
        (code, function, found): (&Ir, FnId, &FnCode),
        running: bool,
        run: impl FnOnce(&mut Self) -> T,
    ) -> T {
        match self.shared.profile {
            Some(profile) => {
                let site = Site::Code {
                    input: code.input(),
                    function,
                };
                profile.call((site, Some((code, found))), running, || run(self))
            }
            None => run(self),
        }
    }

    /// Runs `run` on this frame as a call of `builtin`, its arguments
    /// evaluated, tallied if the run is profiled (see
    /// [Profiling](#profiling)).
    pub(super) fn profiled_builtin<T>(
        &mut self,
        builtin: Builtin,
        run: impl FnOnce(&mut Self) -> T,
    ) -> T {
        match self.shared.profile {
            Some(profile) => profile.call((Site::Builtin(builtin), None), true, || run(self)),
            None => run(self),
        }
    }

    /// Notes that the arguments of the innermost call running are
    /// evaluated, if the run is profiled: its own time begins (see
    /// [Profiling](#profiling)).
    pub(super) fn arguments_done(&self) {
        if let Some(profile) = self.shared.profile {
            profile.arguments_done();
        }
    }

    /// Notes that the innermost call running found its memo entry, if the
    /// run is profiled.
    pub(super) fn memo_found(&self) {
        if let Some(profile) = self.shared.profile {
            profile.found();
        }
    }
}
