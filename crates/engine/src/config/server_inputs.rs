//! Where a deploy's `host`, `port` and `user` come from: the file, `${NAME}`
//! in the file, `SHIPDECK_SSH_*` variables, or flags, in that order of
//! weakness. See `docs/feature/config-v2.md`, "Server details outside the
//! file".

use shipdeck_protocol::message::SshOverride;

use super::env::Env;
use super::types::{Port, SshTarget};

/// One of the three fields, with the flag and variable that can replace it.
#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    flag: &'static str,
    var: &'static str,
    /// The flag's value in `SshOverride`.
    of: fn(&SshOverride) -> Option<&str>,
}

const HOST: Field = Field {
    name: "host",
    flag: "--host",
    var: "SHIPDECK_SSH_HOST",
    of: |flags| flags.host.as_deref(),
};
const PORT: Field = Field {
    name: "port",
    flag: "--port",
    var: "SHIPDECK_SSH_PORT",
    of: |flags| flags.port.as_deref(),
};
const USER: Field = Field {
    name: "user",
    flag: "--user",
    var: "SHIPDECK_SSH_USER",
    of: |flags| flags.user.as_deref(),
};

/// What one run gives for the server details: the values it can read, and the
/// flags it was started with.
#[derive(Debug, Clone, Default)]
pub struct ServerInputs {
    env: Env,
    flags: SshOverride,
    run: Vec<String>,
}

/// One mistake in one of the three fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// `host`, `port` or `user`.
    pub field: &'static str,
    pub message: String,
    pub help: String,
    /// The mistake is a variable that is not set, or is empty.
    pub unset: bool,
}

/// A target with its details filled in, and where each one came from.
#[derive(Debug, Clone)]
pub struct Applied {
    pub ssh: SshTarget,
    /// For each detail that is not from the file: `host from --host`.
    pub from: Vec<String>,
}

impl ServerInputs {
    /// `run` names the root file's deploys this run uses; empty means all.
    #[must_use]
    pub fn new(env: Env, flags: SshOverride, run: Vec<String>) -> Self {
        Self { env, flags, run }
    }

    /// Only variables for `${NAME}`: no flags, and every deploy is in the run.
    #[must_use]
    pub fn from_env(env: Env) -> Self {
        Self::new(env, SshOverride::default(), Vec::new())
    }

    /// The values this run reads.
    #[must_use]
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// The deploys the root file lists that the run uses, without repeats.
    #[must_use]
    pub fn run<'a>(&'a self, listed: impl Iterator<Item = &'a String>) -> Vec<&'a str> {
        if self.run.is_empty() {
            return listed.map(String::as_str).collect();
        }
        let mut names: Vec<&str> = Vec::new();
        for name in &self.run {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
        names
    }

    /// The flags and variables that are set, as the output names them.
    fn set(&self) -> Vec<&'static str> {
        [HOST, PORT, USER]
            .into_iter()
            .filter_map(|field| {
                if (field.of)(&self.flags).is_some() {
                    Some(field.flag)
                } else {
                    self.inputs_var(field)
                }
            })
            .collect()
    }

    /// The variable's name, when it is set.
    fn inputs_var(&self, field: Field) -> Option<&'static str> {
        self.env.get(field.var).map(|_| field.var)
    }

    /// Whether the flags and variables apply to a run of `count` deploys:
    /// `Ok(true)` for one deploy that takes them, `Ok(false)` when none is set.
    ///
    /// # Errors
    /// Returns what to tell the owner when they are set and the run has
    /// more than one deploy.
    pub fn single(&self, count: usize) -> Result<bool, String> {
        let set = self.set();
        if set.is_empty() {
            return Ok(false);
        }
        if count > 1 {
            return Err(format!(
                "{} set the server of one deploy, and this run has {count} deploys; \
                 give each deploy its own server: an alias from ~/.ssh/config as `host`, \
                 or `${{NAME}}` variables in `server.ssh`",
                set.join(", ")
            ));
        }
        Ok(count == 1)
    }

    /// The details of one deploy: `overridden` when the flags and variables
    /// apply to it.
    #[must_use]
    pub fn details(&self, overridden: bool) -> Details<'_> {
        Details {
            inputs: self,
            overridden,
            lenient: false,
        }
    }

    /// Whether the run names its deploys. If not, it lists them all.
    #[must_use]
    pub fn names_run(&self) -> bool {
        !self.run.is_empty()
    }
}

/// How one deploy gets its server details.
#[derive(Clone, Copy)]
pub struct Details<'a> {
    inputs: &'a ServerInputs,
    overridden: bool,
    lenient: bool,
}

