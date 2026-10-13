//! What a built binary needs from the server: its CPU architecture and the
//! oldest glibc it can run on.

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use object::{Architecture, Object, ObjectSection};

#[derive(Debug, Clone)]
pub struct BinaryInfo {
    /// In `uname -m` spelling: `x86_64`, `aarch64`, ...
    pub arch: String,
    /// The newest `GLIBC_x.y` symbol version it links against; `None` for a
    /// static or musl binary.
    pub glibc: Option<GlibcVersion>,
    pub size: u64,
}

pub fn inspect(path: &Path) -> Result<BinaryInfo> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let file = object::File::parse(&*data).with_context(|| {
        format!(
            "{} is not an executable this tool understands",
            path.display()
        )
    })?;
    let glibc = file
        .section_by_name(".dynstr")
        .and_then(|section| section.data().ok())
        .and_then(newest_glibc);
    Ok(BinaryInfo {
        arch: arch_name(file.architecture()).to_string(),
        glibc,
        size: data.len() as u64,
    })
}

fn arch_name(arch: Architecture) -> &'static str {
    match arch {
        Architecture::X86_64 => "x86_64",
        Architecture::Aarch64 => "aarch64",
        Architecture::Arm => "arm",
        Architecture::I386 => "i686",
        Architecture::Riscv64 => "riscv64",
        Architecture::PowerPc64 => "ppc64le",
        Architecture::S390x => "s390x",
        _ => "unknown",
    }
}

/// Maps `uname -m` output onto the names `arch_name` uses.
pub fn normalize_arch(uname: &str) -> String {
    match uname.trim() {
        "amd64" => "x86_64".into(),
        "arm64" => "aarch64".into(),
        a if a.starts_with("armv") => "arm".into(),
        "i386" | "i486" | "i586" => "i686".into(),
        a => a.into(),
    }
}

/// The highest `GLIBC_x.y` in a dynamic string table.
fn newest_glibc(dynstr: &[u8]) -> Option<GlibcVersion> {
    dynstr
        .split(|&b| b == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .filter_map(|s| s.strip_prefix("GLIBC_"))
        .filter_map(GlibcVersion::parse)
        .max()
}

/// A glibc version such as `2.31`, compared number by number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion(Vec<u32>);

impl GlibcVersion {
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Option<Vec<u32>> = text.trim().split('.').map(|p| p.parse().ok()).collect();
        parts.filter(|p| !p.is_empty()).map(Self)
    }
}

impl fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

#[cfg(test)]
#[path = "binary_test.rs"]
mod tests;
