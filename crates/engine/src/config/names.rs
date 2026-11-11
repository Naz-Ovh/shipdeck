//! Names and small value helpers the checks and the pipeline share.

use std::path::Path;

/// The arguments after a program, split the way systemd splits
/// `ExecStart=`: at spaces, with `"..."` or `'...'` kept as one argument.
/// `None` when a quote is never closed.
#[must_use]
pub fn split_args(text: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, c) if c.is_whitespace() => args.extend(word.take()),
            (_, c) => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    args.extend(word);
    Some(args)
}

/// A systemd unit name without `.service`: `my-api`, `worker@1`.
#[must_use]
pub fn is_unit_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._@-".contains(c))
}

/// A user, app or deploy name: letters, digits, `.`, `_` and `-`.
#[must_use]
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// `node`, `/usr/bin/node` or `bin/server`: safe in a shell word and a unit
/// line, and never climbing out with `..`.
#[must_use]
pub fn is_program(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-/".contains(c))
        && !text.split('/').any(|part| part == "..")
}

/// A release folder on the server: absolute, and made of plain names.
#[must_use]
pub fn is_safe_dir(dir: &str) -> bool {
    dir.starts_with('/')
        && dir.len() > 1
        && dir
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        && !dir.split('/').any(|part| part == "..")
}

/// A path's file name.
#[must_use]
pub fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(
        || path.to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `1 command`, `3 commands`.
#[must_use]
pub fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}
