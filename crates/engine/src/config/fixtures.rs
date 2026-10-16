//! Test fixtures for the config tests: sample files and builders.

use std::path::{Path, PathBuf};

use super::*;

pub(super) const SAMPLE: &str = "\
name: Demo
ssh: { host: example.com, user: deploy }
service: { name: demo, dir: /opt/demo }
pipeline:
  build:
    run:
      local: cargo build --release
  pack:
    binary: target/release/demo
";

/// The full example from the README.
pub(super) const NODE: &str = "\
name: Web (prod)
project: ../../../web
ssh:
  host: prod
  user: deploy
service:
  name: web
  dir: /opt/web
  start: node dist/main.js
  env_file: .env
  env: { NODE_ENV: production }
  shared: [uploads]
pipeline:
  preflight:
    requires: [node, npm]
    run:
      server: pg_isready -q
      local: npm test
  build:
    version_from: package.json
    run:
      local: [npm ci, npm run build]
  pack:
    files:
      include:
        project: [dist, package.json, package-lock.json]
        deploy: [.env]
      exclude:
        project: [\"**/*.map\"]
  setup:
    run:
      server:
        - npm ci --omit=dev
        - npm run migrate
    check:
      server: node --check dist/main.js
  backup:
    files:
      include: [data/keys]
      exclude: [\"**/*.map\"]
    keep: { open: 3, compressed: 7 }
  health:
    url: http://127.0.0.1:3000/health
    check:
      server: ./bin/smoke-test
      local: curl -fsS https://web.example.com/health
    timeout_secs: 30
  done:
    run:
      server: ./bin/warm-cache
      local: ./notify.sh \"web $SHIPDECK_VERSION is live\"
  failed:
    rollback: auto
    run:
      local: ./notify.sh \"failed at $SHIPDECK_STEP\"
";

pub(super) fn sample() -> DeployConfig {
    parse(SAMPLE).unwrap()
}

/// The problems in `yaml`, which must have some.
pub(super) fn problems(yaml: &str) -> Vec<Problem> {
    match parse(yaml) {
        Ok(_) => panic!("expected problems in:\n{yaml}"),
        Err(problems) => problems,
    }
}

pub(super) fn messages(yaml: &str) -> Vec<String> {
    problems(yaml).into_iter().map(|p| p.message).collect()
}

pub(super) fn deployable(folder: &Path, config: DeployConfig) -> Deployable {
    Deployable {
        folder: folder.to_path_buf(),
        slug: "demo".into(),
        endpoint: crate::ssh::Endpoint::from_config(
            &config.ssh,
            crate::ssh::ssh_config::HostConfig::default(),
        )
        .unwrap(),
        config,
        yaml: String::new(),
    }
}

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// SAMPLE with PM2 and `extra` in `service`.
pub(super) fn pm2(extra: &str) -> String {
    SAMPLE.replace(
        "dir: /opt/demo }",
        &format!("dir: /opt/demo, manager: pm2{extra} }}"),
    )
}

pub(super) fn with_files(include_project: &str) -> String {
    SAMPLE.replace(
        "    binary: target/release/demo\n",
        &format!(
            "    binary: target/release/demo\n    files:\n      include:\n        project: {include_project}\n"
        ),
    )
}