impl Details<'_> {
    /// A deploy whose variables are not set is unavailable, not an error.
    #[must_use]
    pub fn or_unavailable(self) -> Self {
        Self {
            lenient: true,
            ..self
        }
    }

    /// Whether a missing variable makes the deploy unavailable.
    #[must_use]
    pub fn is_lenient(&self) -> bool {
        self.lenient
    }

    /// `target` with every detail decided: a flag, then a `SHIPDECK_SSH_*`
    /// variable, then the file's value with its `${NAME}` replaced. A file
    /// value is read only when nothing above it is set.
    ///
    /// # Errors
    /// Returns every field that cannot be used, naming the deploy.
    pub fn apply(&self, target: &SshTarget, deploy: &str) -> Result<Applied, Vec<Failure>> {
        let mut from = Vec::new();
        let mut failures = Vec::new();
        let host = self.text(HOST, Some(&target.host), deploy);
        let user = self.text(USER, target.user.as_deref(), deploy);
        let port = self.port(target.port.as_ref(), deploy);
        let host = self.settle(HOST, host, &mut from, &mut failures);
        let user = self.settle(USER, user, &mut from, &mut failures);
        let port = self.settle(PORT, port, &mut from, &mut failures);
        if !failures.is_empty() {
            return Err(failures);
        }
        Ok(Applied {
            ssh: SshTarget {
                host: host.unwrap_or_default(),
                port: port.map(Port::Number),
                user,
                shell_env: target.shell_env,
            },
            from,
        })
    }

    /// The value of a field, noting where it came from when that is not the
    /// file, or the failure.
    fn settle<T>(
        &self,
        field: Field,
        result: Result<Option<(T, bool)>, Failure>,
        from: &mut Vec<String>,
        failures: &mut Vec<Failure>,
    ) -> Option<T> {
        match result {
            Ok(Some((value, outside))) => {
                if outside {
                    from.push(self.source(field));
                }
                Some(value)
            }
            Ok(None) => None,
            Err(failure) => {
                failures.push(failure);
                None
            }
        }
    }

    /// `host from --host`, or `host from SHIPDECK_SSH_HOST`, whichever set it.
    fn source(&self, field: Field) -> String {
        let from = if (field.of)(&self.inputs.flags).is_some() {
            field.flag
        } else {
            field.var
        };
        format!("{} from {from}", field.name)
    }

    /// The flag's value, else the variable's, when this deploy takes them.
    fn given(&self, field: Field) -> Option<String> {
        if !self.overridden {
            return None;
        }
        (field.of)(&self.inputs.flags)
            .or_else(|| self.inputs.env.get(field.var))
            .map(str::to_string)
    }

    /// A text field: `(value, whether it came from outside the file)`.
    fn text(
        &self,
        field: Field,
        file: Option<&str>,
        deploy: &str,
    ) -> Result<Option<(String, bool)>, Failure> {
        if let Some(value) = self.given(field) {
            return Ok(Some((value, true)));
        }
        file.map(|written| {
            self.replace(field, written, deploy)
                .map(|text| (text, false))
        })
        .transpose()
    }

    /// The port: `(number, whether it came from outside the file)`.
    fn port(&self, file: Option<&Port>, deploy: &str) -> Result<Option<(u16, bool)>, Failure> {
        let (text, outside) = if let Some(value) = self.given(PORT) {
            (value, true)
        } else {
            match file {
                None => return Ok(None),
                Some(Port::Number(number)) => (number.to_string(), false),
                Some(Port::Text(written)) => (self.replace(PORT, written, deploy)?, false),
            }
        };
        match text.trim().parse::<u16>() {
            Ok(number) if number >= 1 => Ok(Some((number, outside))),
            _ => Err(Failure {
                field: PORT.name,
                message: format!(
                    "`server.ssh.port` of the deploy {deploy} is {text:?}; \
                     it must be a number from 1 to 65535"
                ),
                help: "a port is a whole number, such as 22 or 2222".into(),
                unset: false,
            }),
        }
    }

    /// Replaces every `${NAME}` in `written` with the variable's value.
    fn replace(&self, field: Field, written: &str, deploy: &str) -> Result<String, Failure> {
        let mut out = String::with_capacity(written.len());
        let mut rest = written;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                // No closing brace: it is plain text, as it is anywhere else.
                rest = &rest[start..];
                break;
            };
            let name = &after[..end];
            if !is_variable_name(name) {
                return Err(Failure {
                    field: field.name,
                    message: format!(
                        "`server.ssh.{}` of the deploy {deploy} holds `${{{name}}}`, \
                         and {name:?} is not a variable name",
                        field.name
                    ),
                    help: "a name is letters, digits and `_`, and does not start with a digit"
                        .into(),
                    unset: false,
                });
            }
            let Some(value) = self.inputs.env.get(name) else {
                return Err(Failure {
                    field: field.name,
                    message: format!(
                        "the variable {name} is not set, or is empty; \
                         `server.ssh.{}` of the deploy {deploy} needs it",
                        field.name
                    ),
                    help: format!(
                        "set {name} in your environment or in .env, or replace this detail \
                         for one deploy with `{}`",
                        field.flag
                    ),
                    unset: true,
                });
            };
            out.push_str(value);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// Letters, digits and `_`, not starting with a digit.
fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "server_inputs_tests.rs"]
mod tests;
