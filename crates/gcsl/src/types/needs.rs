//! The contexts function types need ([`FnTy::needs`]), interned.
//!
//! # Needs
//!
//! A function type's needs are a set of [`Need`]s, every one of which a
//! caller must meet. A need is a context with no default, by its
//! declaration's key, first, and then the contexts with defaults it is read
//! through, sorted: in a function that reads `^target`, whose default reads
//! `^arch`, which has none, the need is `arch` unless `target` is bound,
//! since the default of a bound context is never evaluated. A need is met
//! where any of its contexts is bound ([`met`]). One need *implies* another
//! if its contexts are among the other's ([`implies`]): whatever meets it
//! meets the other. A set is kept minimal, with no need that another of it
//! implies, and sorted, and is interned as types are, so that two sets are
//! equal exactly when their ids are: a [`NeedsId`], the empty set
//! [`NeedsId::NONE`], which every table holds.
//!
//! A function type fits another only if each of its needs is implied by
//! one of the other's ([`TypeTable::needs_fit`]), and two join to one
//! needing what either needs. A need is shown as its contexts, `^arch`, or
//! `^arch | ^target` for one read through a default.

use std::collections::HashMap;

use crate::names::Name;

use super::{FnTy, TyId, TyKind, TypeTable};

/// One need of a function type (see [Needs](#needs)): a context with no
/// default, by its key, then the contexts with defaults it is read through,
/// sorted.
pub type Need = Box<[Name]>;

/// Whether `need` is met where the contexts `bound` are: whether one of its
/// contexts is among them.
#[must_use]
pub fn met(need: &[Name], bound: &[Name]) -> bool {
    need.iter().any(|key| bound.contains(key))
}

/// Whether the need `strong` implies the need `weak`: whether its contexts
/// are among `weak`'s, so that what meets it meets `weak`.
#[must_use]
pub fn implies(strong: &[Name], weak: &[Name]) -> bool {
    strong.iter().all(|key| weak.contains(key))
}

/// `need` read through the context of the key `through`, which has a
/// default: met where that is bound, too.
#[must_use]
pub fn through(need: &[Name], through: Name) -> Need {
    let Some((&first, guards)) = need.split_first() else {
        return need.into();
    };
    let mut guards = guards.to_vec();
    if first != through && !guards.contains(&through) {
        guards.push(through);
        guards.sort_unstable();
    }
    std::iter::once(first).chain(guards).collect()
}

/// A set of needs, interned in a [`TypeTable`] (see [Needs](#needs)):
/// read it with [`TypeTable::need_set`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NeedsId(u32);

impl NeedsId {
    /// The empty set, which every table holds.
    pub const NONE: NeedsId = NeedsId(0);
}

/// A table's sets of needs, and what each context's default needs (see
/// [Needs](#needs)).
#[derive(Clone, Debug)]
pub(super) struct NeedTable {
    /// Each set, by its id, the empty set first.
    sets: Vec<Box<[Need]>>,
    /// Each set's id.
    ids: HashMap<Box<[Need]>, NeedsId>,
    /// What the default of each context with one needs, by its key.
    defaults: HashMap<Name, Box<[Need]>>,
}

impl Default for NeedTable {
    fn default() -> Self {
        Self {
            sets: vec![Box::default()],
            ids: HashMap::new(),
            defaults: HashMap::new(),
        }
    }
}

impl TypeTable {
    /// The function type `ty` needing `needs` too, as well as its own (see
    /// [Needs](#needs)); `ty` itself if it is no function type or already
    /// needs them all.
    pub fn with_needs(&mut self, ty: TyId, needs: &[Need]) -> TyId {
        let Some(TyKind::Fn(sig)) = self.kinds.get(ty.index()).cloned() else {
            return ty;
        };
        let had = self.need_set(sig.needs);
        if needs
            .iter()
            .all(|need| had.iter().any(|old| implies(old, need)))
        {
            return ty;
        }
        let joined: Vec<Need> = had.iter().chain(needs).cloned().collect();
        let sig = FnTy {
            needs: self.intern_needs(joined),
            ..sig
        };
        self.intern(TyKind::Fn(sig))
    }

    /// The function type `ty` needing the set `needs` too, as
    /// [`TypeTable::with_needs`] makes it.
    pub fn with_need_set(&mut self, ty: TyId, needs: NeedsId) -> TyId {
        if needs == NeedsId::NONE {
            return ty;
        }
        let needs = self.need_set(needs).to_vec();
        self.with_needs(ty, &needs)
    }

    /// The id of the set of `needs`, kept minimal and sorted, adding it to
    /// the table if it is new.
    pub fn intern_needs(&mut self, mut needs: Vec<Need>) -> NeedsId {
        needs.sort_unstable();
        needs.dedup();
        let kept: Vec<Need> = needs
            .iter()
            .filter(|need| {
                !needs
                    .iter()
                    .any(|other| other != *need && implies(other, need))
            })
            .cloned()
            .collect();
        if kept.is_empty() {
            return NeedsId::NONE;
        }
        let set: Box<[Need]> = kept.into();
        if let Some(&id) = self.need_table.ids.get(&set) {
            return id;
        }
        let id = NeedsId(u32::try_from(self.need_table.sets.len()).unwrap_or(u32::MAX));
        self.need_table.sets.push(set.clone());
        self.need_table.ids.insert(set, id);
        id
    }

    /// The needs in the set `id`.
    #[must_use]
    pub fn need_set(&self, id: NeedsId) -> &[Need] {
        self.need_table
            .sets
            .get(id.0 as usize)
            .map_or(&[], |set| set)
    }

    /// What the function type `ty` needs (see [Needs](#needs)); nothing for
    /// any other type.
    #[must_use]
    pub fn needs(&self, ty: TyId) -> &[Need] {
        match self.kinds.get(ty.index()) {
            Some(TyKind::Fn(sig)) => self.need_set(sig.needs),
            _ => &[],
        }
    }

    /// Whether a function needing the set `offered` fits where one needing
    /// `required` is expected: whether each of `offered` is implied by one
    /// of `required`.
    #[must_use]
    pub fn needs_fit(&self, offered: NeedsId, required: NeedsId) -> bool {
        let required = self.need_set(required);
        self.need_set(offered)
            .iter()
            .all(|need| required.iter().any(|allowed| implies(allowed, need)))
    }

    /// The set of what the sets `a` and `b` need, as a join needs it.
    pub fn joined_needs(&mut self, a: NeedsId, b: NeedsId) -> NeedsId {
        if a == b || b == NeedsId::NONE {
            return a;
        }
        let joined: Vec<Need> = self
            .need_set(a)
            .iter()
            .chain(self.need_set(b))
            .cloned()
            .collect();
        self.intern_needs(joined)
    }

    /// Records what the default of the context whose declaration's key is
    /// `key` needs, as its file's check found.
    pub fn set_context_needs(&mut self, key: Name, needs: &[Need]) {
        self.need_table.defaults.insert(key, needs.into());
    }

    /// What the default of the context whose declaration's key is `key`
    /// needs, if its file recorded it; nothing otherwise.
    #[must_use]
    pub fn context_needs(&self, key: Name) -> &[Need] {
        self.need_table
            .defaults
            .get(&key)
            .map_or(&[], |needs| needs)
    }
}
