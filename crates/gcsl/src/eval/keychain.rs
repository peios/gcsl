//! The keychain's built-ins: secrets by name, for the `keychain` module.
//!
//! # Keychains
//!
//! `keychain_string(name, {profile; file})` is the secret string of the
//! name `name` for the profile `profile`, and `keychain_file` the secret
//! file: what the `keychain` module's `string` and `file` give. Its source
//! is, in order:
//!
//! - the file at `file`, an absolute path, if it is not empty: the path the
//!   keychain's configuration gives the name;
//! - else the file at the path of `$GCSL_SECRET_FILE_<NAME>`, `NAME` the
//!   name in capitals, every character that is no letter or digit `_`;
//! - else, for a string alone, the value of `$GCSL_SECRET_<NAME>`;
//!
//! and with none of them, "no secret `api-token` for the profile `dev`:
//! configure it in the keychain, or set `$GCSL_SECRET_API_TOKEN`". A
//! string from a file is its contents, a newline at the end left out, and
//! must be UTF-8. A file is read too, for its identity.
//!
//! A secret's identity is the host's ([`Effects::keychain`]), and by default
//! `profile/name`; a file's path is the host's, the path its programs find
//! it at, and by default the file's own. No message names a secret's
//! contents.

use std::path::Path;
use std::rc::Rc;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{File, Record, Secret, Value};

use super::effects::{Effects, KeychainAnswer, KeychainSecret};
use super::{Flow, mistyped};

/// Where a keychain's secret comes from (see [Keychains](#keychains)).
enum Source {
    /// A file, at its path as GCSL code names it.
    File(String),
    /// A value, an environment variable's.
    Value(Vec<u8>),
}

/// Applies `builtin`, `keychain_string` or `keychain_file`, for the call
/// `node`, to the name `name` and the record `from` of its profile and its
/// configured file (see [Keychains](#keychains)).
pub(super) fn keychain(
    builtin: Builtin,
    name: &str,
    from: &Record,
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let field = |wanted: &str| {
        from.layout
            .texts
            .iter()
            .position(|text| &**text == wanted)
            .and_then(|at| match from.values.get(at) {
                Some(Value::Str(text)) => Some(Rc::clone(text)),
                _ => None,
            })
    };
    let (Some(profile), Some(file)) = (field("profile"), field("file")) else {
        return mistyped(
            node,
            "a keychain's secret asked for with no profile or file",
        );
    };
    let string = builtin == Builtin::KeychainString;
    let variable = variable_of(name);
    let source = if !file.is_empty() {
        Source::File(file.to_string())
    } else if let Some(path) = std::env::var_os(format!("GCSL_SECRET_FILE_{variable}")) {
        Source::File(path.to_string_lossy().into_owned())
    } else if let Some(value) =
        std::env::var_os(format!("GCSL_SECRET_{variable}")).filter(|_| string)
    {
        Source::Value(value.into_encoded_bytes())
    } else {
        let set = if string { "" } else { "FILE_" };
        return fail(format!(
            "no secret `{name}` for the profile `{profile}`: configure it in the keychain, or \
             set `$GCSL_SECRET_{set}{variable}`"
        ));
    };
    let (contents, path) = match source {
        Source::Value(value) => (value, None),
        Source::File(path) => {
            if !Path::new(&path).is_absolute() {
                return fail(format!(
                    "the secret `{name}`'s file must be at an absolute path, not `{path}`"
                ));
            }
            let read = effects
                .host_path(&path)
                .and_then(|host| std::fs::read(host).map_err(|error| error.to_string()));
            match read {
                Ok(contents) => (contents, Some(path)),
                Err(reason) => {
                    return fail(format!(
                        "cannot read the secret `{name}`'s file `{path}`: {reason}"
                    ));
                }
            }
        }
    };
    let asked = KeychainSecret {
        name,
        profile: &profile,
        contents: &contents,
        file: path.as_deref(),
    };
    let answer = match effects.keychain(&asked) {
        Some(Ok(answer)) => answer,
        Some(Err(reason)) => return fail(format!("the secret `{name}`: {reason}")),
        None => KeychainAnswer {
            identity: format!("{profile}/{name}"),
            path: path.clone(),
        },
    };
    let value = if string {
        let text = contents.strip_suffix(b"\n").unwrap_or(&contents);
        match std::str::from_utf8(text) {
            Ok(text) => Value::Str(Rc::from(text)),
            Err(_) => return fail(format!("the secret `{name}` is not UTF-8 text")),
        }
    } else {
        let Some(path) = answer.path.or(path) else {
            return mistyped(node, "a keychain's secret file with no path");
        };
        Value::File(Rc::new(File {
            path: Rc::from(path),
            identity: None,
        }))
    };
    Ok(Value::Secret(Rc::new(Secret {
        value,
        identity: Rc::from(answer.identity),
        revealed: false,
    })))
}

/// The name `name` as it is in a keychain's environment variables: in
/// capitals, each character that is no ASCII letter or digit `_`.
fn variable_of(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}
