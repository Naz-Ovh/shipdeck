//! Colours on stderr, unless it is not a terminal or `NO_COLOR` is set.

use std::io::IsTerminal;

/// ANSI colors on stderr, unless it is not a terminal or `NO_COLOR` is set.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Paint {
    on: bool,
}

impl Paint {
    pub(crate) fn stderr() -> Self {
        Self {
            on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn plain() -> Self {
        Self { on: false }
    }

    fn wrap(self, code: &str, text: &str) -> String {
        if self.on && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    pub(crate) fn step(self, text: &str) -> String {
        self.wrap("1;36", text)
    }
    pub(crate) fn command(self, text: &str) -> String {
        self.wrap("35", text)
    }
    pub(crate) fn info(self, text: &str) -> String {
        self.wrap("34", text)
    }
    pub(crate) fn success(self, text: &str) -> String {
        self.wrap("1;32", text)
    }
    pub(crate) fn warn(self, text: &str) -> String {
        self.wrap("33", text)
    }
    pub(crate) fn error(self, text: &str) -> String {
        self.wrap("1;31", text)
    }
    pub(crate) fn muted(self, text: &str) -> String {
        self.wrap("2", text)
    }
}
