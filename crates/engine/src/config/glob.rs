//! Globs for `files`, `exclude` and the backup step's lists, matched by the `fast-glob` crate, the
//! syntax of fast-glob and Tailwind: `*` and `?` stay inside one folder,
//! `**` is zero or more folders, `[a-z]` and `[!x]`, `{js,ts}` (nested),
//! and `\` escapes. Names starting with a dot match like any other.
//!
//! On top of that, shipdeck reads:
//! - a leading `!`: the pattern turns around. In `include` it leaves out what
//!   it matches; in `exclude` it keeps what it matches.
//! - a trailing `/`: folders only.
//! - `.`: the source folder itself, so everything in it.
//!
//! A pattern that matches a folder covers everything inside it.

use fast_glob::{ErrorKind, glob_match, validate};

#[derive(Debug, Clone)]
pub struct Pattern {
    /// As written in the config.
    pub text: String,
    /// It started with `!`.
    pub negated: bool,
    /// It ended with `/`.
    dir_only: bool,
    /// What fast-glob matches: without `!`, `./` and a trailing `/`.
    /// `None` for `.`: the source folder itself.
    glob: Option<String>,
    /// The leading folders without wildcards, where any match must be.
    base: Vec<String>,
    /// How many folders deep a match is; `None` when `**` or braces that
    /// could hold a `/` make the depth open-ended.
    depth: Option<usize>,
}

/// Characters fast-glob treats as wildcards or escapes.
const SPECIAL: [char; 5] = ['*', '?', '[', '{', '\\'];

impl Pattern {
    /// Reads one pattern.
    ///
    /// # Errors
    /// Returns a message when the pattern is empty, absolute, climbs out with
    /// `..`, or is not valid fast-glob.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bangs = text.chars().take_while(|c| *c == '!').count();
        let negated = bangs % 2 == 1;
        let mut body = &text[bangs..];
        if body.trim().is_empty() {
            return Err("is empty".into());
        }
        if body.starts_with('/') {
            return Err("starts with /; patterns are relative to their source".into());
        }
        while let Some(rest) = body.strip_prefix("./") {
            body = rest;
        }
        let dir_only = body.len() > 1 && body.ends_with('/');
        let body = body.trim_end_matches('/');
        if body.is_empty() || body == "." {
            return Ok(Self {
                text: text.to_string(),
                negated,
                dir_only: false,
                glob: None,
                base: Vec::new(),
                depth: None,
            });
        }
        if let Err(err) = validate(body) {
            return Err(match err.kind {
                ErrorKind::UnclosedBrace => "has a `{` that is never closed".into(),
                ErrorKind::UnclosedBracket => "has a `[` that is never closed".into(),
                ErrorKind::TrailingBackslash => "ends with a lone \\".into(),
                ErrorKind::BraceNestingTooDeep => "nests braces more than 10 deep".into(),
                ErrorKind::TooManyBraceGroups => "has more than 10 brace groups".into(),
                _ => err.to_string(),
            });
        }
        let segments: Vec<&str> = body.split('/').collect();
        if segments.contains(&"..") {
            return Err("climbs out with `..`".into());
        }
        if segments.iter().any(|s| s.is_empty() || *s == ".") {
            return Err("has an empty or `.` folder in the middle".into());
        }
        let wild = |s: &str| s.contains(SPECIAL);
        let base: Vec<String> = segments
            .iter()
            .take_while(|s| !wild(s))
            .map(std::string::ToString::to_string)
            .collect();
        let rest = &segments[base.len()..];
        let deep = rest.iter().any(|s| s.contains("**") || s.contains('{'));
        Ok(Self {
            text: text.to_string(),
            negated,
            dir_only,
            glob: Some(body.to_string()),
            base,
            depth: (!deep).then_some(segments.len()),
        })
    }

    /// Whether `path` (relative, `/`-separated; empty for the source folder)
    /// is one this pattern names. The `!` is the caller's to apply.
    #[must_use]
    pub fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        match &self.glob {
            None => path.is_empty(),
            Some(glob) => glob_match(glob, path),
        }
    }

    /// Whether something strictly inside the folder `dir` could match, so a
    /// walk knows whether to look inside.
    #[must_use]
    pub fn may_match_below(&self, dir: &str) -> bool {
        if self.glob.is_none() {
            return false;
        }
        let parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
        let shared = self.base.len().min(parts.len());
        if self.base[..shared] != parts[..shared] {
            return false;
        }
        if parts.len() < self.base.len() {
            return true;
        }
        self.depth.is_none_or(|depth| depth > parts.len())
    }

    /// The path this pattern names when it has no wildcards and no `!`.
    #[must_use]
    pub fn literal(&self) -> Option<String> {
        if self.negated {
            None
        } else {
            match &self.glob {
                None => Some(String::new()),
                Some(glob) if glob.contains(SPECIAL) => None,
                Some(glob) => Some(glob.clone()),
            }
        }
    }

    /// `.`: the whole source.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.glob.is_none()
    }
}

