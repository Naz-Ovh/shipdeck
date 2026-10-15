//! What the server looks like before anything is touched.

use anyhow::Result;

use super::{Server, pairs};
use crate::build::{GlibcVersion, normalize_arch};
use crate::config::Manager;
use crate::ssh::sh_quote;

#[derive(Debug, Clone)]
pub struct Facts {
    /// In the spelling `build::inspect` uses.
    pub arch: String,
    /// `None` on a musl or otherwise glibc-free server.
    pub glibc: Option<GlibcVersion>,
    pub dir_exists: bool,
    pub dir_writable: bool,
    pub free_bytes: Option<u64>,
    /// The release `current` points to.
    pub current: Option<String>,
    /// The service's state: `systemctl is-active` output, or PM2's state
    /// in the same words.
    pub active: String,
    /// `systemctl is-active` for `service.name`, whichever the manager: a
    /// unit still running holds the app's port when moving to PM2.
    pub systemd: String,
    /// `pm2 -v` output, with PM2 as the manager.
    pub pm2_version: Option<String>,
    /// The commands the user may not run with sudo (never any for root).
    pub sudo_missing: Vec<String>,
    /// Each program asked for, with where `command -v` found it.
    pub tools: Vec<(String, Option<String>)>,
}

impl Facts {
    /// Where the server has `tool`, if it was asked for and found.
    pub fn tool_path(&self, tool: &str) -> Option<&str> {
        self.tools
            .iter()
            .find(|(name, _)| name == tool)
            .and_then(|(_, path)| path.as_deref())
    }
}

impl Server {
    /// Looks the server over, finding each of `tools` on its PATH.
    pub async fn facts(&self, tools: &[String]) -> Result<Facts> {
        // Root runs them without sudo.
        let commands = if self.is_root() {
            Vec::new()
        } else {
            self.sudo_commands()
        };
        let lookups: String = tools
            .iter()
            .map(|tool| {
                format!(
                    "printf 'tool=%s:%s\\n' {q} \"$(command -v {q} 2>/dev/null)\"\n",
                    q = sh_quote(tool)
                )
            })
            .collect();
        let checks: String = commands
            .iter()
            .map(|c| {
                format!(
                    "if sudo -n -l {c} >/dev/null 2>&1; then echo sudo_ok={q}; else echo sudo_missing={q}; fi\n",
                    q = sh_quote(c)
                )
            })
            .collect();
        let script = format!(
            "dir={dir}
echo \"arch=$(uname -m)\"
echo \"glibc=$(ldd --version 2>&1 | head -n1 | grep -o '[0-9][0-9.]*$')\"
if [ -d \"$dir\" ]; then echo dir=yes; else echo dir=no; fi
if [ -w \"$dir\" ]; then echo writable=yes; else echo writable=no; fi
p=\"$dir\"; while [ ! -d \"$p\" ]; do p=$(dirname \"$p\"); done
echo \"free_kb=$(df -Pk \"$p\" | awk 'NR==2 {{print $4}}')\"
echo \"current=$(readlink \"$dir/current\" 2>/dev/null)\"
echo \"active=$(systemctl is-active {svc} 2>&1 | head -n1)\"
{pm2}{lookups}{checks}",
            dir = self.q_dir(),
            svc = self.q_service(),
            pm2 = if self.manager == Manager::Pm2 {
                "echo \"pm2=$(pm2 -v 2>/dev/null | tail -n1)\"\n"
            } else {
                ""
            },
        );
        let out = self
            .session
            .exec(&script)
            .await?
            .check("checking the server")?;
        let values = pairs(&out.stdout);
        let get = |key: &str| {
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default()
        };

        let systemd = get("active");
        let active = match self.manager {
            Manager::Systemd => systemd.clone(),
            Manager::Pm2 => self.pm2_state().await?.0,
        };
        Ok(Facts {
            arch: normalize_arch(&get("arch")),
            glibc: GlibcVersion::parse(&get("glibc")),
            dir_exists: get("dir") == "yes",
            dir_writable: get("writable") == "yes",
            free_bytes: get("free_kb").parse::<u64>().ok().map(|kb| kb * 1024),
            current: Some(get("current"))
                .filter(|c| !c.is_empty())
                .map(|c| c.rsplit('/').next().unwrap_or(&c).to_string()),
            active,
            systemd,
            pm2_version: (self.manager == Manager::Pm2).then(|| get("pm2")),
            sudo_missing: values
                .iter()
                .filter(|(k, _)| *k == "sudo_missing")
                .map(|(_, v)| v.to_string())
                .collect(),
            tools: tools
                .iter()
                .map(|tool| {
                    let path = values
                        .iter()
                        .filter(|(k, _)| *k == "tool")
                        .filter_map(|(_, v)| v.split_once(':'))
                        .find(|(name, _)| name == tool)
                        .map(|(_, path)| path.trim().to_string())
                        .filter(|path| !path.is_empty());
                    (tool.clone(), path)
                })
                .collect(),
        })
    }
}
