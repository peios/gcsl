//! Tests of the keychain: secrets by name, from a configured file or the
//! environment, their identities and paths the host's (see
//! [Keychains](crate::eval::keychain#keychains)).

use std::path::PathBuf;

use super::*;
use crate::eval::Effects;
use crate::eval::effects::{KeychainAnswer, KeychainSecret};
use crate::eval::Direct;
use crate::workspace::tests::effects::run_through;
use crate::workspace::tests::fs::fs_scratch;

/// A host that answers every keychain's secret itself: its identity of its
/// name and contents' length, and a file at `/keys/NAME`.
#[derive(Debug)]
struct Keeper;

impl Effects for Keeper {
    fn keychain(&self, secret: &KeychainSecret<'_>) -> Option<Result<KeychainAnswer, String>> {
        if secret.name == "refused" {
            return Some(Err("this host keeps no such secret".to_owned()));
        }
        Some(Ok(KeychainAnswer {
            identity: format!("kept:{}:{}", secret.name, secret.contents.len()),
            path: secret.file.map(|_| format!("/keys/{}", secret.name)),
        }))
    }

    fn temp_dir(&self) -> Option<Result<String, String>> {
        None
    }
}

/// The identity and value of `value`, a secret.
fn secret_of(value: &Value) -> (String, Value) {
    let Value::Secret(secret) = value else {
        panic!("not a secret: {value}");
    };
    (secret.identity.to_string(), secret.value.clone())
}

/// A key file of `contents` in a scratch directory of `test`, by its path.
fn key_file(test: &str, contents: &str) -> String {
    let dir = PathBuf::from(fs_scratch(test));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("key.pem");
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn a_configured_secret_is_its_file_of_its_profile() {
    let key = key_file("keychain-file", "dev key\n");
    let keys = format!(
        "[{{profile = \"dev\"; name = \"signing\"; file = \"{key}\"}}, \
         {{profile = \"release\"; name = \"signing\"; file = \"/nowhere\"}}]"
    );
    let direct = Direct { temp_dirs: None };
    // A file: its own path, identified by its profile and name.
    let text =
        format!("import keychain\n= keychain.file(\"signing\") with {{ keychain_keys = {keys} }}");
    let (identity, value) = secret_of(&run_through(&text, &direct).unwrap());
    assert_eq!(identity, "dev/signing");
    let Value::File(file) = value else {
        panic!("{value}");
    };
    assert_eq!(&*file.path, key);
    // A string: the file's contents, the newline at the end left out.
    let text = format!(
        "import keychain\n= keychain.string(\"signing\") with {{ keychain_keys = {keys} }}"
    );
    let (identity, value) = secret_of(&run_through(&text, &direct).unwrap());
    assert_eq!(
        (identity.as_str(), value),
        ("dev/signing", Value::Str(Rc::from("dev key")))
    );
    // Shown, it is a secret still.
    let text = format!(
        "import keychain\nlet key = keychain.string(\"signing\") with {{ keychain_keys = {keys} }}\n= \"{{key}}\""
    );
    assert_eq!(run_through(&text, &direct).unwrap().to_string(), "***");
    // Another profile's file is its own, which here is not there.
    let text = format!(
        "import keychain\n= keychain.file(\"signing\") with {{ keychain_keys = {keys}; \
         keychain_profile = \"release\" }}"
    );
    let error = run_through(&text, &direct).unwrap_err();
    assert!(
        error
            .message
            .starts_with("cannot read the secret `signing`'s file `/nowhere`: "),
        "{}",
        error.message
    );
}

#[test]
fn a_secret_nothing_gives_names_where_it_could_be_given() {
    let direct = Direct { temp_dirs: None };
    let text = "import keychain\n= keychain.string(\"gcsl-test-unset-7f3a\")";
    let error = run_through(text, &direct).unwrap_err();
    assert_eq!(
        error.message,
        "no secret `gcsl-test-unset-7f3a` for the profile `dev`: configure it in the keychain, \
         or set `$GCSL_SECRET_GCSL_TEST_UNSET_7F3A`"
    );
    // At the call, not in the module.
    let at = u32::try_from(text.find("keychain.string").unwrap()).unwrap();
    assert_eq!(error.span.start, at, "{error:?}");
    let error = run_through(
        "import keychain\n= keychain.file(\"gcsl-test-unset-7f3a\")",
        &direct,
    )
    .unwrap_err();
    assert!(
        error
            .message
            .ends_with("or set `$GCSL_SECRET_FILE_GCSL_TEST_UNSET_7F3A`"),
        "{}",
        error.message
    );
}

#[test]
fn a_host_gives_a_secret_its_identity_and_its_file_s_path() {
    let key = key_file("keychain-host", "12345");
    let keys = format!("[{{profile = \"dev\"; name = \"signing\"; file = \"{key}\"}}]");
    let text =
        format!("import keychain\n= keychain.file(\"signing\") with {{ keychain_keys = {keys} }}");
    let (identity, value) = secret_of(&run_through(&text, &Keeper).unwrap());
    assert_eq!(identity, "kept:signing:5");
    let Value::File(file) = value else {
        panic!("{value}");
    };
    assert_eq!(&*file.path, "/keys/signing");
    // A host's refusal is the call's error.
    let keys = format!("[{{profile = \"dev\"; name = \"refused\"; file = \"{key}\"}}]");
    let text =
        format!("import keychain\n= keychain.file(\"refused\") with {{ keychain_keys = {keys} }}");
    let error = run_through(&text, &Keeper).unwrap_err();
    assert_eq!(
        error.message,
        "the secret `refused`: this host keeps no such secret"
    );
}