/// A source's include and exclude patterns, applied together.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub include: Vec<Pattern>,
    pub exclude: Vec<Pattern>,
}

/// Whether a plain pattern or a `!` pattern matched, for one side.
#[derive(Debug, Clone, Copy, Default)]
struct Matched {
    plain: bool,
    negated: bool,
}

/// What the patterns say about one path, carried down into its folder:
/// a pattern that matches a folder covers everything inside it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verdict {
    include: Matched,
    exclude: Matched,
}

impl Verdict {
    #[must_use]
    pub fn ships(&self) -> bool {
        self.included() && !self.excluded()
    }

    /// An include pattern names it, and no `!` include leaves it out.
    #[must_use]
    pub fn included(&self) -> bool {
        self.include.plain && !self.include.negated
    }

    /// An exclude pattern names it, and no `!` exclude keeps it.
    #[must_use]
    pub fn excluded(&self) -> bool {
        self.exclude.plain && !self.exclude.negated
    }
}

impl Rules {
    /// Reads include and exclude patterns together.
    ///
    /// # Errors
    /// Returns a message naming the pattern that cannot be read.
    pub fn parse(include: &[String], exclude: &[String]) -> Result<Self, String> {
        let parse = |list: &[String]| {
            list.iter()
                .map(|text| Pattern::parse(text).map_err(|why| format!("{text:?} {why}")))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Self {
            include: parse(include)?,
            exclude: parse(exclude)?,
        })
    }

    /// No pattern that brings anything in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.include.iter().any(|p| !p.negated)
    }

    /// The verdict for `path`, given its folder's. `hits` counts, per
    /// include pattern, the paths it matched.
    pub fn judge(&self, parent: Verdict, path: &str, is_dir: bool, hits: &mut [usize]) -> Verdict {
        let mut verdict = parent;
        for (pattern, hit) in self.include.iter().zip(hits.iter_mut()) {
            if pattern.matches(path, is_dir) {
                *hit += 1;
                if pattern.negated {
                    verdict.include.negated = true;
                } else {
                    verdict.include.plain = true;
                }
            }
        }
        for pattern in &self.exclude {
            if pattern.matches(path, is_dir) {
                if pattern.negated {
                    verdict.exclude.negated = true;
                } else {
                    verdict.exclude.plain = true;
                }
            }
        }
        verdict
    }

    /// Whether a walk should look inside the folder `dir`.
    #[must_use]
    pub fn enter(&self, verdict: Verdict, dir: &str) -> bool {
        if verdict.include.negated {
            return false;
        }
        if verdict.exclude.plain
            && !verdict.exclude.negated
            && !self
                .exclude
                .iter()
                .any(|p| p.negated && p.may_match_below(dir))
        {
            return false;
        }
        verdict.include.plain
            || self
                .include
                .iter()
                .any(|p| !p.negated && p.may_match_below(dir))
    }
}

#[cfg(test)]
#[path = "glob_test.rs"]
mod tests;
