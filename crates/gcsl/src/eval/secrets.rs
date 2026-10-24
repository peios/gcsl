//! Evaluating operations on secrets: unwrapping their operands, wrapping their
//! results, and redacting their failures.
//!
//! # Secrets
//!
//! A secret, [`Value::Secret`], is a value and its public identity, shown
//! as `***` (see the checker's "Secrets"); it is never the value of another
//! secret. An operation on secrets is lowered as a `SecretWrap` of the
//! operation, each of whose operands is a `SecretValue`: one that is a
//! secret evaluates to its value, its identity pushed onto a stack in the
//! run's shared state (`Evaluator::secret_value`), and, for a comparison of
//! values holding secrets, every secret it holds is its value, their
//! identities pushed in the order met (`unsealed`). The wrap
//! (`Evaluator::secret_wrap`) makes what the operation computes a secret,
//! its identity the identities pushed while it ran, joined by plus signs,
//! as in `user + password`; a secret it computes, a secret's field that is
//! one, is its value, the identities joined with its own.
//!
//! A failure of the operation itself, a division by zero, an overflow or an
//! index past the end of a secret list, can tell of the secret: it is "an
//! operation on a secret failed", where it failed, naming nothing else, and
//! marked so that no `recover` catches and no `#[retry]` retries it
//! ([`crate::diagnostic`]'s `Mark::SECRET`). A failure of an operand is the
//! operand's, and passes through as it is: the operand's `SecretValue`
//! flags it, and the wrap clears the flag. An operand that is a secret
//! computed on secrets failed in a wrap of its own, already redacted. An
//! error with a mark passes through too: a bug in GCSL, and the passing of
//! an enclosing call's deadline, which that call turns into its own error
//! as ever.
//!
//! Anywhere else two secrets are compared, they are equal if their
//! identities are, whatever their values ([`crate::value::Secret`]), and a
//! memoised call's arguments hash a secret by its identity alone: total
//! equality, `===`, compares fields a value's type does not name, which may
//! hold a secret the checker cannot see, and a memoised call's cache hit is
//! seen in whether it runs, so neither may depend on a secret's value.
//! Two secrets of one identity are one secret to them: an identity names
//! its value, as a key's fingerprint does.

use std::rc::Rc;

use crate::diagnostic::{Diagnostic, Mark as ErrorMark};
use crate::ir::{IrId, IrNode};
use crate::value::{Secret, Value};

use super::{Evaluator, Flow, Unwind, unsealed};

impl Evaluator<'_> {
    /// The value of `secret`'s value, if it is a secret, its identity kept
    /// for the innermost operation on secrets, or the value itself; if
    /// `deep`, with every secret it holds its value, and their identities
    /// kept too, in order (see [Secrets](#secrets)).
    pub(super) fn secret_value(&mut self, secret: IrId, deep: bool) -> Flow<Value> {
        let value = match self.value(secret) {
            Ok(value) => value,
            Err(unwind) => {
                // A failure of an operand, none of the operation's own.
                self.shared.operand_failed.set(true);
                return Err(unwind);
            }
        };
        let mut identities = self.shared.identities.borrow_mut();
        if deep {
            return Ok(unsealed(value, &mut identities));
        }
        match value {
            Value::Secret(secret) => {
                identities.push(Rc::clone(&secret.identity));
                Ok(secret.value.clone())
            }
            value => Ok(value),
        }
    }

    /// Evaluates `computed`, an operation on the values of secrets, for the
    /// `SecretWrap` `node` (see [Secrets](#secrets)): its value as a secret
    /// whose identity is the identities of the secrets it computed on,
    /// joined by ` + ` in order; a failure of it is "an operation on a
    /// secret failed", where it failed, naming nothing else, which nothing
    /// recovers or retries, since whether it fails can depend on a secret.
    /// A failure of one of its operands is that operand's, and passes
    /// through as it is: an operand that is a secret computed on secrets
    /// failed in an operation of its own. So does an error with a mark: a
    /// bug in GCSL, the
    /// passing of an enclosing call's deadline, which that call turns into
    /// its own error as ever, and the failure of an operation on a secret
    /// inside this one, already redacted where it failed.
    #[inline(never)]
    pub(super) fn secret_wrap(&mut self, computed: IrId, node: &IrNode) -> Flow<Value> {
        let depth = self.shared.identities.borrow().len();
        let result = self.value(computed);
        let identities: Vec<Rc<str>> = self.shared.identities.borrow_mut().split_off(depth);
        let of_operand = self.shared.operand_failed.replace(false);
        match result {
            Ok(value) => {
                // A value that is a secret already, a secret's field that
                // is one, is its value: a secret's value is never a secret.
                let (value, inner) = match value {
                    Value::Secret(secret) => {
                        (secret.value.clone(), Some(Rc::clone(&secret.identity)))
                    }
                    value => (value, None),
                };
                let identity: Vec<&str> =
                    identities.iter().chain(&inner).map(AsRef::as_ref).collect();
                Ok(Value::Secret(Rc::new(Secret {
                    value,
                    identity: Rc::from(identity.join(" + ")),
                })))
            }
            Err(Unwind::Error(error)) if error.mark == ErrorMark::NONE && !of_operand => {
                let mut failed = Diagnostic::error("an operation on a secret failed", node.span);
                failed.mark = ErrorMark::SECRET;
                failed.input = error.input;
                Err(failed.into())
            }
            Err(unwind) => Err(unwind),
        }
    }
}
