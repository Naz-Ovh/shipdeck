//! A release's environment file, read as systemd's `EnvironmentFile=` reads
//! it: comments and blank lines skipped, an optional `export`, one pair of
//! surrounding quotes removed, and nothing expanded.

use std::collections::BTreeMap;

/// The pairs the file sets, in file order; a later pair wins.
pub(crate) fn parse(bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty()
            || key.starts_with(|ch: char| ch.is_ascii_digit())
            || !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            continue;
        }
        values.insert(key.to_string(), unquote(value));
    }
    values
}

/// Removes one matching pair of surrounding quotes, as the shell prelude does.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && first == last {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
#[path = "env_file_tests.rs"]
mod tests;
