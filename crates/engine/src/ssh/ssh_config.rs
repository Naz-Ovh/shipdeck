//! Reading `~/.ssh/config` the way `ssh` does, for the parts shipdeck uses:
//! `HostName`, `Port`, `User` and `IdentityFile` of a host or alias.
//!
//! Rules kept from `ssh`: lines before the first `Host` apply to every host,
//! a `Host` line applies when one of its patterns matches and no `!pattern`
//! does, `*` and `?` are wildcards, the first value found for a keyword wins,
//! and `Include` files are read in place (missing ones are skipped). `Match`
//! blocks are not understood and never apply.

use std::path::{Path, PathBuf};

const MAX_INCLUDE_DEPTH: usize = 8;

/// What `~/.ssh/config` says about one host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostConfig {
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<PathBuf>,
    /// `ProxyJump` or `ProxyCommand`, which shipdeck cannot follow.
    pub proxy: Option<String>,
}

/// Looks `host` up in `~/.ssh/config`. No file means no settings.
pub fn lookup(host: &str) -> HostConfig {
    match std::env::home_dir() {
        Some(home) => lookup_in(&home.join(".ssh").join("config"), &home, host),
        None => HostConfig::default(),
    }
}

/// Looks `host` up in the config file at `path`, with `home` for `~`.
pub fn lookup_in(path: &Path, home: &Path, host: &str) -> HostConfig {
    let mut found = HostConfig::default();
    read_file(path, host, home, 0, &mut found);
    found.expand(host, home);
    found
}

fn read_file(path: &Path, host: &str, home: &Path, depth: usize, found: &mut HostConfig) {
    if depth > MAX_INCLUDE_DEPTH {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(path) {
        parse(&text, host, home, depth, found);
    }
}

fn parse(text: &str, host: &str, home: &Path, depth: usize, found: &mut HostConfig) {
    // Lines before the first Host or Match apply to every host.
    let mut active = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, rest) = split_keyword(line);
        let args = words(rest);
        let first = args.first().cloned();
        match keyword.to_ascii_lowercase().as_str() {
            "host" => active = host_matches(host, &args),
            "match" => active = false,
            _ if !active => {}
            "include" => {
                for pattern in &args {
                    for file in include_files(pattern, home) {
                        read_file(&file, host, home, depth + 1, found);
                    }
                }
            }
            "hostname" => keep_first(&mut found.host_name, first),
            "user" => keep_first(&mut found.user, first),
            "port" => keep_first(&mut found.port, first.and_then(|p| p.parse().ok())),
            "identityfile" => keep_first(&mut found.identity_file, first.map(PathBuf::from)),
            // `none` turns a proxy off.
            "proxyjump" | "proxycommand"
                if first
                    .as_deref()
                    .is_some_and(|f| !f.eq_ignore_ascii_case("none")) =>
            {
                keep_first(
                    &mut found.proxy,
                    Some(format!("{keyword} {}", args.join(" "))),
                );
            }
            _ => {}
        }
    }
}

fn keep_first<T>(slot: &mut Option<T>, value: Option<T>) {
    if slot.is_none() {
        *slot = value;
    }
}

/// `Keyword value`, `Keyword=value` and `Keyword = value` are all valid.
fn split_keyword(line: &str) -> (&str, &str) {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let rest = line[end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    (&line[..end], rest)
}

/// Splits on whitespace, keeping `"double quoted"` words together.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(current);
    }
    out
}

fn host_matches(host: &str, patterns: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    let mut matched = false;
    for pattern in patterns {
        let pattern = pattern.to_ascii_lowercase();
        if let Some(negated) = pattern.strip_prefix('!') {
            if wildcard(negated, &host) {
                return false;
            }
        } else if wildcard(&pattern, &host) {
            matched = true;
        }
    }
    matched
}

/// `*` matches any run of characters, `?` exactly one.
fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((star_p, star_t)) = star {
            pi = star_p + 1;
            ti = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// The files an `Include` names: relative to `~/.ssh`, with `*` and `?`
/// allowed in the file name.
fn include_files(pattern: &str, home: &Path) -> Vec<PathBuf> {
    let path = expand_home(pattern, home);
    let path = if path.is_absolute() {
        path
    } else {
        home.join(".ssh").join(path)
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .is_some_and(|n| wildcard(&name, &n.to_string_lossy()))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

impl HostConfig {
    /// Applies `%h`, `%d`, `%u`, `%%` and `~` the way `ssh` does.
    fn expand(&mut self, host: &str, home: &Path) {
        if let Some(name) = &self.host_name {
            self.host_name = Some(tokens(name, host, home));
        }
        let remote_host = self.host_name.clone().unwrap_or_else(|| host.to_string());
        if let Some(file) = &self.identity_file {
            let text = tokens(&file.to_string_lossy(), &remote_host, home);
            self.identity_file = Some(expand_home(&text, home));
        }
    }
}

fn tokens(text: &str, host: &str, home: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('h') => out.push_str(host),
            Some('d') => out.push_str(&home.to_string_lossy()),
            Some('u') => {
                let user = if cfg!(windows) { "USERNAME" } else { "USER" };
                out.push_str(&std::env::var(user).unwrap_or_default());
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
#[path = "ssh_config_test.rs"]
mod tests;
