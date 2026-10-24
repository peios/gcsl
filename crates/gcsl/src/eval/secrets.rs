//! Evaluating operations on secrets: unwrapping their operands, wrapping their
//! results, and redacting their failures.
//!
//! # Secrets
//!
//! A secret, [`Value::Secret`], is a value and its public identity, shown
//! as `***` (see the checker's "Secrets"); it is never the value of another
//! secret. An operation on secrets is lowered as a `SecretWrap` of the
//! operation, each of whose operands is a `SecretValue`: one that is a
//! secret evaluates to its value, and, for a comparison of values holding
//! secrets, every secret it holds is its value (`unsealed`); each pushes
//! its token onto a stack in the run's shared state
//! (`Evaluator::secret_value`), what the operand is written as in the
//! operation's derivation: its source text with every secret in it written
//! `$(identity)` ([`crate::value::Derivation`]), or a digest of a long one.
//! An interpolation of a secret has its literal parts for operands too. The
//! wrap (`Evaluator::secret_wrap`) makes what the operation computes a
//! secret whose identity is its derivation, the operation's name and the
//! tokens pushed while it ran, `str-concat($(user), ":")` or
//! `field.pass($(login))`: equal derivations have equal identities, and
//! different ones different, so that an identity names one value, as
//! secrets' equality and memo keys need. A secret it computes, a secret's
//! field that is one, is its value, the derivation naming it. The wrap
//! makes
//! it revealed if the checker found every secret computed on revealed, and
//! only if a secret it computes is revealed too; `secret_reveal` makes a
//! secret revealed, of its identity, and `exec_secret` gives a revealed
//! secret's value to a program as an environment variable, reporting what
//! the program did as a secret.
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
use crate::ir::{IrId, IrKind, IrNode};
use crate::value::{Secret, Value};

use super::{Evaluator, Flow, Unwind, unsealed};

impl Evaluator<'_> {
    /// The value of `secret`'s value, if it is a secret, or the value
    /// itself; if `deep`, with every secret it holds its value (see
    /// [Secrets](#secrets)). Whichever it is, the operand's token, what it
    /// is in the derivation of the innermost operation on secrets, is kept
    /// for that operation: its [`Value::derivation`], every secret in it
    /// its identity, digested if it is long ([`token`]).
    pub(super) fn secret_value(&mut self, secret: IrId, deep: bool) -> Flow<Value> {
        let value = match self.value(secret) {
            Ok(value) => value,
            Err(unwind) => {
                // A failure of an operand, none of the operation's own.
                self.shared.operand_failed.set(true);
                return Err(unwind);
            }
        };
        self.shared.identities.borrow_mut().push(token(&value));
        if deep {
            return Ok(unsealed(value));
        }
        match value {
            Value::Secret(secret) => Ok(secret.value.clone()),
            value => Ok(value),
        }
    }

    /// Evaluates `computed`, an operation on the values of secrets, for the
    /// `SecretWrap` `node` (see [Secrets](#secrets)): its value as a secret,
    /// revealed if `revealed`, whose identity is its derivation, the
    /// operation and the tokens of its operands, `int-add($(answer), 1)`, so
    /// that one identity is one value; a failure of it is "an operation on a
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
    pub(super) fn secret_wrap(
        &mut self,
        computed: IrId,
        revealed: bool,
        node: &IrNode,
    ) -> Flow<Value> {
        let depth = self.shared.identities.borrow().len();
        let result = self.value(computed);
        let tokens: Vec<Rc<str>> = self.shared.identities.borrow_mut().split_off(depth);
        let of_operand = self.shared.operand_failed.replace(false);
        match result {
            Ok(value) => {
                // A value that is a secret already, a secret's field that
                // is one, is its value: a secret's value is never a secret.
                // It is revealed only if that secret is too; its identity is
                // the derivation, which names it already.
                let (value, revealed) = match value {
                    Value::Secret(secret) => (secret.value.clone(), revealed && secret.revealed),
                    value => (value, revealed),
                };
                let identity = format!("{}({})", self.operation(computed), tokens.join(", "));
                Ok(Value::Secret(Rc::new(Secret {
                    value,
                    identity: Rc::from(identity),
                    revealed,
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

impl Evaluator<'_> {
    /// The operation the node `computed` is, as a secret's derivation names
    /// it: its kind's name, and a field read's field, `field.pass`.
    fn operation(&self, computed: IrId) -> String {
        let kind = self.ir.get(computed).kind;
        match kind {
            IrKind::Field { name, .. } => {
                format!("field.{}", self.ir.name_text(name).unwrap_or("?"))
            }
            kind => kind.name().to_owned(),
        }
    }
}

/// The token of `value`, an operand of an operation on secrets, in the
/// operation's derivation: its [`Value::derivation`], or, past
/// [`TOKEN_LIMIT`] bytes, `#` and the hexadecimal FNV-1a-128 digest of it,
/// so that a derivation stays short whatever its operands are.
pub(super) fn token(value: &Value) -> Rc<str> {
    let text = value.derivation().to_string();
    if text.len() <= TOKEN_LIMIT {
        return Rc::from(text);
    }
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for &byte in text.as_bytes() {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    Rc::from(format!("#{hash:032x}"))
}

/// The longest token of an operand that is written out, in bytes.
const TOKEN_LIMIT: usize = 256;
