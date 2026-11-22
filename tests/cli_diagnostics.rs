#![cfg(unix)]
// Test helpers fail loudly, as in the existing adapter tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use den::{
    cache::{CachedObservation, FileCache},
    ports::ObservationCache,
};

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};
use support::build::{ME, ObservationBuilder, pr};
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
        // Started outside any clone, so den doesn't run (the fake) git to
        // find which Repo it is in.
        command
            .current_dir(self.root.path())
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

/// The real git on the test's own PATH.
fn real_git() -> std::path::PathBuf {
    den::launcher::LaunchEnvironment::from_env(|name| std::env::var_os(name))
        .find("git")
        .unwrap()
}

/// A Main Clone at `dir` whose `origin` is `url`, made with the real git.
fn clone_with_origin(dir: &std::path::Path, url: &str) {
    let git = real_git();
    for args in [
        vec!["init", "--quiet", "."],
        vec!["config", "remote.origin.url", url],
    ] {
        fs::create_dir_all(dir).unwrap();
        assert!(
            Command::new(&git)
                .current_dir(dir)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
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
fn a_linear_workflow_needs_its_key_without_echoing_the_value() {
    let fixture = Fixture::new();
    fixture.config("owners = ['example-owner']\n[linear.example-owner]\nworkspace = 'acme'\napi_key_env = 'DEN_TEST_LINEAR_KEY'\n");
    let missing = fixture.doctor();
    assert_eq!(missing.status.code(), Some(1));
    assert!(stdout(&missing).contains("api_key_env"));
    let present = fixture
        .command()
        .arg("--doctor")
        .env("DEN_TEST_LINEAR_KEY", "secret-sentinel")
        .output()
        .unwrap();
    assert!(present.status.success(), "{present:?}");
    assert!(!stdout(&present).contains("secret-sentinel"));
    assert!(!String::from_utf8_lossy(&present.stderr).contains("secret-sentinel"));
    assert!(!fixture.root.path().join("state").exists());
    assert!(!fixture.root.path().join("invoked").exists());
}

#[test]
fn an_unusable_request_review_template_fails_diagnostics() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.path().join("config/den/prompts.toml"),
        "request_review = '/secret-sentinel {url}'",
    )
    .unwrap();
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("required placeholder"));
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

    // Git discovers the parent repository when a nested .git directory is invalid.
    let nested = clone.join("nested");
    fs::create_dir_all(nested.join(".git")).unwrap();
    fixture.config(&format!(
        "owners = ['example-owner']\n[main_clones]\n'example-owner/app' = {nested:?}\n"
    ));
    let output = fixture.doctor();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("not the root of a Git Main Clone"));
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
        // Launch and diagnostic output now share the stdout-specific error wording.
        assert!(String::from_utf8_lossy(&output.stderr).contains("writing to stdout"));
    }
    assert!(!fixture.root.path().join("state").exists());
}

#[test]
fn dump_and_launch_report_a_closed_stdout_pipe_without_panicking() {
    let fixture = Fixture::new();
    fixture.config("owners = ['octo']\n");
    let cached = CachedObservation::fresh(
        ObservationBuilder::new()
            .owner("octo")
            .repo("octo/app")
            .pull_request(pr("octo/app", 7).author(ME).changes_requested())
            .build(),
    );
    FileCache::in_state_dir(&fixture.root.path().join("state/den"))
        .store(&cached)
        .unwrap();

    for args in [["--dump", "80x24"], ["--launch", "octo/app#7"]] {
        // Close the reader before spawning, so the first stdout write reliably fails.
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let output = fixture
            .command()
            .args(args)
            .env("DEN_GRAPHICS", "off")
            .stdout(writer)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {error}");
        assert!(error.contains("writing to stdout"), "{args:?}: {error}");
        assert!(error.contains("Broken pipe"), "{args:?}: {error}");
        assert!(!error.contains("panicked"), "{args:?}: {error}");
    }
    assert!(!fixture.root.path().join("state/den/jobs.json").exists());
}

/// `--doctor` says what `den` would open in the start directory (spec 0003,
/// "CLI"); outside any clone it runs no git to find out (the fixture's fake
/// git records any call, and `doctor()` fails on it).
#[test]
fn doctor_says_what_the_start_directory_opens() {
    let fixture = Fixture::new();
    let output = fixture.doctor();
    assert!(output.status.success(), "{output:?}");
    let text = stdout(&output);
    assert!(text.contains("\nStart directory:\n"), "{text}");
    assert!(
        text.contains(&format!(
            "OK   start directory {:?} opens the watched Owners\n",
            fs::canonicalize(fixture.root.path()).unwrap()
        )),
        "{text}"
    );
}

/// Inside a clone whose `origin` is on GitHub, `--doctor` names that Repo,
/// as `den` there would open it (startup rule 3), without the URL's
/// credentials.
#[test]
fn doctor_names_the_repo_a_clone_opens() {
    let fixture = Fixture::new();
    let clone = fixture.root.path().join("clone");
    clone_with_origin(
        &clone,
        "https://secret-sentinel@github.com/example-owner/app.git",
    );
    fs::remove_file(fixture.root.path().join("bin/git")).unwrap();
    std::os::unix::fs::symlink(real_git(), fixture.root.path().join("bin/git")).unwrap();
    let output = fixture
        .command()
        .current_dir(&clone)
        .arg("--doctor")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "OK   start directory {:?} opens Repo example-owner/app\n",
            fs::canonicalize(&clone).unwrap()
        )),
        "{text}"
    );
    assert!(!text.contains("secret-sentinel"), "{text}");
}

/// Inside a clone, which Repo it is takes git; without git on PATH the
/// check fails and says so, rather than claiming the watched Owners.
#[test]
fn doctor_cannot_tell_a_clones_repo_without_git() {
    let fixture = Fixture::new();
    let clone = fixture.root.path().join("clone");
    clone_with_origin(&clone, "https://github.com/example-owner/app.git");
    fs::remove_file(fixture.root.path().join("bin/git")).unwrap();
    let output = fixture
        .command()
        .current_dir(&clone)
        .arg("--doctor")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = stdout(&output);
    assert!(text.contains("without git on PATH"), "{text}");
    assert!(!text.contains("opens"), "{text}");
}
