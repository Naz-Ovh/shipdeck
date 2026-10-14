//! Globs for `pipeline.pack.files`, matched by the `fast-glob` crate, the
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
    /// As written in deploy.yaml.
    pub text: String,
    /// It started with `!`.
    pub negated: bool,
    /// It ended with `/`.
    dir_only: bool,
    /// `.`: the source folder itself.
    root: bool,
    /// What fast-glob matches: without `!`, `./` and a trailing `/`.
    glob: String,
    /// The leading folders without wildcards, where any match must be.
    base: Vec<String>,
    /// After `base`: whether `**`, or braces that could hold a `/`, make the
    /// depth open-ended.
    deep: bool,
    /// How many folders deep a match is, when `deep` is false.
    depth: usize,
}

/// Characters fast-glob treats as wildcards or escapes.
const SPECIAL: [char; 5] = ['*', '?', '[', '{', '\\'];

impl Pattern {
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
                root: true,
                glob: String::new(),
                base: Vec::new(),
                deep: true,
                depth: 0,
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
            .map(|s| s.to_string())
            .collect();
        let rest = &segments[base.len()..];
        Ok(Self {
            text: text.to_string(),
            negated,
            dir_only,
            root: false,
            glob: body.to_string(),
            deep: rest.iter().any(|s| s.contains("**") || s.contains('{')),
            depth: segments.len(),
            base,
        })
    }

    /// Whether `path` (relative, `/`-separated; empty for the source folder)
    /// is one this pattern names. The `!` is the caller's to apply.
    pub fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if self.root {
            return path.is_empty();
        }
        glob_match(&self.glob, path)
    }

    /// Whether something strictly inside the folder `dir` could match, so a
    /// walk knows whether to look inside.
    pub fn may_match_below(&self, dir: &str) -> bool {
        if self.root {
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
        self.deep || self.depth > parts.len()
    }

    /// The path this pattern names when it has no wildcards and no `!`.
    pub fn literal(&self) -> Option<String> {
        if self.negated {
            None
        } else if self.root {
            Some(String::new())
        } else if self.glob.contains(SPECIAL) {
            None
        } else {
            Some(self.glob.clone())
        }
    }

    /// `.`: the whole source.
    pub fn is_root(&self) -> bool {
        self.root
    }
}

/// A source's include and exclude patterns, applied together.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub include: Vec<Pattern>,
    pub exclude: Vec<Pattern>,
}

/// What the patterns say about one path, carried down into its folder:
/// a pattern that matches a folder covers everything inside it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verdict {
    included: bool,
    unincluded: bool,
    excluded: bool,
    kept: bool,
}

impl Verdict {
    pub fn ships(&self) -> bool {
        self.included && !self.unincluded && !(self.excluded && !self.kept)
    }
}

impl Rules {
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
                    verdict.unincluded = true;
                } else {
                    verdict.included = true;
                }
            }
        }
        for pattern in &self.exclude {
            if pattern.matches(path, is_dir) {
                if pattern.negated {
                    verdict.kept = true;
                } else {
                    verdict.excluded = true;
                }
            }
        }
        verdict
    }

    /// Whether a walk should look inside the folder `dir`.
    pub fn enter(&self, verdict: Verdict, dir: &str) -> bool {
        if verdict.unincluded {
            return false;
        }
        if verdict.excluded
            && !verdict.kept
            && !self
                .exclude
                .iter()
                .any(|p| p.negated && p.may_match_below(dir))
        {
            return false;
        }
        verdict.included
            || self
                .include
                .iter()
                .any(|p| !p.negated && p.may_match_below(dir))
    }
}

#[cfg(test)]
#[path = "glob_test.rs"]
mod tests;
