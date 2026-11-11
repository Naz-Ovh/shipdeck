//! Value checks for one app: its paths, process, managers and nginx.

use super::{PM2_OWN, Rules, SYSTEMD_OWN, is_option_value};
use crate::config::names::{is_plain_name, is_program, is_unit_name, split_args};
use crate::config::paths;
use crate::config::types::{App, FileEntry, KeepUp, Process};

impl Rules<'_> {
    pub(super) fn app(&mut self, name: &str, app: &App) {
        let process = app.process.as_ref();
        let is_static = app.static_site == Some(true);
        match (process, app.static_site) {
            (Some(_), Some(_)) => self.fail(
                &["apps", name, "static"],
                "`process` and `static` cannot both be set",
                "pick one",
                "remove `static` for a process, or `process` for a static site",
            ),
            (None, None) => self.fail(
                &["apps", name],
                "one of `process` or `static: true` is needed",
                "nothing to run or serve",
                "add `process` with `start`, or `static: true`",
            ),
            (None, Some(false)) => self.fail(
                &["apps", name, "static"],
                "`static` must be `true`",
                "not true",
                "remove the line for a process app",
            ),
            _ => {}
        }
        self.app_paths(name, app);
        if let Some(process) = process {
            self.process(name, app, process);
        }
        if is_static {
            self.static_app(name, app);
        }
        self.nginx(name, app);
        self.commands(
            &["apps", name, "steps"],
            [
                (
                    "preflight",
                    &app.steps.preflight.local,
                    &app.steps.preflight.server,
                ),
                ("build", &app.steps.build.local, &app.steps.build.server),
                ("upload", &app.steps.upload.local, &app.steps.upload.server),
                ("setup", &app.steps.setup.local, &app.steps.setup.server),
                ("backup", &app.steps.backup.local, &app.steps.backup.server),
                ("switch", &app.steps.switch.local, &app.steps.switch.server),
                (
                    "restart",
                    &app.steps.restart.local,
                    &app.steps.restart.server,
                ),
                ("nginx", &app.steps.nginx.local, &app.steps.nginx.server),
                ("health", &app.steps.health.local, &app.steps.health.server),
                ("done", &app.steps.done.local, &app.steps.done.server),
                (
                    "rollback",
                    &app.steps.rollback.local,
                    &app.steps.rollback.server,
                ),
                ("failed", &app.steps.failed.local, &app.steps.failed.server),
            ],
        );
        if let Some(url) = &app.steps.health.url {
            self.url(&["apps", name, "steps", "health", "url"], url);
        }
        self.patterns(
            &["apps", name, "steps", "backup", "include"],
            &app.steps.backup.include,
        );
        self.patterns(
            &["apps", name, "steps", "backup", "exclude"],
            &app.steps.backup.exclude,
        );
    }

    /// The app's binary, file entries and shared folders.
    fn app_paths(&mut self, name: &str, app: &App) {
        let base = ["apps", name];
        if let Some(binary) = &app.binary {
            self.binary(name, app, binary);
        }
        self.files(
            &base,
            &app.files,
            &app.exclude,
            &app.shared,
            Some((name, app)),
        );
        self.shared(&base, &app.shared);
    }

    /// The manager, the start command, its environment and its own file.
    fn process(&mut self, name: &str, app: &App, process: &Process) {
        if process.manager.is_none() {
            self.fail(
                &["apps", name, "process"],
                "a process needs one manager: `systemd`, `pm2` or `oxmgr`",
                "no manager",
                "there is no default manager",
            );
        }
        let process_name = self.process_name(name);
        if !process_name.contains("{{")
            && (!is_unit_name(&process_name)
                || process_name.starts_with('.')
                || process_name.chars().all(|c| c.is_ascii_digit()))
        {
            self.fail(
                &["apps", name, "process", "name"],
                format!("{process_name:?} is not a usable process name"),
                "not allowed",
                "letters, digits, `.`, `_` and `-`, with at least one letter",
            );
        }
        self.start(name, process);
        if process.start.is_none() && app.binary.is_none() {
            self.fail(
                &["apps", name, "process"],
                "`process.start` is needed when there is no `binary`",
                "what does it run?",
                "such as `start: node dist/main.js`, or `binary: dist/app`",
            );
        }
        if let Some(file) = &process.env_file
            && let Err(why) = paths::release_path(file)
        {
            self.fail(
                &["apps", name, "process", "env_file"],
                format!("`env_file` {why}"),
                "not a path in the app folder",
                "such as `.env`",
            );
        }
        for (key, value) in &process.env {
            let valid = !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                self.fail(
                    &["apps", name, "process", "env", key],
                    format!("{key:?} is not a variable name"),
                    "not a variable name",
                    "letters, digits and `_`, not starting with a digit",
                );
            } else if value.contains(['"', '\\', '\n', '\r']) {
                self.fail(
                    &["apps", name, "process", "env", key],
                    format!("`env.{key}` must not contain quotes, backslashes or newlines"),
                    "not allowed",
                    "put values like this in `env_file`",
                );
            }
        }
        match &process.manager {
            Some(KeepUp::Systemd(systemd)) => self.systemd(name, systemd),
            Some(KeepUp::Pm2(pm2)) => self.pm2(name, app, process, pm2),
            Some(KeepUp::Oxmgr(_)) | None => {}
        }
    }

    /// `process.start`: one line, a closed quote, a program first.
    fn start(&mut self, name: &str, process: &Process) {
        let Some(start) = &process.start else {
            return;
        };
        let program = start.split_whitespace().next().unwrap_or_default();
        if start.trim().is_empty() {
            self.fail(
                &["apps", name, "process", "start"],
                "`start` is empty",
                "empty",
                "",
            );
        } else if start.contains(['\n', '\r', '\\']) {
            self.fail(
                &["apps", name, "process", "start"],
                "`start` must be one line without backslashes",
                "not allowed",
                "",
            );
        } else if split_args(start).is_none() {
            self.fail(
                &["apps", name, "process", "start"],
                "`start` has a quote that is never closed",
                "unclosed quote",
                "quote an argument with spaces like `--title \"my api\"`",
            );
        } else if !is_program(program) {
            self.fail(
                &["apps", name, "process", "start"],
                format!("`start` must begin with a program, not {program:?}"),
                "not a program",
                "a name such as `node`, or a path such as `./my-app`",
            );
        }
    }

    /// `binary`: a path, or a path and its place in the app folder.
    fn binary(&mut self, name: &str, app: &App, binary: &FileEntry) {
        let (path, to) = match binary {
            FileEntry::Path(path) => (path.as_str(), None),
            FileEntry::To(mapped) => (mapped.path.as_str(), mapped.to.as_deref()),
        };
        let at: &[&str] = if to.is_some() {
            &["apps", name, "binary", "to"]
        } else {
            &["apps", name, "binary"]
        };
        if path.trim().is_empty() {
            self.fail(at, "`binary` is empty", "empty", "");
            return;
        }
        if paths::has_wildcard(path) {
            self.fail(
                at,
                "`binary` must name one file, not a pattern",
                "not one file",
                "",
            );
            return;
        }
        let Some(to) = to else {
            return;
        };
        match paths::release_path(to) {
            Err(why) => self.fail(
                at,
                format!("`binary.to` {why}"),
                "not a path in the app folder",
                "such as `bin/api`",
            ),
            Ok(place) => {
                if place == ".shipdeck" || place.starts_with(".shipdeck/") {
                    self.fail(at, ".shipdeck belongs to shipdeck", "reserved", "");
                    return;
                }
                let shared: Vec<String> = app
                    .shared
                    .iter()
                    .filter_map(|entry| paths::release_path(entry).ok())
                    .collect();
                if let Some(folder) = shared.iter().find(|folder| paths::overlaps(folder, &place)) {
                    self.fail(
                        at,
                        format!("{place:?} is in the shared folder {folder:?}"),
                        "shared across releases",
                        "shared folders are never part of a release",
                    );
                }
            }
        }
    }

    fn systemd(&mut self, name: &str, systemd: &crate::config::types::Systemd) {
        if systemd.instances.is_some() {
            self.fail(
                &["apps", name, "process", "systemd", "instances"],
                "`systemd.instances` is planned, not supported yet",
                "reserved",
                "systemd instances are not built yet; remove it",
            );
        }
        if systemd.template.is_some() && systemd.options.is_some() {
            self.fail(
                &["apps", name, "process", "systemd", "options"],
                "`template` and `options` both write the unit",
                "pick one",
                "keep `template` (your whole file), or `options` (shipdeck's unit, changed)",
            );
        }
        if let Some(template) = &systemd.template
            && template.trim().is_empty()
        {
            self.fail(
                &["apps", name, "process", "systemd", "template"],
                "`template` is empty",
                "empty",
                "a path on your PC",
            );
        }
        if let Some(user) = &systemd.user
            && !is_plain_name(user)
        {
            self.fail(
                &["apps", name, "process", "systemd", "user"],
                "`user` is not a user name",
                "not allowed",
                "letters, digits, `.`, `_` and `-`",
            );
        }
        if let Some(options) = &systemd.options {
            for (section, entries) in [
                ("Unit", &options.unit),
                ("Service", &options.service),
                ("Install", &options.install),
            ] {
                for (key, value) in entries {
                    if let Some((own, help)) = SYSTEMD_OWN.iter().find(|(own, _)| *own == key) {
                        self.fail(
                            &["apps", name, "process", "systemd", "options", section, key],
                            format!("shipdeck writes `{own}` itself"),
                            "set by shipdeck",
                            help,
                        );
                    } else if !is_option_value(value) {
                        self.fail(
                            &["apps", name, "process", "systemd", "options", section, key],
                            format!("`{section}.{key}` must be text, a number or a list of them"),
                            "cannot go in a unit line",
                            "",
                        );
                    }
                }
            }
        }
    }

    fn pm2(&mut self, name: &str, _app: &App, process: &Process, pm2: &crate::config::types::Pm2) {
        if let Some(file) = &pm2.ecosystem {
            if process.start.is_some() {
                self.fail(
                    &["apps", name, "process", "start"],
                    "`start` and `pm2.ecosystem` both say what runs",
                    "pick one",
                    "keep `pm2.ecosystem` (your file starts the app), or `start`",
                );
            }
            if !pm2.options.is_empty() {
                self.fail(
                    &["apps", name, "process", "pm2", "options"],
                    "`pm2.options` and `pm2.ecosystem` are both set",
                    "pick one",
                    "the ecosystem file is a whole PM2 file",
                );
            }
            if let Err(why) = paths::release_path(file) {
                self.fail(
                    &["apps", name, "process", "pm2", "ecosystem"],
                    format!("`ecosystem` {why}"),
                    "not a path in the app folder",
                    "such as `ecosystem.config.js`",
                );
            }
        }
        for key in pm2.options.keys() {
            if let Some((own, help)) = PM2_OWN.iter().find(|(own, _)| own == key) {
                self.fail(
                    &["apps", name, "process", "pm2", "options", key],
                    format!("shipdeck sets the PM2 `{own}` itself"),
                    "set by shipdeck",
                    help,
                );
            }
        }
    }

    fn static_app(&mut self, name: &str, app: &App) {
        let base = ["apps", name];
        let refuse = |rules: &mut Self, key: &str, why: &str| {
            let mut path = base.to_vec();
            path.extend(key.split('.'));
            rules.fail(
                &path,
                format!("`{key}` does not apply to a static app"),
                "nothing runs on the server",
                why,
            );
        };
        if app.binary.is_some() {
            refuse(self, "binary", "remove it");
        }
        if !app.shared.is_empty() {
            refuse(
                self,
                "shared",
                "a static app has nothing to keep across releases",
            );
        }
        let setup_present = self
            .root
            .and_then(|root| root.span_of(&["apps", name, "steps", "setup"]))
            .is_some()
            || !app.steps.setup.local.is_empty()
            || !app.steps.setup.server.is_empty();
        if setup_present {
            refuse(self, "steps.setup", "nothing is set up for a static app");
        }
        let include_present = self
            .root
            .and_then(|root| root.span_of(&["apps", name, "steps", "backup", "include"]))
            .is_some()
            || !app.steps.backup.include.is_empty();
        if include_present {
            refuse(
                self,
                "steps.backup.include",
                "setup makes nothing, so nothing extra is backed up",
            );
        }
    }

    fn nginx(&mut self, name: &str, app: &App) {
        let Some(nginx) = &app.nginx else {
            return;
        };
        let static_app = app.static_site == Some(true);
        if nginx.domains.is_empty() {
            self.fail(
                &["apps", name, "nginx", "domains"],
                "`nginx.domains` is empty",
                "no domain",
                "such as `domains: [example.com]`",
            );
        }
        self.nginx_tls(name, nginx);
        self.nginx_kind(name, nginx, static_app);
        if let Some(url) = &nginx.proxy_pass {
            self.url(&["apps", name, "nginx", "proxy_pass"], url);
        }
        if let Some(root) = &nginx.root
            && let Err(why) = paths::release_path(root)
        {
            self.fail(
                &["apps", name, "nginx", "root"],
                format!("`root` {why}"),
                "not a folder in the app folder",
                "such as `dist`",
            );
        }
        if let Some(index) = &nginx.index
            && (index.trim().is_empty() || index.contains('/'))
        {
            self.fail(
                &["apps", name, "nginx", "index"],
                "`index` must be a file name",
                "not a file name",
                "such as `index.html`",
            );
        }
        for path in nginx.locations.keys() {
            if !path.starts_with('/') {
                self.fail(
                    &["apps", name, "nginx", "locations", path],
                    format!("location {path:?} must start with `/`"),
                    "not a location path",
                    "",
                );
            }
        }
    }

    /// nginx's certificate provider and its email.
    fn nginx_tls(&mut self, name: &str, nginx: &crate::config::types::Nginx) {
        let Some(tls) = &nginx.tls else {
            return;
        };
        let crate::config::types::Tls::LetsEncrypt(provider) = tls else {
            return;
        };
        if provider.provider != "letsencrypt" {
            self.fail(
                &["apps", name, "nginx", "tls", "provider"],
                format!(
                    "{:?} is not a certificate provider shipdeck knows",
                    provider.provider
                ),
                "not known",
                "use `letsencrypt`",
            );
        }
        if !provider.email.contains('@') {
            self.fail(
                &["apps", name, "nginx", "tls", "email"],
                "`tls.email` is not an email address",
                "not an address",
                "",
            );
        }
    }

    /// The fields a static app and a process app each need.
    fn nginx_kind(&mut self, name: &str, nginx: &crate::config::types::Nginx, static_app: bool) {
        if static_app {
            if nginx.proxy_pass.is_some() {
                self.fail(
                    &["apps", name, "nginx", "proxy_pass"],
                    "`proxy_pass` is for a process app, not a static one",
                    "nothing listens",
                    "remove it; a static app has `root` and `index`",
                );
            }
            if nginx.root.is_none() {
                self.fail(
                    &["apps", name, "nginx", "root"],
                    "a static app with `nginx` needs `root`",
                    "no folder to serve",
                    "the folder in the app folder to serve, such as `dist`",
                );
            }
            if nginx.index.is_none() {
                self.fail(
                    &["apps", name, "nginx", "index"],
                    "a static app with `nginx` needs `index`",
                    "no file to serve",
                    "such as `index.html`",
                );
            }
        } else {
            if nginx.proxy_pass.is_none() {
                self.fail(
                    &["apps", name, "nginx", "proxy_pass"],
                    "a process app with `nginx` needs `proxy_pass`",
                    "no address to pass to",
                    "where the app listens, such as http://127.0.0.1:3000",
                );
            }
            if nginx.root.is_some() || nginx.index.is_some() {
                self.fail(
                    &["apps", name, "nginx", "root"],
                    "`root` and `index` are for a static app",
                    "nothing is served from a folder",
                    "remove them; a process app has `proxy_pass`",
                );
            }
        }
    }
}
