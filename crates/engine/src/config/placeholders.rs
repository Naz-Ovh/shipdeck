//! The `{{NAME}}` placeholders, where each is valid, and its value.

use std::fmt;

/// One name a deploy file may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    /// `/opt/shop`, the server `dir`.
    Root,
    /// `/opt/shop/current`.
    Current,
    /// `/opt/shop/shared`.
    Shared,
    /// `/opt/shop/backups`.
    Backups,
    /// `/opt/shop/releases/<id>`.
    Release,
    /// The release id.
    ReleaseId,
    /// The release's version.
    Version,
    /// The deploy's name.
    Name,
    /// The server's host, as written.
    Host,
    /// The project folder on your PC.
    Project,
    /// The deploy file's folder on your PC.
    Config,
    /// The app's name.
    App,
    /// `/opt/shop/current/apps/<app>`.
    AppCurrent,
    /// `/opt/shop/releases/<id>/apps/<app>`.
    AppRelease,
    /// `/opt/shop/shared/apps/<app>`.
    AppShared,
    /// The app's source directory on your PC.
    Source,
    /// The process name.
    Process,
}

impl Placeholder {
    /// Every known name, in the design's table order.
    pub const ALL: [Placeholder; 17] = [
        Placeholder::Root,
        Placeholder::Current,
        Placeholder::Shared,
        Placeholder::Backups,
        Placeholder::Release,
        Placeholder::ReleaseId,
        Placeholder::Version,
        Placeholder::Name,
        Placeholder::Host,
        Placeholder::Project,
        Placeholder::Config,
        Placeholder::App,
        Placeholder::AppCurrent,
        Placeholder::AppRelease,
        Placeholder::AppShared,
        Placeholder::Source,
        Placeholder::Process,
    ];

    /// The name, as written between the braces.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "ROOT",
            Self::Current => "CURRENT",
            Self::Shared => "SHARED",
            Self::Backups => "BACKUPS",
            Self::Release => "RELEASE",
            Self::ReleaseId => "RELEASE_ID",
            Self::Version => "VERSION",
            Self::Name => "NAME",
            Self::Host => "HOST",
            Self::Project => "PROJECT",
            Self::Config => "CONFIG",
            Self::App => "APP",
            Self::AppCurrent => "APP_CURRENT",
            Self::AppRelease => "APP_RELEASE",
            Self::AppShared => "APP_SHARED",
            Self::Source => "SOURCE",
            Self::Process => "PROCESS",
        }
    }

    /// The name a written placeholder names, if it is known. `INSTANCE` is
    /// reserved, so it comes back here and is refused by [`Self::problem`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|placeholder| placeholder.name() == name)
    }

    /// It is only valid inside an app.
    #[must_use]
    pub fn app_only(self) -> bool {
        matches!(
            self,
            Self::App
                | Self::AppCurrent
                | Self::AppRelease
                | Self::AppShared
                | Self::Source
                | Self::Process
        )
    }

    /// It is only valid inside a process app.
    #[must_use]
    pub fn process_only(self) -> bool {
        self == Self::Process
    }

    /// It has no value until the release is named, at the start of `build`.
    #[must_use]
    pub fn needs_release(self) -> bool {
        matches!(
            self,
            Self::Release | Self::ReleaseId | Self::Version | Self::AppRelease
        )
    }

    /// Its value, when the deploy has one here.
    #[must_use]
    pub fn value(self, values: &Values<'_>) -> Option<String> {
        let root = values.root;
        match self {
            Self::Root => Some(root.to_string()),
            Self::Current => Some(format!("{root}/current")),
            Self::Shared => Some(format!("{root}/shared")),
            Self::Backups => Some(format!("{root}/backups")),
            Self::Release => Some(format!("{root}/releases/{}", values.release?)),
            Self::ReleaseId => Some(values.release?.to_string()),
            Self::Version => Some(values.version?.to_string()),
            Self::Name => Some(values.name.to_string()),
            Self::Host => Some(values.host.to_string()),
            Self::Project => Some(values.project.to_string()),
            Self::Config => Some(values.config.to_string()),
            Self::App => Some(values.app.as_ref()?.name.to_string()),
            Self::AppCurrent => Some(format!("{root}/current/apps/{}", values.app.as_ref()?.name)),
            Self::AppRelease => Some(format!(
                "{root}/releases/{}/apps/{}",
                values.release?,
                values.app.as_ref()?.name
            )),
            Self::AppShared => Some(format!("{root}/shared/apps/{}", values.app.as_ref()?.name)),
            Self::Source => Some(values.app.as_ref()?.source.clone()),
            Self::Process => values.app.as_ref()?.process.clone(),
        }
    }
}

impl fmt::Display for Placeholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{{{}}}}}", self.name())
    }
}

/// Where a string is written, and so which names have a value there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// The string is inside an app.
    pub app: bool,
    /// The app has a process, so `{{PROCESS}}` has a value.
    pub process: bool,
    /// The release is named by the time this string is used.
    pub release: bool,
}

