#![cfg(unix)]
// Test helpers fail loudly, as in the existing adapter tests.
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("config/den")).unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        let fixture = Self { root };
        fixture.config("owners = [\"example-owner\"]\n");
        for program in ["gh", "git", "claude", "kitty"] {
            // Any invocation is a test failure, even if its output gets ignored.
            let path = fixture.root.path().join("bin").join(program);
            fs::write(
                &path,
                "#!/bin/sh\nprintf invoked > \"$HOME/invoked\"\nexit 99\n",
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fixture
    }

    fn config(&self, text: &str) {
        fs::write(self.root.path().join("config/den/config.toml"), text).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_den"));
        command
            .env_clear()
            .env("HOME", self.root.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("PATH", self.root.path().join("bin"))
            .env("KITTY_WINDOW_ID", "1")
            .env("DEN_GRAPHICS", "invalid-so-normal-startup-would-fail");
        command
    }

    fn doctor(&self) -> Output {
        let output = self.command().arg("--doctor").output().unwrap();
        assert!(!self.root.path().join("state").exists());
        assert!(!self.root.path().join("invoked").exists());
        output
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn help_and_version_need_no_environment_or_configuration() {
    for (flag, expected) in [
        ("--help", "--doctor"),
        ("--version", concat!("den ", env!("CARGO_PKG_VERSION"))),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_den"))
            .env_clear()
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(stdout(&output).contains(expected));
    }
}

#[test]
fn local_success_uses_default_prompts_without_invoking_tools_or_writing_state() {
    let fixture = Fixture::new();
    let output = fixture.doctor();
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("OK   prompts (defaults when absent)"));
    assert!(stdout(&output).contains("authentication, remote availability"));
    assert!(!fixture.root.path().join("config/den/prompts.toml").exists());
}

#[test]
fn missing_or_malformed_config_fails_without_echoing_contents() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.path().join("config/den/config.toml")).unwrap();
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("create"));
    fixture.config("owners = [\"secret-sentinel!\"]");
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("invalid configuration"));
    assert!(!stdout(&output).contains("secret-sentinel"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-sentinel"));
}

#[test]
fn malformed_prompts_fail_without_echoing_contents() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.path().join("config/den/prompts.toml"),
        "implement = '{secret-sentinel}'",
    )
    .unwrap();
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("invalid prompts"));
    assert!(!stdout(&output).contains("secret-sentinel"));
}

#[test]
fn missing_and_non_executable_programs_are_reported_together() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.path().join("bin/gh")).unwrap();
    fs::set_permissions(
        fixture.root.path().join("bin/claude"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("FAIL gh"));
    assert!(text.contains("FAIL claude"));
}

#[test]
fn clipboard_fallback_requires_an_available_tool_but_never_runs_it() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .arg("--doctor")
        .env_remove("KITTY_WINDOW_ID")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("no graphical session"));
    fs::copy(
        fixture.root.path().join("bin/kitty"),
        fixture.root.path().join("bin/wl-copy"),
    )
    .unwrap();
    let output = fixture
        .command()
        .arg("--doctor")
        .env_remove("KITTY_WINDOW_ID")
        .env("WAYLAND_DISPLAY", "wayland-0")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("OK   clipboard executable"));
    assert!(!fixture.root.path().join("invoked").exists());
    assert!(!fixture.root.path().join("state").exists());
}

#[test]
fn configured_clone_and_search_root_must_exist() {
    let fixture = Fixture::new();
    let missing = fixture.root.path().join("missing");
    fixture.config(&format!("owners = ['example-owner']\nmain_clone_root = {missing:?}\n[main_clones]\n'example-owner/app' = {missing:?}\n"));
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("must be a readable directory"));
    assert!(stdout(&output).contains("must point at a Main Clone"));
}

#[test]
fn configured_clone_needs_a_matching_remote_without_exposing_credentials() {
    let fixture = Fixture::new();
    let clone = fixture.root.path().join("clone");
    let git = den::launcher::LaunchEnvironment::from_env(|name| std::env::var_os(name))
        .find("git")
        .unwrap();
    assert!(
        Command::new(&git)
            .args(["init", "--quiet"])
            .arg(&clone)
            .status()
            .unwrap()
            .success()
    );
    let run_git = |url: &str| {
        assert!(
            Command::new(&git)
                .arg("-C")
                .arg(&clone)
                .args(["config", "remote.origin.url", url])
                .status()
                .unwrap()
                .success()
        );
    };
    fs::remove_file(fixture.root.path().join("bin/git")).unwrap();
    std::os::unix::fs::symlink(&git, fixture.root.path().join("bin/git")).unwrap();
    fixture.config(&format!(
        "owners = ['example-owner']\n[main_clones]\n'example-owner/app' = {clone:?}\n"
    ));
    run_git("https://secret-sentinel@github.com/example-owner/other");
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("no remote matches"));
    assert!(!stdout(&output).contains("secret-sentinel"));
    run_git("https://secret-sentinel@github.com/example-owner/app.git");
    let before = fs::read(clone.join(".git/config")).unwrap();
    let output = fixture.doctor();
    assert!(output.status.success(), "{output:?}");
    assert!(!stdout(&output).contains("secret-sentinel"));
    assert_eq!(fs::read(clone.join(".git/config")).unwrap(), before);
}

#[test]
fn diagnostic_flags_do_not_swallow_unexpected_arguments() {
    let fixture = Fixture::new();
    for flag in ["--help", "--version", "--doctor"] {
        let output = fixture.command().args([flag, "--yes"]).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    assert!(!fixture.root.path().join("state").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn diagnostic_output_failure_is_reported_without_a_panic() {
    let fixture = Fixture::new();
    for flag in ["--help", "--version", "--doctor"] {
        let output = fixture
            .command()
            .arg(flag)
            .stdout(
                fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .unwrap(),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("writing output"));
    }
    assert!(!fixture.root.path().join("state").exists());
}
