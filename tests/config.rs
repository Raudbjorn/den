//! Config and paths: Owners come from the config file; missing or malformed
//! config is an error with the file and line, never silently defaulted.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::{
    config::{self, ConfigError, Location},
    paths::{Paths, PathsError},
};
use std::{collections::HashMap, ffi::OsString, path::Path};

const PATH: &str = "/home/me/.config/den/config.toml";

fn parse(text: &str) -> Result<config::Config, ConfigError> {
    config::parse(Path::new(PATH), text)
}

fn location(error: &ConfigError) -> Option<Location> {
    match error {
        ConfigError::Malformed { location, .. } => *location,
        _ => None,
    }
}

#[test]
fn owners_are_read_in_order() {
    let config =
        parse("owners = [\"Raudbjorn\", \"massif-network\", \"AldaOryggi\"]\n").expect("valid");
    assert_eq!(config.owners, ["Raudbjorn", "massif-network", "AldaOryggi"]);
    assert_eq!(config.main_clone_root, None);
    assert!(config.main_clones.is_empty());
}

#[test]
fn later_fields_are_accepted() {
    let config = parse(
        "owners = [\"a\"]\nmain_clone_root = \"/home/me/dev\"\n\n[main_clones]\n\"a/b\" = \"/x/b\"\n",
    )
    .expect("valid");
    assert_eq!(
        config.main_clone_root.as_deref(),
        Some(Path::new("/home/me/dev"))
    );
    assert_eq!(
        config.main_clones.get("a/b").map(|p| p.as_path()),
        Some(Path::new("/x/b"))
    );
}

#[test]
fn malformed_config_names_the_file_and_line() {
    let cases = [
        ("syntax", "# watched\nowners = [\"a\",\n", 2),
        ("wrong type", "\nowners = [\"a\", 3]\n", 2),
        ("typo'd field", "owners = [\"a\"]\n\nonwers = [\"b\"]\n", 3),
        (
            "not a login",
            "owners = [\n  \"good\",\n  \"has space\",\n]\n",
            3,
        ),
        ("duplicate", "owners = [\"Octo\",\n \"octo\"]\n", 2),
        ("empty", "\n\nowners = []\n", 3),
        (
            "relative clone root",
            "owners = [\"a\"]\nmain_clone_root = \"dev\"\n",
            2,
        ),
        (
            "relative Main Clone",
            "owners = [\"a\"]\n\n[main_clones]\n\"a/b\" = \"~/dev/b\"\n",
            4,
        ),
    ];
    for (name, text, line) in cases {
        let error = parse(text).expect_err(name);
        let message = error.to_string();
        assert!(message.starts_with(PATH), "{name}: {message}");
        assert_eq!(
            location(&error).map(|l| l.line),
            Some(line),
            "{name}: {message}"
        );
        assert!(
            message.contains(&format!("line {line}")),
            "{name}: {message}"
        );
    }
}

#[test]
fn a_relative_main_clone_path_is_refused_with_why() {
    let error =
        parse("owners = [\"a\"]\n\n[main_clones]\n\"a/b\" = \"b\"\n").expect_err("relative");
    let message = error.to_string();
    assert!(
        message.ends_with(
            "the Main Clone of a/b must be an absolute path, not \"b\" (den doesn't expand ~ or resolve paths against the folder it runs in)"
        ),
        "{message}"
    );
}

#[test]
fn missing_owners_is_malformed_and_names_the_file() {
    let error = parse("main_clone_root = \"/x\"\n").expect_err("no owners");
    assert!(matches!(error, ConfigError::Malformed { .. }), "{error:?}");
    let message = error.to_string();
    assert!(
        message.starts_with(PATH) && message.contains("owners"),
        "{message}"
    );
}

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("den/config.toml");
    let error = config::load(&path).expect_err("missing");
    assert_eq!(error, ConfigError::Missing { path: path.clone() });
    assert!(error.to_string().contains(&path.display().to_string()));
}

#[test]
fn a_file_on_disk_is_loaded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "owners = [\"octo\"]\n").expect("write");
    assert_eq!(config::load(&path).expect("loads").owners, ["octo"]);
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn paths_follow_xdg_and_fall_back_to_home() {
    let xdg = Paths::from_env(env(&[
        ("HOME", "/home/me"),
        ("XDG_CONFIG_HOME", "/cfg"),
        ("XDG_STATE_HOME", "/state"),
    ]))
    .expect("paths");
    assert_eq!(xdg.config_file, Path::new("/cfg/den/config.toml"));
    assert_eq!(xdg.log_file(), Path::new("/state/den/den.log"));

    let home = Paths::from_env(env(&[("HOME", "/home/me"), ("XDG_STATE_HOME", "relative")]))
        .expect("paths");
    assert_eq!(
        home.config_file,
        Path::new("/home/me/.config/den/config.toml")
    );
    assert_eq!(
        home.state_dir,
        Path::new("/home/me/.local/state/den"),
        "relative XDG ignored"
    );

    assert!(matches!(
        Paths::from_env(env(&[])),
        Err(PathsError::NoHome { .. })
    ));
}