impl Scope {
    /// The whole release, after the release is named.
    pub const RELEASE: Self = Self {
        app: false,
        process: false,
        release: true,
    };

    /// The whole release, before the release is named: `preflight`.
    pub const EARLY: Self = Self {
        app: false,
        process: false,
        release: false,
    };

    /// Inside an app.
    #[must_use]
    pub fn app(process: bool, release: bool) -> Self {
        Self {
            app: true,
            process,
            release,
        }
    }
}

/// The values a deploy has for the placeholders right now.
#[derive(Debug, Clone)]
pub struct Values<'a> {
    /// The server `dir`.
    pub root: &'a str,
    /// The deploy's name.
    pub name: &'a str,
    /// The server's host, as written.
    pub host: &'a str,
    /// The project folder on your PC.
    pub project: &'a str,
    /// The deploy file's folder on your PC.
    pub config: &'a str,
    /// The release id, once named.
    pub release: Option<&'a str>,
    /// The release's version, once named.
    pub version: Option<&'a str>,
    /// The app, inside an app.
    pub app: Option<AppValues<'a>>,
}

/// The values an app adds.
#[derive(Debug, Clone)]
pub struct AppValues<'a> {
    /// The app's name.
    pub name: &'a str,
    /// The app's source directory on your PC.
    pub source: String,
    /// The process name, for a process app.
    pub process: Option<String>,
}

/// What can be wrong with a placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadPlaceholder {
    /// What is wrong.
    pub message: String,
    /// How to put it right.
    pub help: Option<String>,
}

/// Checks every placeholder in `text` against `scope`.
///
/// # Errors
/// Returns the first name that is unknown, reserved, or has no value here.
pub fn check(text: &str, scope: Scope) -> Result<(), BadPlaceholder> {
    for found in scan(text) {
        let name = found.name;
        if name == "INSTANCE" {
            return Err(BadPlaceholder {
                message: "`{{INSTANCE}}` is planned, not supported yet".into(),
                help: Some("systemd instances are not built yet; remove it".into()),
            });
        }
        let Some(placeholder) = Placeholder::from_name(&name) else {
            return Err(BadPlaceholder {
                message: format!("`{{{{{name}}}}}` is not a placeholder shipdeck knows"),
                help: Some(format!("known names: {}", known_list())),
            });
        };
        if placeholder.app_only() && !scope.app {
            return Err(BadPlaceholder {
                message: format!("`{placeholder}` has no value here"),
                help: Some("it is only valid inside an app".into()),
            });
        }
        if placeholder.process_only() && !scope.process {
            return Err(BadPlaceholder {
                message: format!("`{placeholder}` has no value here"),
                help: Some("it is only valid inside a process app".into()),
            });
        }
        if placeholder.needs_release() && !scope.release {
            return Err(BadPlaceholder {
                message: format!("`{placeholder}` has no value here"),
                help: Some("the release is named at the start of `build`".into()),
            });
        }
    }
    Ok(())
}

/// Replaces every placeholder in `text` with its value.
///
/// # Errors
/// Returns a message when a placeholder is unknown or has no value; a
/// deploy file is checked when it is read, so this means a bug in shipdeck.
pub fn expand(text: &str, values: &Values<'_>) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let content = &after[..end];
        let name: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        if !is_name(&name) {
            out.push_str(&rest[start..start + 2 + end + 2]);
            rest = &after[end + 2..];
            continue;
        }
        let Some(placeholder) = Placeholder::from_name(&name) else {
            return Err(format!(
                "`{{{{{name}}}}}` is not a placeholder shipdeck knows"
            ));
        };
        let Some(value) = placeholder.value(values) else {
            return Err(format!("`{placeholder}` has no value here"));
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The placeholders that have a value here, as `SHIPDECK_<NAME>`.
#[must_use]
pub fn env(values: &Values<'_>) -> Vec<(String, String)> {
    Placeholder::ALL
        .iter()
        .filter_map(|placeholder| {
            let value = placeholder.value(values)?;
            Some((format!("SHIPDECK_{}", placeholder.name()), value))
        })
        .collect()
}

/// The environment variables every command gets, beyond the placeholders.
pub const EXTRA_ENV: [&str; 4] = [
    "SHIPDECK_ACTION",
    "SHIPDECK_STEP",
    "SHIPDECK_PREVIOUS",
    "SHIPDECK_ERROR",
];

/// One placeholder found in a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The name between the braces, without spaces.
    pub name: String,
}

/// Every placeholder in `text`, in order. Text between braces that is not a
/// name, such as JavaScript or Python code, is not a placeholder.
#[must_use]
pub fn scan(text: &str) -> Vec<Found> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let name: String = after[..end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if is_name(&name) {
            found.push(Found { name });
        }
        rest = &after[end + 2..];
    }
    found
}

/// A placeholder name: capital letters, digits and `_`.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// `{{ROOT}}, {{CURRENT}}, ...`
fn known_list() -> String {
    Placeholder::ALL
        .iter()
        .map(|placeholder| format!("{{{{{}}}}}", placeholder.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "placeholders_tests.rs"]
mod tests;
