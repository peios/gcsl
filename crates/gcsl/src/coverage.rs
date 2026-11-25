//! What a run's code used, recorded for tests of reach (see
//! [Reach](crate::workspace::reach#reach)): each function whose code it
//! looked up to run, each top-level cell a function read, and each context
//! it read. Built only
//! for tests, or with the `coverage` feature; nothing is recorded unless a
//! thread [`start`]s recording.

use std::cell::{Cell, RefCell};

/// Something a run's code used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Used {
    /// The function of input `input` whose body begins at offset `at`.
    Function {
        /// Its code's input number.
        input: u32,
        /// Where its body begins.
        at: u32,
    },
    /// The context of the declaration's key `key`, read.
    Context {
        /// Its declaration's key.
        key: crate::names::Name,
    },
    /// The cell `slot` of the top level's frame, read by code of input
    /// `input`.
    Top {
        /// The reading code's input number.
        input: u32,
        /// The cell's slot.
        slot: u32,
    },
}

thread_local! {
    /// Whether this thread records.
    static ON: Cell<bool> = const { Cell::new(false) };
    /// What this thread recorded.
    static USED: RefCell<Vec<Used>> = const { RefCell::new(Vec::new()) };
}

/// Starts recording on this thread, forgetting what was recorded before.
pub fn start() {
    USED.with(|used| used.borrow_mut().clear());
    ON.with(|on| on.set(true));
}

/// Stops recording on this thread: what was recorded since [`start`].
pub fn stop() -> Vec<Used> {
    ON.with(|on| on.set(false));
    USED.with(|used| std::mem::take(&mut *used.borrow_mut()))
}

/// Records `used`, if this thread records.
pub(crate) fn record(used: Used) {
    if ON.with(Cell::get) {
        USED.with(|all| all.borrow_mut().push(used));
    }
}
