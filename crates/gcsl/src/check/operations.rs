//! Checking host operations and handle types.
//!
//! # Host operations
//!
//! An *operation*, `extern fn boot(profile: Profile) -> Vm = "vm.boot"` (see
//! the parser's [Host operations](crate::parser::externs#host-operations)),
//! is a function whose calls the host of the language answers: the program
//! gives the host the operation's name and its arguments, and the host gives
//! back a value. Its function is a function item like any other, a named
//! function, a method or a `def`, impure, typed by its declared parameters
//! and result; so a call of it binds its arguments, gives its defaults,
//! fills its rest parameter and is refused in a pure function as any
//! impure function's is, and it is a value of its function type, passed and
//! stored. Only its body is its own: the call the parser made of an
//! [`ExprKind::Operation`], with the operation's name and a list of a read
//! of each parameter (`Checker::check_operation_call`).
//!
//! The host is given *data* and takes none of the evaluator's own values,
//! so each parameter is read as its bits all the way down, awaited deeply
//! where it may be a future or hold one (`Checker::measure_deep`), and what
//! it is then must be data
//! ([`TypeTable::is_data`](crate::types::TypeTable::is_data)): a scalar, a
//! file, a directory, a record, a list or a union of data, or a value of an
//! enum whose bound is data, as a handle is. Anything else, a function, a
//! secret, a sandbox, a variant as a value, at any depth, is "an
//! operation's parameter must be data, found `fn(): Int`", at the
//! parameter, labelled with its type and noted "the host is given plain
//! data and handles: no function, future, secret or sandbox". The declared
//! result must be data too, "an operation's result must be data, found
//! `Secret<String>`", at the return type: a bare type in it is present,
//! since the host's answer is a value, and one written a future,
//! `Future<Int>`, at any depth, is no data. An operation whose result is
//! `{}` gives nothing back.
//!
//! The call's type is the declared result, present: the host's answer is
//! needed at once. What the host gives is checked where it arrives: the
//! call is recorded with the test of the declared result's type, as an `as`
//! is, of a value known to be of no type at all
//! ([`TypeTable::test_plan`](crate::types::TypeTable::test_plan),
//! [`Resolution::test_of`](super::Resolution::test_of)), which lowering
//! wraps the call in. A value of another shape is then the failure "the
//! result of operation `vm.boot` is not a Vm", at the call, labelled with
//! what the value is, and never a value of the wrong type further on.
//!
//! An operation reports its failures at its call, as a function marked
//! `#[track_caller]` does (see [Located at the
//! call](crate::eval::calls#located-at-the-call)): its body is written
//! nowhere, so a failure the host gives, or a result of the wrong shape, is
//! reported where the program called the operation
//! (`Checker::track_operations`).
//!
//! # Handle types
//!
//! A *handle type*, `extern type Vm = "vm.machine"`, is an extensible enum
//! (see [Enums](crate::check::enums#enums)) that no program adds a variant
//! to: a nominal type by its key, which is made of its kind's text alone
//! ([`crate::names::extern_key`]), whose bound is `Int`
//! (`check_enum_bounds`). A *handle*, a value of it, is the value of an
//! enum the host makes ([`Plain::handle`](crate::eval::plain::Plain::handle)):
//! its variant's key is made of the kind too
//! ([`crate::names::handle_key`]), and its payload is the `Int` the host
//! chose. So a handle is everything a value of a bounded enum is, with no
//! rule of its own: it is told apart by `is` and `as`, compared with `==`
//! by its kind and its number, a key of a memoised call, and data, which a
//! record, a list and a rest-field of `*` hold and an operation is given.
//!
//! No program makes a handle or opens one. A variant declared for a handle
//! type, `enum Vm::Fake<Int>`, is "`Vm` is a handle type: only the host
//! makes its values", at the variant's name, labelled at the handle type
//! where it is in the same file (`check_variant_payloads`); it is a variant
//! in error, as any is, so a use of it, `Vm::Fake`, fails the check
//! quietly. With no variant as a value there is nothing to call, and
//! nothing to give `kind.matches(value)` or `kind.payload(value)`, which
//! take one. A
//! handle has no source form: `json.decode` makes none, and `show` and an
//! interpolation give `<vm.machine 3>`, its kind and its number, which is
//! no expression.
//!
//! Two handle types of one kind would be one type under two names, so a
//! kind is declared once in a workspace: a second declaration is "handle
//! kind `vm.machine` is declared twice", at the later, labelled at each,
//! noted "declare a handle type once and import it where it is used" (see
//! [`crate::workspace`], "Handle types").

use crate::ast::{ArgRange, AttributeId, Effect, ExprId, ExprKind};
use crate::diagnostic::{Diagnostic, InternalError};
use crate::source::Span;
use crate::types::{TyId, TyKind};

