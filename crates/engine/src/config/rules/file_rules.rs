//! Value checks for commands, health, patterns, files and placeholders.

use std::path::Path;

use super::{Rules, inside, wildcard_base};
use crate::config::glob::Pattern;
use crate::config::paths::{self, Form};
use crate::config::types::{App, Command, CommandList, FileEntry, HealthStep};

impl Rules<'_> {
    pub(super) fn commands<const N: usize>(
        &mut self,
        base: &[&str],
        steps: [(&str, &CommandList, &CommandList); N],
    ) {
        for (step, local, server) in steps {
            for (side, list) in [("local", local), ("server", server)] {
                for (index, command) in list.iter().enumerate() {
                    let index = index.to_string();
                    let mut path: Vec<&str> = base.to_vec();
                    path.push(step);
                    path.push(side);
                    path.push(&index);
                    let text = match command {
                        Command::Sh(text) => text,
                        Command::Runner { code, .. } => code,
                    };
                    if text.trim().is_empty() {
                        self.fail(&path, "this command is empty", "empty", "remove it");
                    }
                }
            }
        }
        for (step, local, server) in steps {
            for (side, list) in [("local", local), ("server", server)] {
                let _ = (step, side, list);
            }
        }
    }

    pub(super) fn health(&mut self, base: &[&str], health: Option<&HealthStep>, _has_url: bool) {
        if let Some(health) = health {
            if health.timeout_secs < 1 {
                let mut path = base.to_vec();
                path.push("timeout_secs");
                self.fail(&path, "`timeout_secs` must be at least 1", "too short", "");
            }
            if let Some(url) = &health.url {
                let mut path = base.to_vec();
                path.push("url");
                self.url(&path, url);
            }
        }
    }

    pub(super) fn url(&mut self, path: &[&str], url: &str) {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.fail(
                path,
                format!("{url:?} is not a URL"),
                "not a URL",
                "such as http://127.0.0.1:3000/health",
            );
        } else if url.contains(['\'', '"', '\n', '\r', ' ']) {
            self.fail(
                path,
                "a URL must not contain quotes, spaces or newlines",
                "not allowed",
                "",
            );
        }
    }

    pub(super) fn patterns(&mut self, base: &[&str], patterns: &[String], list: &str) {
        for (index, text) in patterns.iter().enumerate() {
            let index = index.to_string();
            let mut path = base.to_vec();
            path.push(&index);
            if let Err(why) = Pattern::parse(text) {
                self.fail(
                    &path,
                    format!("{text:?} {why}"),
                    "not a usable pattern",
                    if why.contains("..") {
                        "a pattern stays inside the project folder"
                    } else {
                        ""
                    },
                );
            }
        }
        let _ = list;
    }

    pub(super) fn shared(&mut self, base: &[&str], shared: &[String]) {
        let mut stored: Vec<String> = Vec::new();
        for (index, entry) in shared.iter().enumerate() {
            let index = index.to_string();
            let mut path = base.to_vec();
            path.push("shared");
            path.push(&index);
            match paths::release_path(entry) {
                Err(why) => self.fail(
                    &path,
                    format!("`shared` entry {entry:?} {why}"),
                    "not a folder in the release",
                    "such as `uploads`",
                ),
                Ok(folder) if folder == ".shipdeck" || folder.starts_with(".shipdeck/") => {
                    self.fail(&path, ".shipdeck belongs to shipdeck", "reserved", "");
                }
                Ok(folder) => {
                    if let Some(other) = stored.iter().find(|other| paths::overlaps(other, &folder))
                    {
                        self.fail(
                            &path,
                            format!("`shared` lists {folder:?} and {other:?}, which overlap"),
                            "overlaps",
                            "keep the outer folder",
                        );
                    } else {
                        stored.push(folder);
                    }
                }
            }
        }
    }

    /// Release and app file entries: a place, or the `to` they need.
    pub(super) fn files(
        &mut self,
        base: &[&str],
        files: &[FileEntry],
        exclude: &[String],
        shared: &[String],
        app: Option<(&str, &App)>,
    ) {
        let shared_paths: Vec<String> = shared
            .iter()
            .filter_map(|path| paths::release_path(path).ok())
            .collect();
        for (index, entry) in files.iter().enumerate() {
            self.file_entry(base, index, entry, &shared_paths, app);
        }
        for (index, text) in exclude.iter().enumerate() {
            let index = index.to_string();
            let mut path = base.to_vec();
            path.push("exclude");
            path.push(&index);
            if let Err(why) = Pattern::parse(text) {
                self.fail(
                    &path,
                    format!("{text:?} {why}"),
                    "not a usable pattern",
                    if why.contains("..") {
                        "a pattern stays inside the project folder"
                    } else {
                        ""
                    },
                );
            }
        }
    }

    pub(super) fn file_entry(
        &mut self,
        base: &[&str],
        index: usize,
        entry: &FileEntry,
        shared_paths: &[String],
        app: Option<(&str, &App)>,
    ) {
        let index = index.to_string();
        let mut path = base.to_vec();
        path.push("files");
        path.push(&index);
        let (text, to) = match entry {
            FileEntry::Path(text) => (text.as_str(), None),
            FileEntry::To(mapped) => (mapped.path.as_str(), mapped.to.as_deref()),
        };
        if text.trim().is_empty() {
            self.fail(&path, "this path is empty", "empty", "remove it");
            return;
        }
        let form = paths::form(text);
        if form != Form::Project && to.is_none() {
            self.fail(
                &path,
                format!("{text:?} is outside the project folder, so it needs `to`"),
                "no place of its own",
                "such as `- { path: ./prod/config.json, to: config/prod.json }`",
            );
            return;
        }
        if to.is_none() && paths::has_wildcard(text) && !self.pattern_ok(&path, text) {
            return;
        }
        if let Some(app) = app
            && form == Form::Project
            && !self.inside_source(app.0, app.1, text)
        {
            self.fail(
                &path,
                format!("{text:?} is outside the app's `source`, so it needs `to`"),
                "no place of its own",
                "such as `- { path: shared/logo.png, to: logo.png }`",
            );
            return;
        }
        if let Some(to) = to {
            let mut path = base.to_vec();
            path.push("files");
            path.push(&index);
            path.push("to");
            if paths::has_wildcard(text) {
                self.fail(
                    &path,
                    "`to` cannot place a pattern; only one file or folder",
                    "not with a glob",
                    "",
                );
            }
            match paths::release_path(to) {
                Err(why) => self.fail(
                    &path,
                    format!("`to` {why}"),
                    "not a path in the release",
                    "",
                ),
                Ok(place) => {
                    if place == ".shipdeck" || place.starts_with(".shipdeck/") {
                        self.fail(&path, ".shipdeck belongs to shipdeck", "reserved", "");
                    } else if let Some(folder) = shared_paths
                        .iter()
                        .find(|folder| paths::overlaps(folder, &place))
                    {
                        self.fail(
                            &path,
                            format!("{place:?} is in the shared folder {folder:?}"),
                            "shared across releases",
                            "shared folders are never part of a release",
                        );
                    }
                }
            }
        } else if let Ok(place) = paths::release_path(&paths::slash(Path::new(text)))
            && (place == ".shipdeck" || place.starts_with(".shipdeck/"))
        {
            self.fail(&path, ".shipdeck belongs to shipdeck", "reserved", "");
        }
    }

    pub(super) fn pattern_ok(&mut self, path: &[&str], text: &str) -> bool {
        match Pattern::parse(text) {
            Ok(_) => true,
            Err(why) => {
                self.fail(path, format!("{text:?} {why}"), "not a usable pattern", "");
                false
            }
        }
    }

    /// Whether a project-relative entry's base is inside the app's source.
    pub(super) fn inside_source(&self, name: &str, _app: &App, text: &str) -> bool {
        let written = self
            .deploy
            .apps
            .get(name)
            .and_then(|app| app.source.as_deref())
            .unwrap_or(".");
        // A source outside the project cannot be compared with a
        // project-relative entry here; the pack step reports that.
        if paths::form(written) != Form::Project {
            return true;
        }
        let source_rel = paths::slash(Path::new(written));
        let base_rel = paths::slash(Path::new(&wildcard_base(text)));
        inside(&source_rel, &base_rel)
    }
}
