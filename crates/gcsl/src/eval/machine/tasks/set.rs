//! The tasks of a unit set aside, those waiting for their turn kept apart
//! in program order (see [Tasks](super#tasks)).
//!
//! A loop whose iterations wait their turn for effects the host does not
//! isolate is split into a task for each iteration, so a unit can have
//! thousands of tasks set aside, nearly all waiting for their turn. The
//! machine looks at its tasks set aside each time one may run, ends or is
//! woken; were those that wait for their turn among them, each look would
//! be a look at every iteration, and a loop of N operations would take N²
//! looks, each of them more than once. So they are kept apart, by their
//! paths ([`Tasks::turns`]): the earliest is the first, the only one of
//! them that can have its turn but for those that hold a demanded run (see
//! [Across units](super#across-units)), and the others are seen only by
//! what must see every task, a cancellation, a wound, a charge or a stop.
//! Which task runs, and in what order effects are made, is what it would
//! be with them all in one list; only the order the list keeps among tasks
//! that may run at once can differ.

use std::collections::{BTreeMap, HashMap};

use super::{Status, Task};

/// The key a task waiting for its turn is kept by: its path, then its
/// number, which no other task has.
type TurnKey = (Vec<u32>, u32);

/// The tasks of a unit set aside (see the module documentation).
#[derive(Default)]
pub(in crate::eval) struct Tasks {
    /// Those not waiting for their turn, in the order the machine keeps
    /// them, which decides which of several that may run runs first.
    pub(in crate::eval) set_aside: Vec<Task>,
    /// Those waiting for their turn, by their paths.
    turns: BTreeMap<TurnKey, Task>,
    /// The key of each of those, by its number.
    keys: HashMap<u32, TurnKey>,
}

impl Tasks {
    /// Sets `task` aside: among those waiting for their turn if its status
    /// is [`Status::Waiting`], else last of the others.
    pub(in crate::eval) fn push(&mut self, task: Task) {
        if matches!(task.status, Status::Waiting(_)) {
            let key = (task.path().to_vec(), task.meta.id);
            self.keys.insert(task.meta.id, key.clone());
            self.turns.insert(key, task);
        } else {
            self.set_aside.push(task);
        }
    }

    /// How many tasks are set aside.
    pub(in crate::eval) fn len(&self) -> usize {
        self.set_aside.len() + self.turns.len()
    }

    /// Whether no task is set aside.
    pub(in crate::eval) fn is_empty(&self) -> bool {
        self.set_aside.is_empty() && self.turns.is_empty()
    }

    /// Every task set aside: those not waiting for their turn, in order,
    /// then those waiting, in program order.
    pub(in crate::eval) fn iter(&self) -> impl Iterator<Item = &Task> {
        self.set_aside.iter().chain(self.turns.values())
    }

    /// [`Tasks::iter`], to change them, each where it is: a change of a
    /// status from or to [`Status::Waiting`] is made through
    /// [`Tasks::settle`].
    pub(in crate::eval) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Task> {
        self.set_aside.iter_mut().chain(self.turns.values_mut())
    }

    /// The tasks waiting for their turn, in program order.
    pub(super) fn turns(&self) -> impl Iterator<Item = &Task> {
        self.turns.values()
    }

    /// The earliest task waiting for its turn, if any is.
    pub(super) fn first_turn(&self) -> Option<&Task> {
        self.turns.values().next()
    }

    /// Puts each task where its status says it goes, after
    /// [`Tasks::iter_mut`] changed some: one waiting for its turn no longer
    /// among those, or one among the others now waiting, is moved. Costs a
    /// look at each task that waits for its turn.
    pub(in crate::eval) fn settle(&mut self) {
        let woken: Vec<TurnKey> = self
            .turns
            .iter()
            .filter(|(_, task)| !matches!(task.status, Status::Waiting(_)))
            .map(|(key, _)| key.clone())
            .collect();
        for key in woken {
            self.keys.remove(&key.1);
            if let Some(task) = self.turns.remove(&key) {
                self.set_aside.push(task);
            }
        }
        let mut at = 0;
        while at < self.set_aside.len() {
            if self
                .set_aside
                .get(at)
                .is_some_and(|task| matches!(task.status, Status::Waiting(_)))
            {
                let task = self.set_aside.remove(at);
                self.push(task);
            } else {
                at += 1;
            }
        }
    }

    /// Has `woken` change the task waiting for its turn whose number is
    /// `id`, its status among what it changes, and puts it where its status
    /// then says: it is woken, or cancelled. Whether it was there.
    pub(super) fn take_turn(&mut self, id: u32, woken: impl FnOnce(&mut Task)) -> bool {
        let Some(key) = self.keys.get(&id).cloned() else {
            return false;
        };
        self.take_turn_at(&key, woken)
    }

    /// [`Tasks::take_turn`] of the task waiting for its turn at `key`.
    fn take_turn_at(&mut self, key: &TurnKey, woken: impl FnOnce(&mut Task)) -> bool {
        let Some(mut task) = self.turns.remove(key) else {
            return false;
        };
        self.keys.remove(&key.1);
        woken(&mut task);
        self.push(task);
        true
    }

    /// Takes the task numbered `id` out, wherever it is.
    pub(super) fn remove_id(&mut self, id: u32) -> Option<Task> {
        if let Some(at) = self.set_aside.iter().position(|task| task.meta.id == id) {
            return Some(self.set_aside.swap_remove(at));
        }
        let key = self.keys.remove(&id)?;
        self.turns.remove(&key)
    }

    /// The numbers of the tasks waiting for their turn that `pick` picks,
    /// in program order: those to wake or cancel, looked at one by one.
    pub(super) fn turn_ids(&self, mut pick: impl FnMut(&Task) -> bool) -> Vec<u32> {
        self.turns
            .values()
            .filter(|task| pick(task))
            .map(|task| task.meta.id)
            .collect()
    }
}
