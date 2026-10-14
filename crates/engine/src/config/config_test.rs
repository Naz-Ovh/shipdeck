use super::*;

const SAMPLE: &str = "
name: Demo
ssh: { host: example.com, user: deploy }
build:
  command: [cargo, build]
  binary: target/release/demo
remote: { dir: /opt/demo, service: demo }
";

fn sample() -> DeployConfig {
    parse(SAMPLE).unwrap()
}

fn endpoint(config: &DeployConfig) -> crate::ssh::Endpoint {
    crate::ssh::Endpoint::from_config(&config.ssh, crate::ssh::ssh_config::HostConfig::default())
        .unwrap()
}

fn deployable(folder: &Path, config: DeployConfig) -> Deployable {
    Deployable {
        folder: folder.to_path_buf(),
        slug: "demo".into(),
        endpoint: endpoint(&config),
        config,
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fills_in_defaults() {
    let config = sample();
    assert_eq!(config.ssh.port, None);
    assert_eq!(config.ssh.user.as_deref(), Some("deploy"));
    assert_eq!(config.build.cwd, ".");
    assert_eq!(config.hooks, Hooks::default());
    assert_eq!(
        config.releases,
        Retention {
            open: 5,
            compressed: 15
        }
    );
    assert_eq!(config.health.timeout_secs, 20);
}

#[test]
fn unknown_fields_are_errors_with_a_line_number() {
    let err = parse(&format!("{SAMPLE}colour: red\n"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("colour"), "{err}");
    assert!(err.contains("line 8"), "{err}");
}

#[test]
fn reads_a_json_config_too() {
    // JSON is YAML, so a renamed deploy.json keeps working.
    let config = parse(
        r#"{
            "name": "Demo",
            "ssh": { "host": "example.com", "user": "deploy" },
            "build": { "command": ["cargo", "build"], "binary": "target/release/demo" },
            "remote": { "dir": "/opt/demo", "service": "demo", "args": ["-E=.env"] },
            "bundle": [".env"]
        }"#,
    )
    .unwrap();
    assert_eq!(config.remote.args, ["-E=.env"]);
}

#[test]
fn a_command_is_a_shell_line_or_a_list() {
    let shell = parse(&SAMPLE.replace("[cargo, build]", "npm ci && npm run build")).unwrap();
    assert_eq!(
        shell.build.command,
        Some(CommandLine::Shell("npm ci && npm run build".into()))
    );
    assert_eq!(
        sample().build.command,
        Some(CommandLine::Args(vec!["cargo".into(), "build".into()]))
    );
    assert_eq!(sample().build.command.unwrap().display(), "cargo build");
}

const NODE: &str = "
name: Web (prod)
ssh: { host: prod, user: deploy }
hooks:
  before:
    - npm test
  server:
    - npm ci --omit=dev
  after: [echo done]
build:
  cwd: ../../web
  command: npm ci && npm run build
  files: [dist, package.json, package-lock.json]
remote:
  dir: /opt/web
  service: web
  start: node dist/main.js --port 8080
  requires: [npm]
";

#[test]
fn a_node_app_ships_files_and_starts_with_node() {
    let config = parse(NODE).unwrap();
    assert_eq!(config.hooks.before, ["npm test"]);
    assert_eq!(config.hooks.server, ["npm ci --omit=dev"]);
    assert_eq!(config.hooks.after, ["echo done"]);
    assert_eq!(config.build.binary, None);
    let deployable = deployable(Path::new("/tmp/deploy/web"), config);
    assert_eq!(
        deployable.start(),
        (
            Program::Tool("node".into()),
            "dist/main.js --port 8080".into()
        )
    );
    assert_eq!(deployable.required_tools(), ["node", "npm"]);
    assert_eq!(deployable.binary_name(), None);
}

#[test]
fn the_binary_is_the_default_start() {
    let deployable = deployable(Path::new("/tmp/deploy/demo"), sample());
    assert_eq!(
        deployable.start(),
        (Program::InRelease("demo".into()), String::new())
    );
    assert!(deployable.required_tools().is_empty());

    let mut config = sample();
    config.remote.start = Some("/usr/bin/env demo".into());
    config.remote.requires = vec!["env".into(), "/usr/bin/env".into()];
    let deployable = deployable_with(config);
    assert_eq!(
        deployable.start().0,
        Program::Absolute("/usr/bin/env".into())
    );
    assert_eq!(deployable.required_tools(), ["/usr/bin/env", "env"]);
}

fn deployable_with(config: DeployConfig) -> Deployable {
    deployable(Path::new("/tmp/deploy/demo"), config)
}

#[test]
fn something_must_ship_and_something_must_start() {
    let mut config = sample();
    config.build.binary = None;
    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("nothing to ship"), "{err}");

    config.build.files = vec!["dist".into()];
    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("`remote.start` is needed"), "{err}");

    config.remote.start = Some("node dist/main.js".into());
    assert!(config.validate().is_ok());
}

