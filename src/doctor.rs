//! Local diagnostics only: no application startup, remote calls or repairs.
use den::{
    config::{self, ConfigError},
    handoff::{WindowMode, clone, command::KITTY_PROGRAM, prompts},
    launcher::{self, LaunchEnvironment},
    paths::Paths,
    ports::Workspace,
    workspace::GitWorkspace,
};
use std::io::{self, Write};

pub fn run(
    paths: &Paths,
    environment: &LaunchEnvironment,
    out: &mut impl Write,
) -> io::Result<bool> {
    let mut passed = true;
    writeln!(out, "Survey prerequisites:")?;
    let config = config::load(&paths.config_file).and_then(|config| {
        config.check_linear_keys(&paths.config_file, |name| std::env::var_os(name))?;
        Ok(config)
    });
    let config_check = match &config {
        Ok(_) => Ok(()),
        Err(ConfigError::Missing { .. }) => Err(format!(
            "create {:?} with owners = [\"your-login\"]",
            paths.config_file
        )),
        Err(ConfigError::Unreadable { .. }) => Err(format!(
            "cannot read {:?}; check file permissions",
            paths.config_file
        )),
        // Parser messages can quote arbitrary file contents, including pasted secrets.
        Err(ConfigError::Malformed { location, .. }) => Err(format!(
            "invalid configuration in {:?}{}; check the README configuration syntax",
            paths.config_file,
            location.map(|l| format!(", {l}")).unwrap_or_default()
        )),
        Err(ConfigError::MissingLinearKey { .. }) => Err(
            "set the api_key_env variable named by each Linear Workflow in the configuration"
                .to_owned(),
        ),
    };
    passed &= check(out, "configuration", config_check)?;
    passed &= executable(out, environment, "gh")?;

    writeln!(out, "\nLaunch prerequisites:")?;
    let git_available = executable(out, environment, "git")?;
    passed &= git_available;
    passed &= executable(out, environment, "claude")?;
    let prompt_check = match prompts::load(&paths.prompts_file) {
        Ok(prompts) if prompts.problems().next().is_some() => Err(
            "a prompt template is missing a required placeholder; check prompt keys and placeholders"
                .to_owned(),
        ),
        Ok(_) => Ok(()),
        Err(prompts::PromptsError::Unreadable { .. }) => Err(format!(
            "cannot read {:?}; check file permissions",
            paths.prompts_file
        )),
        Err(prompts::PromptsError::Malformed { location, .. }) => Err(format!(
            "invalid prompts in {:?}{}; check prompt keys and placeholders",
            paths.prompts_file,
            location.map(|l| format!(", {l}")).unwrap_or_default()
        )),
    };
    passed &= check(out, "prompts (defaults when absent)", prompt_check)?;
    match launcher::window_mode(environment, environment.find(KITTY_PROGRAM).is_some()) {
        WindowMode::Kitty => {
            passed &= check(out, "Kitty executable", Ok(()))?;
        }
        WindowMode::Clipboard { why } => {
            writeln!(out, "  Clipboard fallback: {why}")?;
            passed &= check(
                out,
                "clipboard executable",
                launcher::clipboard_invocation(environment, |p| environment.find(p).is_some())
                    .map(|_| ()),
            )?;
        }
    }
    if let Ok(config) = config {
        if let Some(root) = &config.main_clone_root {
            passed &= check(
                out,
                &format!("Main Clone search root {root:?}"),
                std::fs::read_dir(root).map(|_| ()).map_err(|_| {
                    "must be a readable directory; fix main_clone_root or its permissions"
                        .to_owned()
                }),
            )?;
        }
        for (repo, path) in &config.main_clones {
            let result = if !path.join(".git").is_dir() {
                Err("must point at a Main Clone with a .git directory, not a worktree".to_owned())
            } else if !git_available {
                Err("cannot inspect the Main Clone without git on PATH".to_owned())
            } else {
                let workspace = GitWorkspace::new();
                workspace.worktrees(path)
                    .map_err(|_| "cannot inspect Git worktrees; check repository ownership and local Git configuration".to_owned())
                    .and_then(|worktrees| {
                        if worktrees.iter().any(|worktree| worktree.path == *path) {
                            Ok(())
                        } else {
                            Err("configured path is not the root of a Git Main Clone".to_owned())
                        }
                    })
                    .and_then(|()| {
                        workspace.remotes(path).map_err(|_| {
                            "cannot read Git remotes; check repository ownership and local Git configuration".to_owned()
                        })
                    })
                    .and_then(|remotes| {
                        clone::remote_for(&remotes, &format!("https://github.com/{repo}"))
                            .map(|_| ())
                            .ok_or_else(|| "no remote matches the configured GitHub Repo; correct [main_clones] or its remote".to_owned())
                    })
            };
            passed &= check(out, &format!("Main Clone {repo:?} at {path:?}"), result)?;
        }
    }
    writeln!(
        out,
        "\nLocal checks only: authentication, remote availability, remote Git access, filesystem writability,"
    )?;
    writeln!(
        out,
        "desktop/clipboard connectivity and Claude workflows were not verified."
    )?;
    writeln!(
        out,
        "No survey, automatic clone search, state files or Job launch was performed."
    )?;
    Ok(passed)
}

fn executable(
    out: &mut impl Write,
    environment: &LaunchEnvironment,
    program: &str,
) -> io::Result<bool> {
    check(
        out,
        program,
        environment
            .find(program)
            .map(|_| ())
            .ok_or_else(|| format!("install {program} and make it executable on PATH")),
    )
}

fn check(out: &mut impl Write, name: &str, result: Result<(), String>) -> io::Result<bool> {
    match result {
        Ok(()) => {
            writeln!(out, "  OK   {name}")?;
            Ok(true)
        }
        Err(message) => {
            writeln!(out, "  FAIL {name}: {message}")?;
            Ok(false)
        }
    }
}
