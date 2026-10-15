//! The one-time server setup. `server-setup.sh` lives at the root of the
//! shipdeck repository; whoever owns the server runs it once as root, fetched
//! at a release tag. It creates the deploy user, authorizes its key, creates
//! the app folder and installs the sudo rule shipdeck needs, or with PM2
//! starts PM2 at boot instead. shipdeck itself never runs it: the sudo rule
//! it installs is a contract with `remote::sudo_commands`, checked in the
//! tests.

pub const SCRIPT_NAME: &str = "server-setup.sh";

/// The script at the tag of this shipdeck version, so what runs as root only
/// changes with a new release.
pub fn script_url() -> String {
    raw_url(SCRIPT_NAME)
}

/// A file at the root of the shipdeck repository, at this version's tag.
pub fn raw_url(file: &str) -> String {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!(
        "https://raw.githubusercontent.com/{path}/v{}/{file}",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
#[path = "setup_test.rs"]
mod tests;
