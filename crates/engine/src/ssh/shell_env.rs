//! The environment a terminal on the server would have. An SSH command
//! runs without a terminal, so the login shell skips the files people set
//! their tools up in (`~/.bashrc`, `~/.zshrc`, `config.fish`, ...): nvm, a
//! language's version manager, a PATH entry. Once per session shipdeck
//! starts the login shell the way `ssh <host>` does for a terminal,
//! interactive and login, and keeps what it exports for every command.

use std::time::Duration;

/// How long the shell's startup files may take.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Starts `$SHELL -i -l` (bash, zsh, fish, ... each read their own files),
/// which runs `sh -c "export -p"`. POSIX prints the exported variables in a
/// form `sh` reads back, so the result goes in front of every script as it
/// is. The shell's own output and input are cut off, so a startup file
/// that prints or reads cannot get in the way.
pub const CAPTURE: &str = r#"f=$(mktemp) || exit 1
SHIPDECK_ENV_FILE="$f" "${SHELL:-/bin/sh}" -i -l -c 'sh -c "export -p" > "$SHIPDECK_ENV_FILE"' </dev/null >/dev/null 2>&1
code=$?
cat "$f"
rm -f "$f"
exit "$code"
"#;

/// Whether commands on a session get the login shell's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEnv {
    Loaded,
    /// `ssh.shell_env: false`, or `--no-shell-env`.
    Off,
    /// The startup files failed or took too long; commands run without them.
    Failed(String),
}

/// What [`CAPTURE`] printed, as a prelude for scripts, or why it cannot be.
pub fn prelude(code: u32, stdout: &str) -> Result<String, String> {
    if code != 0 {
        return Err(format!(
            "the login shell's startup files failed (exit {code})"
        ));
    }
    if stdout.trim().is_empty() {
        return Err("the login shell exported nothing".into());
    }
    let mut prelude = stdout.to_string();
    if !prelude.ends_with('\n') {
        prelude.push('\n');
    }
    Ok(prelude)
}

// The tests run the server's script with this machine's sh and bash.
#[cfg(all(test, unix))]
#[path = "shell_env_test.rs"]
mod tests;
