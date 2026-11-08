//! Real Linux server information for the client.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{ServerFacts, ServerFactsRequest};

/// Reads system files and disk information outside the async request loop.
pub(crate) async fn read(request: ServerFactsRequest) -> Result<ServerFacts> {
    tokio::task::spawn_blocking(move || collect(&request)).await?
}

fn collect(request: &ServerFactsRequest) -> Result<ServerFacts> {
    if std::env::consts::OS != "linux" || !Path::new(&request.disk_path).is_absolute() {
        bail!("server facts require Linux and an absolute disk path");
    }
    let os_release = std::fs::read_to_string("/etc/os-release").context("reading Linux version")?;
    let os = os_name(&os_release)?.to_string();
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .context("reading kernel version")?
        .trim()
        .to_string();
    let memory = std::fs::read_to_string("/proc/meminfo").context("reading memory information")?;
    let memory_total = memory_bytes(&memory, "MemTotal")?;
    let memory_available = memory_bytes(&memory, "MemAvailable")?;
    let uptime = std::fs::read_to_string("/proc/uptime").context("reading uptime")?;
    let uptime_seconds = uptime
        .split_whitespace()
        .next()
        .context("missing uptime")?
        .split('.')
        .next()
        .context("invalid uptime")?
        .parse()
        .context("invalid uptime")?;
    let disk_path = existing_path(Path::new(&request.disk_path))?;
    let disk_path = disk_path
        .to_str()
        .context("disk path is not UTF-8")?
        .to_string();
    let (disk_total, disk_available) = disk(&disk_path)?;
    let (tools, notes) = find_tools();
    Ok(ServerFacts {
        os,
        kernel,
        arch: std::env::consts::ARCH.to_string(),
        cpu_count: u32::try_from(std::thread::available_parallelism()?.get())?,
        memory_total,
        memory_available,
        disk_total,
        disk_available,
        disk_path,
        uptime_seconds,
        tools,
        notes,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn existing_path(mut path: &Path) -> Result<&Path> {
    loop {
        match std::fs::metadata(path) {
            Ok(_) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                path = path.parent().context("no existing parent for disk path")?;
            }
            Err(err) => return Err(err).with_context(|| format!("checking {}", path.display())),
        }
    }
}

fn os_name(text: &str) -> Result<&str> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .context("Linux version has no PRETTY_NAME")?;
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        bail!("Linux version is empty");
    }
    Ok(value)
}

fn memory_bytes(text: &str, key: &str) -> Result<u64> {
    let line = text
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)
                .and_then(|value| value.strip_prefix(':'))
        })
        .with_context(|| format!("missing {key}"))?;
    let mut fields = line.split_whitespace();
    let amount: u64 = fields.next().context("missing memory value")?.parse()?;
    if fields.next() != Some("kB") {
        bail!("invalid memory unit for {key}");
    }
    amount.checked_mul(1024).context("memory size overflow")
}

fn disk(path: &str) -> Result<(u64, u64)> {
    let output = Command::new("df")
        .args(["-Pk", "--", path])
        .env("LC_ALL", "C")
        .output()
        .context("running df")?;
    if !output.status.success() {
        bail!(
            "df failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_disk(std::str::from_utf8(&output.stdout)?)
}

fn parse_disk(text: &str) -> Result<(u64, u64)> {
    let line = text.lines().nth(1).context("df returned no filesystem")?;
    let fields: Vec<_> = line.split_whitespace().collect();
    let total: u64 = fields.get(1).context("missing disk total")?.parse()?;
    let available: u64 = fields
        .get(3)
        .context("missing disk availability")?
        .parse()?;
    Ok((
        total.checked_mul(1024).context("disk size overflow")?,
        available.checked_mul(1024).context("disk size overflow")?,
    ))
}

fn find_tools() -> (BTreeMap<String, Option<String>>, Vec<String>) {
    let mut tools = BTreeMap::new();
    let mut notes = Vec::new();
    let paths = if let Some(value) = std::env::var_os("PATH") {
        std::env::split_paths(&value).collect()
    } else {
        notes.push("PATH is not set; installed tools could not be checked".to_string());
        Vec::new()
    };
    for name in [
        "curl",
        "docker",
        "node",
        "pm2",
        "sha256sum",
        "sudo",
        "systemctl",
        "tar",
        "wget",
        "zstd",
    ] {
        let mut found = None;
        for folder in &paths {
            let path = folder.join(name);
            match std::fs::metadata(&path) {
                Ok(metadata) if executable(&metadata) => {
                    found = Some(path.to_string_lossy().into_owned());
                    break;
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => notes.push(format!("could not check {}: {err}", path.display())),
            }
        }
        tools.insert(name.to_string(), found);
    }
    (tools, notes)
}

#[cfg(unix)]
fn executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file()
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