#[test]
fn rejects_unsafe_start_commands_and_tools() {
    for start in [
        "",
        "node a\nrm -rf /",
        "$(id) x",
        "'node' x",
        "../bin/x",
        "node a\\b",
    ] {
        let mut config = sample();
        config.remote.start = Some(start.into());
        assert!(config.validate().is_err(), "{start:?} should be rejected");
    }
    for file in ["/etc/passwd", "../.env", "a b", ""] {
        let mut config = sample();
        config.remote.env_file = Some(file.into());
        assert!(
            config.validate().is_err(),
            "env_file {file:?} should be rejected"
        );
    }
    for tool in ["", "no de", "a;b", "../x"] {
        let mut config = sample();
        config.remote.requires = vec![tool.into()];
        assert!(config.validate().is_err(), "{tool:?} should be rejected");
    }
}

#[test]
fn rejects_empty_hooks_and_commands() {
    let mut config = sample();
    config.hooks.server = vec!["  ".into()];
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("hooks.server")
    );

    for command in [CommandLine::Shell(" ".into()), CommandLine::Args(vec![])] {
        let mut config = sample();
        config.build.command = Some(command);
        assert!(config.validate().is_err());
    }
}

#[test]
fn rejects_unsafe_remote_dir() {
    for dir in ["opt/demo", "/", "/opt/../etc", "/opt/my app", "/opt/$(id)"] {
        let mut config = sample();
        config.remote.dir = dir.into();
        assert!(config.validate().is_err(), "{dir} should be rejected");
    }
}

#[test]
fn rejects_service_with_suffix_or_shell_characters() {
    for service in ["demo.service", "demo;rm", "", "a b"] {
        let mut config = sample();
        config.remote.service = service.into();
        assert!(config.validate().is_err(), "{service:?} should be rejected");
    }
}

#[test]
fn rejects_paths_outside_their_folder() {
    for entry in ["../secret", "/etc/passwd", "files/../../x"] {
        let mut config = sample();
        config.bundle = vec![entry.into()];
        assert!(
            config.validate().is_err(),
            "bundle {entry} should be rejected"
        );
        let mut config = sample();
        config.build.files = vec![entry.into()];
        assert!(
            config.validate().is_err(),
            "files {entry} should be rejected"
        );
    }
}

#[test]
fn rejects_zero_open_releases() {
    let mut config = sample();
    config.releases.open = 0;
    assert!(config.validate().is_err());
}

fn with_bundle(entries: &[&str]) -> DeployConfig {
    let mut config = sample();
    config.bundle = entries.iter().map(|e| e.to_string()).collect();
    config
}

#[test]
fn rejects_entries_naming_the_whole_folder() {
    for entry in ["", ".", "./", "./."] {
        let err = with_bundle(&[entry]).validate().unwrap_err();
        assert!(
            err.to_string().contains("whole deployable folder"),
            "{entry:?}"
        );
        let mut config = sample();
        config.build.files = vec![entry.into()];
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("whole build folder"), "{entry:?}");
    }
}

#[test]
fn rejects_duplicate_and_overlapping_entries() {
    let err = with_bundle(&[".env", "./.env"]).validate().unwrap_err();
    assert!(err.to_string().contains("twice"));
    let err = with_bundle(&["files/", "files/a.txt"])
        .validate()
        .unwrap_err();
    assert!(err.to_string().contains("overlap"));
    let err = with_bundle(&["files/a.txt", "files"])
        .validate()
        .unwrap_err();
    assert!(err.to_string().contains("overlap"));
    // A shared prefix is not an overlap.
    assert!(
        with_bundle(&["files", "files-extra", ".env"])
            .validate()
            .is_ok()
    );

    // build.files and bundle share the release root.
    let mut config = with_bundle(&["config"]);
    config.build.files = vec!["config".into()];
    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("shipped twice"), "{err}");
    config.build.files = vec!["config/base.yaml".into()];
    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("overlap"), "{err}");
}

#[test]
fn rejects_entries_that_replace_release_files() {
    for entry in ["release.json", "demo", "./demo.service"] {
        let err = with_bundle(&[entry]).validate().unwrap_err();
        assert!(err.to_string().contains("would replace"), "{entry}");
        let mut config = sample();
        config.build.files = vec![entry.into()];
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("would replace"), "{entry}");
    }
    // The same names deeper down are fine.
    assert!(with_bundle(&["files/release.json"]).validate().is_ok());
}

