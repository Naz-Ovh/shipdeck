//! Which agent build a server needs, and the script that downloads it there.
//! The bytes never cross the SSH channel: the server fetches the release
//! archive over HTTPS itself, checks it, and unpacks it.

use anyhow::{Context, Result, bail};

use crate::ssh::Session;

/// The oldest glibc that gets the gnu build: Ubuntu 20.04.
const GLIBC_FLOOR: (u32, u32) = (2, 31);

/// The CPU of a Linux server that has an agent build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

/// What the server's C library is, and the glibc version when it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    Glibc(u32, u32),
    Musl,
}

/// The release target for this CPU and C library.
#[must_use]
pub fn target(arch: Arch, libc: Libc) -> &'static str {
    let gnu = matches!(libc, Libc::Glibc(major, minor) if (major, minor) >= GLIBC_FLOOR);
    match (arch, gnu) {
        (Arch::X86_64, true) => "x86_64-unknown-linux-gnu",
        (Arch::Aarch64, true) => "aarch64-unknown-linux-gnu",
        (Arch::X86_64, false) => "x86_64-unknown-linux-musl",
        (Arch::Aarch64, false) => "aarch64-unknown-linux-musl",
    }
}

/// Reads `uname -m`.
///
/// # Errors
///
/// Returns a clear error for a CPU with no agent build.
pub fn arch(machine: &str) -> Result<Arch> {
    match machine.trim() {
        "x86_64" | "amd64" => Ok(Arch::X86_64),
        "aarch64" | "arm64" => Ok(Arch::Aarch64),
        "armv7l" | "armv6l" | "arm" => bail!(
            "32-bit ARM ({machine}) needs its own agent build, which shipdeck does not ship yet"
        ),
        other => bail!("shipdeck has no agent for this CPU: {other}"),
    }
}

/// The first `major.minor` in `text`, as in `glibc 2.31`.
#[must_use]
pub fn parse_glibc_version(text: &str) -> Option<(u32, u32)> {
    text.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| !(c.is_ascii_digit() || c == '.'));
        let (major, minor) = word.split_once('.')?;
        let major = major.parse().ok()?;
        let minor = minor
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?;
        Some((major, minor))
    })
}

/// The agent target this server needs.
///
/// # Errors
///
/// Fails when the server is not Linux, its CPU has no build, or its C library
/// cannot be identified.
pub async fn target_for(session: &Session) -> Result<&'static str> {
    let facts = session
        .exec_raw(
            "printf '%s\\n' \"os=$(uname -s)\" \"machine=$(uname -m)\" \
             \"libc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)\" \
             \"ldd=$(ldd --version 2>&1 | head -n 1 || true)\"",
        )
        .await
        .context("asking the server about itself")?;
    let os = field(&facts.stdout, "os").unwrap_or("").trim();
    if os.is_empty() {
        bail!("the server did not say which operating system it runs");
    }
    if os != "Linux" {
        bail!("{os} is not Linux, and the agent runs on Linux only");
    }
    let machine = field(&facts.stdout, "machine").unwrap_or("");
    Ok(target(arch(machine)?, libc_from(&facts.stdout)?))
}

/// The value after `key=` on its own line.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    text.lines().find_map(|line| line.strip_prefix(&prefix))
}

/// glibc names itself; musl is what is left.
fn libc_from(text: &str) -> Result<Libc> {
    if let Some(version) = field(text, "libc").and_then(parse_glibc_version) {
        return Ok(Libc::Glibc(version.0, version.1));
    }
    let ldd = field(text, "ldd").unwrap_or("");
    if ldd.to_ascii_lowercase().contains("musl") {
        return Ok(Libc::Musl);
    }
    if let Some(version) = parse_glibc_version(ldd) {
        return Ok(Libc::Glibc(version.0, version.1));
    }
    bail!("shipdeck could not tell which C library the server uses")
}

/// The release archive's name for `target`.
#[must_use]
pub fn asset_name(target: &str) -> String {
    format!("shipdeck-agent-{target}.tar.gz")
}

/// Where the archive for `target` is published. `SHIPDECK_AGENT_BASE_URL`
/// names another folder, for a mirror or a local test.
#[must_use]
pub fn asset_url(target: &str) -> String {
    asset_url_with(
        std::env::var("SHIPDECK_AGENT_BASE_URL").ok().as_deref(),
        target,
    )
}

fn asset_url_with(base: Option<&str>, target: &str) -> String {
    let name = asset_name(target);
    if let Some(base) = base.filter(|base| !base.trim().is_empty()) {
        return format!("{}/{name}", base.trim().trim_end_matches('/'));
    }
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    format!(
        "{repository}/releases/download/v{}/{name}",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
#[path = "binary_test.rs"]
mod tests;
