//! Running external programs: a program that leaves a background child holding
//! its output (as kitty `--detach` and clipboard tools may) must not hang den.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use den::process::{Invocation, OUTPUT_GRACE_MS, ProcessError, run_invocation};
use std::time::{Duration, Instant};

/// Comfortably longer than the grace period, so a hang would show.
const BACKGROUND_SLEEP_SECS: u64 = 10;

#[test]
fn a_child_that_leaves_a_background_process_holding_its_output_returns_after_the_grace_period() {
    let invocation = Invocation::new("sh").args([
        "-c",
        &format!("echo started; sleep {BACKGROUND_SLEEP_SECS} & exit 0"),
    ]);
    let began = Instant::now();
    let output = run_invocation(&invocation, &[], Duration::from_secs(30)).expect("runs");
    let took = began.elapsed();
    assert!(output.status.success());
    assert!(output.output_left_open);
    assert_eq!(output.stdout_text(), "started");
    assert!(
        took < Duration::from_millis(OUTPUT_GRACE_MS) + Duration::from_secs(3),
        "took {took:?}"
    );
}

#[test]
fn environment_and_stdin_reach_the_child_and_discarded_output_is_not_read() {
    let invocation = Invocation::new("sh")
        .args(["-c", "test \"$DEN_TEST\" = yes && cat > /dev/null"])
        .env("DEN_TEST", "yes")
        .discard_output();
    let output = run_invocation(&invocation, b"text", Duration::from_secs(10)).expect("runs");
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && !output.output_left_open);
}

#[test]
fn a_missing_program_and_a_timeout_are_errors_naming_the_program() {
    let missing = run_invocation(
        &Invocation::new("den-no-such-program"),
        &[],
        Duration::from_secs(1),
    );
    assert!(
        matches!(missing, Err(ProcessError::NotFound { program }) if program == "den-no-such-program")
    );
    let slow = run_invocation(
        &Invocation::new("sleep").arg("5"),
        &[],
        Duration::from_millis(100),
    );
    assert!(matches!(slow, Err(ProcessError::TimedOut { program, .. }) if program == "sleep"));
}