use super::{Attribute, Checker, Slot, TRACK_CALLER, TestResolution};

/// The note of an operation's parameter or result that is no data.
const DATA_NOTE: &str =
    "the host is given plain data and handles: no function, future, secret or sandbox";

impl Checker<'_> {
    /// Types the call `id` of `callee`, an [`ExprKind::Operation`], with the
    /// arguments `args`, the body the parser gives an operation (see [Host
    /// operations](#host-operations)): its name, a `String`, and a list of
    /// a read of each parameter of the function it is the body of, each
    /// awaited deeply and then data. The call is of the function's declared
    /// result, made present, which must be data, and is recorded with the
    /// test of that type, for lowering to check what the host gives; it is
    /// impure. `Error` if a parameter or the result is erroneous or no
    /// data.
    pub(super) fn check_operation_call(
        &mut self,
        id: ExprId,
        callee: ExprId,
        args: ArgRange,
    ) -> TyId {
        let ast = self.ast;
        let span = ast.get(id).span;
        let parts = match ast.args(args) {
            [name, list] => match (ast.get(name.value).kind, ast.get(list.value).kind) {
                (ExprKind::Str(text), ExprKind::List(elements)) => {
                    Some((name.value, text, list.value, elements))
                }
                _ => None,
            },
            _ => None,
        };
        let result = self.functions.last().and_then(|function| function.result);
        let (Some((name, text, list, elements)), Some((ret, ret_span))) = (parts, result) else {
            self.errors.push(
                InternalError {
                    message: "an operation's call is not the body the parser gives an operation",
                    span,
                }
                .into(),
            );
            return TyId::ERROR;
        };
        self.check_expr(name);
        let mut erroneous = false;
        for element in ast.elements(elements) {
            let found = self.check_expr(element.value);
            let found = self.settle(element.value, found);
            let ty = self.measure_deep(element.value, found);
            erroneous |= ty == TyId::ERROR
                || !self.operation_data(ty, ast.get(element.value).span, "parameter");
        }
        let given = self.types.list(TyId::DATA);
        self.expr_types[list.index()] = given;
        self.impure_calls.insert(id);
        // A result declared a future, at any depth, is no data: the call
        // gives a present value, which would not fit it.
        let present = self.types.present(ret);
        let given_back = match *self.types.kind(ret) {
            _ if self.types.is_subtype(present, ret) => present,
            // As it is written, without a bare type's either shape.
            TyKind::Either(written) => written,
            _ => ret,
        };
        if ret == TyId::ERROR || !self.operation_data(given_back, ret_span, "result") || erroneous {
            return TyId::ERROR;
        }
        self.expr_types[callee.index()] = self.types.fn_type(
            [(None, TyId::STRING), (None, given)],
            present,
            Effect::Impure,
        );
        let resolution = TestResolution {
            test: self.types.test_plan(None, present),
            slot: Slot::NONE,
            subject: format!("the result of operation `{}`", ast.strings().get(text)).into(),
            target: self.with_article(present).into(),
        };
        self.tests.insert(id, resolution);
        present
    }

    /// Whether `ty`, the type of what `what` of an operation, its
    /// "parameter" or its "result", written at `span`, is once present, is
    /// data, which the host can be given or give (see [Host
    /// operations](#host-operations)): if it is not, "an operation's
    /// parameter must be data, found T", at `span`.
    fn operation_data(&mut self, ty: TyId, span: Span, what: &str) -> bool {
        if self.types.is_data(ty) {
            return true;
        }
        let shown = self.in_message(ty);
        let label = format!("this is {}", self.in_label(ty));
        self.errors.push(
            Diagnostic::error(
                format!("an operation's {what} must be data, found {shown}"),
                span,
            )
            .with_label(label)
            .with_note(DATA_NOTE),
        );
        false
    }

    /// Gives every operation the tree declares the attribute
    /// `#[track_caller]`, unless it is written on it, after its own (see
    /// [Host operations](#host-operations)): its body is written nowhere,
    /// so what fails in it is reported at its call.
    pub(super) fn track_operations(&mut self) {
        for operation in self.ast.extern_fns() {
            let attributes = self
                .lowering
                .fn_attributes
                .entry(operation.function)
                .or_default();
            let id = AttributeId::Builtin(TRACK_CALLER);
            if attributes.iter().any(|attribute| attribute.id == id) {
                continue;
            }
            let mut all = std::mem::take(attributes).into_vec();
            all.push(Attribute {
                id,
                args: Box::default(),
                defers: false,
            });
            *attributes = all.into_boxed_slice();
        }
    }
}