#[test]
fn bundle_paths_are_stored_without_dots_or_slashes() {
    assert_eq!(bundle_path("./files/"), "files");
    assert_eq!(bundle_path("files/./sub/a.txt"), "files/sub/a.txt");
    assert_eq!(bundle_path("."), "");
}

#[test]
fn check_bundle_lists_every_missing_entry() {
    let dir = scratch("check");
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let deployable = deployable(&dir, with_bundle(&[".env", "files/", "certs/key.pem"]));
    let err = deployable.check_bundle().unwrap_err().to_string();
    assert!(err.starts_with("2 of 3 bundle entries"), "{err}");
    assert!(
        err.contains(".env: missing") && err.contains("certs/key.pem: missing"),
        "{err}"
    );

    std::fs::write(dir.join(".env"), "A=1").unwrap();
    std::fs::create_dir_all(dir.join("certs")).unwrap();
    std::fs::write(dir.join("certs/key.pem"), "k").unwrap();
    assert!(deployable.check_bundle().is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_files_looks_in_the_build_folder() {
    let dir = scratch("files");
    std::fs::create_dir_all(dir.join("app/dist")).unwrap();
    let mut config = sample();
    config.build.cwd = "app".into();
    config.build.files = vec!["dist".into(), "package.json".into()];
    let deployable = deployable(&dir, config);
    let err = deployable.check_files().unwrap_err().to_string();
    assert!(err.starts_with("1 of 2 build.files entries"), "{err}");
    std::fs::write(dir.join("app/package.json"), "{}").unwrap();
    assert!(deployable.check_files().is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn version_file_is_found_in_the_build_folder() {
    let dir = scratch("version");
    let mut config = sample();
    config.build.cwd = "app".into();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    let found = deployable(&dir, config.clone());
    assert_eq!(found.version_file(), None);
    std::fs::write(dir.join("app/package.json"), "{}").unwrap();
    std::fs::write(dir.join("app/VERSION"), "1.0.0").unwrap();
    assert_eq!(found.version_file(), Some(dir.join("app/package.json")));

    config.build.version_from = Some("app/VERSION".into());
    let named = deployable(&dir, config);
    assert_eq!(named.version_file(), Some(dir.join("app/VERSION")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn user_may_come_from_ssh_config() {
    let config =
        parse(&SAMPLE.replace("{ host: example.com, user: deploy }", "{ host: prod }")).unwrap();
    assert_eq!(config.ssh.user, None);
}

#[test]
fn discovery_reports_old_and_doubled_configs() {
    let root = scratch("discover");
    for name in ["api", "old", "twice"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("api/deploy.yml"), SAMPLE).unwrap();
    std::fs::write(root.join("old/deploy.json"), "{}").unwrap();
    std::fs::write(root.join("twice/deploy.yaml"), SAMPLE).unwrap();
    std::fs::write(root.join("twice/deploy.yml"), SAMPLE).unwrap();

    let found = discover(&root).unwrap();
    let slugs: Vec<&str> = found.deployables.iter().map(|d| d.slug.as_str()).collect();
    assert_eq!(slugs, ["api"]);
    let broken: Vec<(String, &str)> = found
        .broken
        .iter()
        .map(|(path, why)| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                why.as_str(),
            )
        })
        .collect();
    assert_eq!(broken.len(), 2, "{broken:?}");
    assert_eq!(broken[0].0, "old");
    assert!(broken[0].1.contains("now reads deploy.yaml"), "{broken:?}");
    assert_eq!(broken[1].0, "twice");
    assert!(broken[1].1.contains("keep one"), "{broken:?}");
    std::fs::remove_dir_all(root).unwrap();
}

/// The examples in the repository must stay valid.
#[test]
fn examples_parse() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(&examples).unwrap() {
        let file = entry.unwrap().path().join(FILE_NAMES[0]);
        let text = std::fs::read_to_string(&file).unwrap();
        if let Err(err) = parse(&text) {
            panic!("{}: {err:#}", file.display());
        }
        seen += 1;
    }
    assert!(seen >= 4, "expected the rust, go, node and python examples");
}

#[test]
fn plain_yaml_scalars_read_as_text() {
    let config = parse(&format!("{SAMPLE}\nhealth: {{ timeout_secs: 30 }}\n")).unwrap();
    assert_eq!(config.health.timeout_secs, 30);
    let config = parse(&SAMPLE.replace(
        "remote: { dir: /opt/demo, service: demo }",
        "remote:\n  dir: /opt/demo\n  service: demo\n  env:\n    PORT: 3000\n    DEBUG: no\n    RATIO: 1.5",
    ))
    .unwrap();
    assert_eq!(config.remote.env["PORT"], "3000");
    assert_eq!(config.remote.env["DEBUG"], "no");
    assert_eq!(config.remote.env["RATIO"], "1.5");
}
