use super::*;

const SAMPLE: &str = "\
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
const NODE: &str = "\
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

fn sample() -> DeployConfig {
    parse(SAMPLE).unwrap()
}

/// The problems in `yaml`, which must have some.
fn problems(yaml: &str) -> Vec<Problem> {
    match parse(yaml) {
        Ok(_) => panic!("expected problems in:\n{yaml}"),
        Err(problems) => problems,
    }
}

fn messages(yaml: &str) -> Vec<String> {
    problems(yaml).into_iter().map(|p| p.message).collect()
}

fn deployable(folder: &Path, config: DeployConfig) -> Deployable {
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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fills_in_defaults() {
    let config = sample();
    assert_eq!(config.project, ".");
    assert_eq!(config.ssh.port, None);
    let pipeline = &config.pipeline;
    assert_eq!(
        pipeline.backup.keep,
        Keep {
            open: 5,
            compressed: 15
        }
    );
    assert!(pipeline.backup.files.deployed);
    assert!(pipeline.backup.files.include.is_empty());
    assert_eq!(pipeline.health.timeout_secs, 20);
    assert_eq!(pipeline.failed.rollback, Rollback::Ask);
    assert!(pipeline.preflight.run.local.is_empty() && pipeline.done.run.server.is_empty());
    assert_eq!(pipeline.build.run.local.0, ["cargo build --release"]);
}

#[test]
fn reads_every_step_of_the_full_example() {
    let config = parse(NODE).unwrap();
    assert_eq!(config.project, "../../../web");
    assert_eq!(config.service.shared, ["uploads"]);
    let p = &config.pipeline;
    assert_eq!(
        p.backup.keep,
        Keep {
            open: 3,
            compressed: 7
        }
    );
    assert_eq!(p.backup.files.include, ["data/keys"]);
    assert_eq!(p.backup.files.exclude, ["**/*.map"]);
    assert_eq!(p.preflight.requires, ["node", "npm"]);
    assert_eq!(p.preflight.run.server.0, ["pg_isready -q"]);
    assert_eq!(p.build.run.local.0, ["npm ci", "npm run build"]);
    assert_eq!(p.pack.files.include.deploy, [".env"]);
    assert_eq!(p.pack.files.exclude.project, ["**/*.map"]);
    assert_eq!(p.setup.run.server.len(), 2);
    assert_eq!(p.setup.check.server.0, ["node --check dist/main.js"]);
    assert_eq!(p.health.timeout_secs, 30);
    assert_eq!(p.failed.rollback, Rollback::Auto);
    assert_eq!(p.health.check.local.len(), 1);
    assert_eq!(
        p.done.run.local.0,
        ["./notify.sh \"web $SHIPDECK_VERSION is live\""]
    );
    assert_eq!(p.failed.run.local.len(), 1);
}

#[test]
fn json_is_yaml_too() {
    let config = parse(
        r#"{
            "name": "Demo",
            "ssh": { "host": "example.com", "user": "deploy" },
            "service": { "name": "demo", "dir": "/opt/demo", "start": "./demo -E=.env" },
            "pipeline": { "pack": { "binary": "target/release/demo" } }
        }"#,
    )
    .unwrap();
    assert_eq!(config.service.start.as_deref(), Some("./demo -E=.env"));
}

#[test]
fn plain_yaml_scalars_read_as_text() {
    let config = parse(&SAMPLE.replace(
        "service: { name: demo, dir: /opt/demo }",
        "service:\n  name: demo\n  dir: /opt/demo\n  env:\n    PORT: 3000\n    DEBUG: no\n    RATIO: 1.5",
    ))
    .unwrap();
    assert_eq!(config.service.env["PORT"], "3000");
    assert_eq!(config.service.env["DEBUG"], "no");
    assert_eq!(config.service.env["RATIO"], "1.5");
}

#[test]
fn a_key_with_nothing_after_it_means_the_default() {
    let config = parse(&format!("{SAMPLE}  setup:\n  health:\n    url:\n")).unwrap();
    assert!(config.pipeline.setup.run.server.is_empty());
    assert_eq!(config.pipeline.health.url, None);
}

#[test]
fn every_problem_in_a_file_is_reported_at_once_in_line_order() {
    let found = problems(
        "\
name: Demo
ssh: { host: example.com, user: deploy }
service: { name: demo.service, dir: opt/demo }
pipeline:
  unpack:
    run: { server: tar x }
  pack:
    binary: target/release/demo
  health:
    timeout_secs: 1
  failed:
    rollback: later
",
    );
    let lines: Vec<(usize, &str)> = found
        .iter()
        .map(|p| (p.at.unwrap().start.0, p.message.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            (3, "`service.name` is the name without .service"),
            (3, "`service.dir` must be an absolute path"),
            (5, "unpack does not take commands"),
            (10, "`pipeline.health.timeout_secs` must be at least 3"),
            (12, "`pipeline.failed.rollback` must be ask or auto"),
        ]
    );
}

#[test]
fn a_step_takes_commands_only_where_it_can_run_them() {
    let cases = [
        (
            "  build:\n    run:\n      server: npm run build\n",
            "build cannot run commands on the server",
            "build runs on your PC only",
        ),
        (
            "  setup:\n    run:\n      local: npm ci\n",
            "setup cannot run commands on your PC",
            "setup runs on the server only",
        ),
        (
            "  failed:\n    run:\n      server: ./alert\n",
            "failed cannot run commands on the server",
            "failed runs on your PC only",
        ),
        (
            "  health:\n    run:\n      server: ./check\n",
            "health has no `run`",
            "not a field of health",
        ),
        (
            "  restart:\n    run:\n      server: ./x\n",
            "restart does not take commands",
            "restart is done by shipdeck",
        ),
    ];
    let bare = SAMPLE.replace(
        "  build:\n    run:\n      local: cargo build --release\n",
        "",
    );
    for (step, message, label) in cases {
        let yaml = bare.replace("  pack:\n", &format!("{step}  pack:\n"));
        let found = problems(&yaml);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].message, message);
        assert_eq!(found[0].label, label);
        assert!(found[0].at.is_some());
    }
}

#[test]
fn a_bare_command_is_told_where_to_run() {
    let found = problems(&SAMPLE.replace("    run:\n      local: cargo", "    run: cargo"));
    assert_eq!(found[0].message, "`pipeline.build.run` must be a map");
    assert_eq!(
        found[0].help.as_deref(),
        Some("`run: { local: cargo build --release }`")
    );
}

#[test]
fn unknown_keys_get_a_suggestion_or_the_list() {
    let found = problems(&format!("{SAMPLE}  helth:\n    url: http://x\n"));
    assert_eq!(found[0].message, "unknown key `pipeline.helth`");
    assert_eq!(found[0].help.as_deref(), Some("did you mean `health`?"));

    let found = problems(&format!("{SAMPLE}colour: red\n"));
    assert_eq!(found[0].at.unwrap().start, (10, 0));
    assert!(
        found[0]
            .help
            .as_deref()
            .unwrap()
            .contains("the top level takes")
    );

    let found = problems(&SAMPLE.replace(
        "binary:",
        "files:\n      include:\n        src: [dist]\n    binary:",
    ));
    assert_eq!(
        found[0].message,
        "unknown key `pipeline.pack.files.include.src`"
    );
    assert_eq!(found[0].label, "not a source");
    assert_eq!(
        found[0].help.as_deref(),
        Some("sources are project and deploy")
    );
}

#[test]
fn keys_from_0_2_say_where_they_went_and_hold_back_other_checks() {
    let found = messages(
        "\
name: Old
ssh: { host: prod, user: deploy }
remote: { dir: /opt/old, service: old }
build: { command: cargo build, binary: target/release/old }
hooks: { after: [echo hi] }
",
    );
    // No "nothing to ship" or "`service` is missing": those follow from the move.
    assert_eq!(
        found,
        [
            "`remote` moved in shipdeck 0.3",
            "`build` moved in shipdeck 0.3",
            "`hooks` moved in shipdeck 0.3",
        ]
    );
}

#[test]
fn an_unquoted_negated_pattern_is_caught() {
    let found = problems(&SAMPLE.replace(
        "binary:",
        "files:\n      include:\n        project: [dist, !dist/maps]\n    binary:",
    ));
    assert_eq!(
        found[0].message,
        "`pipeline.pack.files.include.project[1]` reads as a YAML tag"
    );
    assert_eq!(
        found[0].help.as_deref(),
        Some("put it in quotes: \"!dist/maps\"")
    );
    assert_eq!(found[0].at.unwrap().start, (11, 24));
}

#[test]
fn missing_and_empty_values_are_named() {
    assert_eq!(
        messages(""),
        [
            "`name` is missing",
            "`ssh` is missing",
            "`service` is missing"
        ]
    );
    let found = messages(&SAMPLE.replace("dir: /opt/demo", "dir:"));
    assert_eq!(found, ["`service.dir` is empty"]);
    let found = messages(&SAMPLE.replace(", dir: /opt/demo", ""));
    assert_eq!(found, ["`service.dir` is missing"]);
}

#[test]
fn yaml_mistakes_point_at_their_line() {
    let found = problems(&format!("{SAMPLE}  setup: {{\n"));
    assert!(
        found[0].message.starts_with("this is not valid YAML"),
        "{found:?}"
    );
    assert!(found[0].at.is_some());
    let found = problems(&format!("{SAMPLE}name: Again\n"));
    assert_eq!(found[0].message, "`name` is written twice");
    assert_eq!(found[0].label, "first written on line 1");
}

#[test]
fn rejects_unsafe_service_values() {
    let bad = [
        ("dir: /opt/demo", "dir: /"),
        ("dir: /opt/demo", "dir: /opt/../etc"),
        ("dir: /opt/demo", "dir: \"/opt/my app\""),
        ("name: demo", "name: \"demo;rm\""),
        ("name: demo", "name: \"a b\""),
    ];
    for (from, to) in bad {
        let yaml = SAMPLE.replace(from, to);
        assert_eq!(problems(&yaml).len(), 1, "{to} should be rejected");
    }
    for start in ["", "node a\\b", "$(id) x", "'node' x", "../bin/x"] {
        let yaml = SAMPLE.replace(
            "dir: /opt/demo }",
            &format!("dir: /opt/demo, start: {start:?} }}"),
        );
        assert!(parse(&yaml).is_err(), "start {start:?} should be rejected");
    }
    for file in ["/etc/passwd", "../.env", "a b"] {
        let yaml = SAMPLE.replace(
            "dir: /opt/demo }",
            &format!("dir: /opt/demo, env_file: {file:?} }}"),
        );
        assert!(
            parse(&yaml).is_err(),
            "env_file {file:?} should be rejected"
        );
    }
    for (key, value) in [("1X", "a"), ("A-B", "a"), ("OK", "say \\\"hi\\\"")] {
        let yaml = SAMPLE.replace(
            "dir: /opt/demo }",
            &format!("dir: /opt/demo, env: {{ {key}: \"{value}\" }} }}"),
        );
        assert!(
            parse(&yaml).is_err(),
            "env {key}={value} should be rejected"
        );
    }
    for shared in ["/abs", "../up", "release.json", "a b"] {
        let yaml = SAMPLE.replace(
            "dir: /opt/demo }",
            &format!("dir: /opt/demo, shared: [{shared:?}] }}"),
        );
        assert!(
            parse(&yaml).is_err(),
            "shared {shared:?} should be rejected"
        );
    }
    let yaml = SAMPLE.replace(
        "dir: /opt/demo }",
        "dir: /opt/demo, shared: [data, data/cache] }",
    );
    assert!(messages(&yaml)[0].contains("overlap"));
}

#[test]
fn a_backup_keeps_at_least_the_live_release_and_holds_something() {
    let backup = |yaml: &str| messages(&format!("{SAMPLE}  backup:\n{yaml}"));
    assert_eq!(
        backup("    keep: { open: 0 }\n"),
        ["`pipeline.backup.keep.open` must be at least 1"]
    );
    assert_eq!(
        backup("    files: { deployed: false }\n"),
        ["the backup would hold nothing but release.json and the service file"]
    );
    assert!(
        parse(&format!(
            "{SAMPLE}  backup:\n    files: {{ deployed: false }}\n    keep: {{ compressed: 0 }}\n"
        ))
        .is_ok()
    );
    assert_eq!(
        backup("    files: { include: [\"../x\", data, data] }\n"),
        ["\"../x\" climbs out with `..`", "\"data\" is listed twice"]
    );
    let shared = SAMPLE.replace("dir: /opt/demo }", "dir: /opt/demo, shared: [uploads] }");
    assert_eq!(
        messages(&format!(
            "{shared}  backup:\n    files: {{ include: [uploads/a] }}\n"
        )),
        ["\"uploads/a\" is in `service.shared`, so no backup holds it"]
    );
}

#[test]
fn keys_moved_since_point_to_their_new_place() {
    let keep = SAMPLE.replace("dir: /opt/demo }", "dir: /opt/demo, keep: { open: 3 } }");
    let found = problems(&keep);
    assert_eq!(found[0].message, "`service.keep` moved");
    assert!(
        found[0]
            .help
            .as_deref()
            .unwrap()
            .contains("pipeline.backup.keep")
    );
    let found = problems(&format!("{SAMPLE}  health:\n    rollback: auto\n"));
    assert_eq!(found[0].message, "`pipeline.health.rollback` moved");
    assert!(
        found[0]
            .help
            .as_deref()
            .unwrap()
            .contains("pipeline.failed.rollback")
    );
}

/// SAMPLE with PM2 and `extra` in `service`.
fn pm2(extra: &str) -> String {
    SAMPLE.replace(
        "dir: /opt/demo }",
        &format!("dir: /opt/demo, manager: pm2{extra} }}"),
    )
}

#[test]
fn pm2_reads_its_settings_with_their_types() {
    let config = parse(&pm2(
        ", pm2: { app: demo-api, options: { instances: 2, exec_mode: cluster, autorestart: false, max_memory_restart: 300M, quoted: \"4\" } }",
    ))
    .unwrap();
    let service = &config.service;
    assert_eq!(service.manager, Manager::Pm2);
    assert_eq!(service.pm2_app(), "demo-api");
    assert_eq!(service.own_file().as_deref(), Some("demo.pm2.json"));
    let options = &service.pm2.options;
    assert_eq!(options["instances"], serde_json::json!(2));
    assert_eq!(options["exec_mode"], serde_json::json!("cluster"));
    assert_eq!(options["autorestart"], serde_json::json!(false));
    assert_eq!(options["max_memory_restart"], serde_json::json!("300M"));
    assert_eq!(options["quoted"], serde_json::json!("4"));

    let config = parse(&pm2(
        ", pm2: { options: { a: 0x1F, b: 0o17, c: -3, d: 1.5e3, e: .5, f: TRUE, g: 1e3, h: .inf, i: 0x1G, j: 1.2.3 } }",
    ))
    .unwrap();
    let options = &config.service.pm2.options;
    let expected = serde_json::json!({
        "a": 31, "b": 15, "c": -3, "d": 1500.0, "e": 0.5, "f": true, "g": 1000.0,
        "h": ".inf", "i": "0x1G", "j": "1.2.3",
    });
    assert_eq!(serde_json::to_value(options).unwrap(), expected);

    let config = parse(&pm2(", pm2: { ecosystem: ecosystem.config.js }")).unwrap();
    assert_eq!(config.service.pm2_app(), "demo");
    assert_eq!(config.service.own_file(), None);
    assert_eq!(config.service.pm2_file(), "ecosystem.config.js");
    assert_eq!(sample().service.manager, Manager::Systemd);
    assert_eq!(sample().service.own_file().as_deref(), Some("demo.service"));
}

#[test]
fn pm2_settings_that_cannot_work_are_named() {
    for (extra, why) in [
        (", pm2: { app: \"a b\" }", "not a PM2 app name"),
        (", pm2: { app: \"42\" }", "not a PM2 app name"),
        (", pm2: { ecosystem: /etc/app.json }", "inside the release"),
        (", pm2: { ecosystem: ../app.json }", "inside the release"),
        (", pm2: { ecosystem: app.js }", "does not read"),
        (", pm2: { options: { name: x } }", "sets the PM2 `name`"),
        (", pm2: { options: { env: {} } }", "sets the PM2 `env`"),
        (", run_as: www", "runs the app as the SSH user"),
        (
            ", start: node --enable-source-maps dist/main.js",
            "node's first argument is the script",
        ),
        (
            ", start: node a.js, pm2: { ecosystem: app.config.js }",
            "both say what runs",
        ),
        (
            ", pm2: { ecosystem: app.config.js, options: { instances: 2 } }",
            "are both set",
        ),
    ] {
        let found = messages(&pm2(extra));
        assert!(found.iter().any(|m| m.contains(why)), "{extra}: {found:?}");
    }
    let yaml = SAMPLE.replace("dir: /opt/demo }", "dir: /opt/demo, pm2: { app: x } }");
    assert!(messages(&yaml)[0].contains("only read with `manager: pm2`"));
    // node's flags have their own PM2 setting.
    parse(&pm2(
        ", start: node dist/main.js, pm2: { options: { node_args: [--enable-source-maps] } }",
    ))
    .unwrap();
    let yaml = pm2(", shared: [demo.pm2.json]");
    assert!(messages(&yaml)[0].contains("belongs to shipdeck"));
}

#[test]
fn the_login_shell_environment_is_on_unless_turned_off() {
    assert!(sample().ssh.shell_env);
    let off = parse(&SAMPLE.replace(
        "ssh: { host: example.com, user: deploy }",
        "ssh: { host: example.com, user: deploy, shell_env: false }",
    ))
    .unwrap();
    assert!(!off.ssh.shell_env);
}

#[test]
fn start_arguments_split_like_systemd() {
    assert_eq!(
        split_args("dist/main.js --port 3000 --title \"my api\" -m 'a b'"),
        Some(
            [
                "dist/main.js",
                "--port",
                "3000",
                "--title",
                "my api",
                "-m",
                "a b"
            ]
            .map(String::from)
            .to_vec()
        )
    );
    assert_eq!(split_args("  "), Some(Vec::new()));
    assert_eq!(
        split_args("--empty \"\""),
        Some(vec!["--empty".into(), String::new()])
    );
    assert_eq!(split_args("--title \"my api"), None);
    let yaml = SAMPLE.replace(
        "dir: /opt/demo }",
        "dir: /opt/demo, start: './demo --title \"oops' }",
    );
    assert!(messages(&yaml)[0].contains("never closed"));
}

#[test]
fn pm2_needs_pm2_on_the_server_and_no_start_with_an_ecosystem() {
    let dir = scratch("pm2-tools");
    let yaml = pm2(", start: node dist/main.js").replace(
        "binary: target/release/demo",
        "files: { include: { project: [dist] } }",
    );
    let found = deployable(&dir, parse(&yaml).unwrap());
    assert_eq!(found.required_tools(), ["node", "pm2"]);
    let yaml = pm2(", pm2: { ecosystem: ecosystem.config.js }").replace(
        "binary: target/release/demo",
        "files: { include: { project: [dist, ecosystem.config.js] } }",
    );
    let found = deployable(&dir, parse(&yaml).unwrap());
    assert_eq!(found.required_tools(), ["pm2"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_unsafe_pipeline_values() {
    let found = messages(&format!(
        "{SAMPLE}  preflight:\n    requires: [\"no de\"]\n"
    ));
    assert_eq!(found, ["\"no de\" is not a program name"]);
    let found = messages(&format!(
        "{SAMPLE}  done:\n    run:\n      local: [ok, \"  \"]\n"
    ));
    assert_eq!(found, ["this command is empty"]);
    let found = messages(&format!("{SAMPLE}  health:\n    url: localhost:3000\n"));
    assert_eq!(
        found,
        ["`pipeline.health.url` must start with http:// or https://"]
    );
    let found = messages(&format!("{SAMPLE}  health:\n    url: \"http://x/'a'\"\n"));
    assert_eq!(
        found,
        ["`pipeline.health.url` must not contain quotes, spaces or newlines"]
    );
    let found = messages(&format!("{SAMPLE}  health:\n    timeout_secs: -1\n"));
    assert_eq!(
        found,
        ["`pipeline.health.timeout_secs` must be a whole number"]
    );
}

fn with_files(include_project: &str) -> String {
    SAMPLE.replace(
        "    binary: target/release/demo\n",
        &format!(
            "    binary: target/release/demo\n    files:\n      include:\n        project: {include_project}\n"
        ),
    )
}

#[test]
fn checks_patterns_at_launch() {
    let found = messages(&with_files("[\"dist/{a,b\"]"));
    assert_eq!(found, ["\"dist/{a,b\" has a `{` that is never closed"]);
    let found = messages(&with_files("[../secret]"));
    assert_eq!(found, ["\"../secret\" climbs out with `..`"]);
    let found = messages(&with_files("[/etc/passwd]"));
    assert!(found[0].contains("starts with /"), "{found:?}");
    let found = messages(&with_files("[dist, ./dist/]"));
    assert_eq!(found, ["\"./dist/\" is listed twice"]);
    let found = messages(&with_files("[release.json]"));
    assert_eq!(found, ["release.json belongs to shipdeck"]);
    let found = messages(&with_files("[demo.service]"));
    assert_eq!(found, ["demo.service belongs to shipdeck"]);
    // The same names deeper down are fine, and so is the whole project.
    assert!(parse(&with_files("[files/release.json, .]")).is_ok());
    let shared =
        with_files("[uploads]").replace("dir: /opt/demo }", "dir: /opt/demo, shared: [uploads] }");
    assert_eq!(
        messages(&shared),
        ["\"uploads\" is in `service.shared`, so it cannot be shipped too"]
    );
}

#[test]
fn something_must_ship_and_something_must_start() {
    let found = messages(&SAMPLE.replace("    binary: target/release/demo\n", "    binary:\n"));
    assert_eq!(
        found,
        [
            "`service.start` is needed when there is no `pipeline.pack.binary`",
            "nothing to ship",
        ]
    );
    // Negated patterns alone bring nothing in.
    let yaml = with_files("[\"!dist\"]").replace("    binary: target/release/demo\n", "");
    assert!(messages(&yaml).contains(&"nothing to ship".to_string()));
    let yaml = with_files("[dist]")
        .replace("    binary: target/release/demo\n", "")
        .replace(
            "dir: /opt/demo }",
            "dir: /opt/demo, start: node dist/main.js }",
        );
    assert!(parse(&yaml).is_ok());
}

#[test]
fn the_binary_is_the_default_start() {
    let demo = deployable(Path::new("/tmp/deploy/demo"), sample());
    assert_eq!(
        demo.start(),
        (Program::InRelease("demo".into()), String::new())
    );
    assert!(demo.required_tools().is_empty());

    let web = deployable(Path::new("/tmp/deploy/web"), parse(NODE).unwrap());
    assert_eq!(
        web.start(),
        (Program::Tool("node".into()), "dist/main.js".into())
    );
    assert_eq!(web.required_tools(), ["node", "npm"]);
    assert_eq!(web.binary_name(), None);
    assert_eq!(web.project_dir(), Path::new("/tmp/deploy/web/../../../web"));

    let mut config = sample();
    config.service.start = Some("/usr/bin/env demo".into());
    config.pipeline.preflight.requires = vec!["env".into(), "/usr/bin/env".into()];
    let absolute = deployable(Path::new("/tmp/deploy/demo"), config);
    assert_eq!(absolute.start().0, Program::Absolute("/usr/bin/env".into()));
    assert_eq!(absolute.required_tools(), ["/usr/bin/env", "env"]);
}

#[test]
fn version_file_is_found_in_the_project() {
    let dir = scratch("version");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    let mut config = sample();
    config.project = "app".into();
    let found = deployable(&dir, config.clone());
    assert_eq!(found.version_file(), None);
    std::fs::write(dir.join("app/package.json"), "{}").unwrap();
    std::fs::write(dir.join("app/VERSION"), "1.0.0").unwrap();
    assert_eq!(found.version_file(), Some(dir.join("app/package.json")));

    config.pipeline.build.version_from = Some("VERSION".into());
    let named = deployable(&dir, config);
    assert_eq!(named.version_file(), Some(dir.join("app/VERSION")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn release_paths_are_stored_without_dots_or_slashes() {
    assert_eq!(release_path("./files/"), "files");
    assert_eq!(release_path("files/./sub/a.txt"), "files/sub/a.txt");
    assert_eq!(release_path("."), "");
}

#[test]
fn user_may_come_from_ssh_config() {
    let config =
        parse(&SAMPLE.replace("{ host: example.com, user: deploy }", "{ host: prod }")).unwrap();
    assert_eq!(config.ssh.user, None);
}

#[test]
fn discovery_collects_every_problem_and_keeps_the_good_ones() {
    let root = scratch("discover");
    for name in ["api", "old", "twice", "broken"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("api/deploy.yml"), SAMPLE).unwrap();
    std::fs::write(root.join("old/deploy.json"), "{}").unwrap();
    std::fs::write(root.join("twice/deploy.yaml"), SAMPLE).unwrap();
    std::fs::write(root.join("twice/deploy.yml"), SAMPLE).unwrap();
    std::fs::write(
        root.join("broken/deploy.yaml"),
        SAMPLE.replace("/opt/demo", "opt/demo"),
    )
    .unwrap();

    let found = discover(&root).unwrap();
    let slugs: Vec<&str> = found.deployables.iter().map(|d| d.slug.as_str()).collect();
    assert_eq!(slugs, ["api"]);
    let problems: Vec<(String, String)> = found
        .problems
        .iter()
        .map(|file| {
            (
                file.path
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                file.problems[0].message.clone(),
            )
        })
        .collect();
    assert_eq!(
        problems,
        [
            (
                "broken".to_string(),
                "`service.dir` must be an absolute path".to_string()
            ),
            (
                "old".to_string(),
                "shipdeck reads deploy.yaml now, not deploy.json".to_string()
            ),
            (
                "twice".to_string(),
                "this folder has both deploy.yaml and deploy.yml".to_string()
            ),
        ]
    );
    assert!(found.problems[0].source.is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_headline_says_what_did_not_happen() {
    // Regression: ISSUE-002 — init's refusal said "shipdeck did not start"
    // Found by /qa on 2026-10-15
    // Report: .gstack/qa-reports/run-20261015T030609Z/qa-report-shipdeck-init-2026-10-15.md
    let files = [FileProblems::without_source(
        PathBuf::from("/work/deploy/my-api/deploy.yaml"),
        Problem::new("`ssh.user` is not a user name"),
    )];
    let text = render(&files, Path::new("/work"), false, "nothing written");
    assert!(
        text.starts_with("✘ nothing written: 1 deploy.yaml file has 1 problem"),
        "{text}"
    );
}

#[test]
fn renders_problems_like_a_compiler() {
    let text = "\
name: Demo
ssh: { host: example.com, user: deploy }
service: { name: demo, dir: /opt/demo }
pipeline:
  build:
    run:
      server: make
  pack:
    binary: target/release/demo
";
    let files = vec![FileProblems {
        path: PathBuf::from("/work/deploy/api/deploy.yaml"),
        source: Some(text.to_string()),
        problems: parse(text).unwrap_err(),
    }];
    let plain = render(&files, Path::new("/work"), false, "shipdeck did not start");
    assert_eq!(
        plain,
        "\
✘ shipdeck did not start: 1 deploy.yaml file has 1 problem

api/deploy.yaml
  error: build cannot run commands on the server
    ┌─ deploy/api/deploy.yaml:7:7
    │
  6 │     run:
  7 │       server: make
    │       ^^^^^^ build runs on your PC only
    │
    = help: the server sets up the release in `pipeline.setup.run.server`
"
    );
    let color = render(&files, Path::new("/work"), true, "shipdeck did not start");
    assert!(color.contains("\x1b[1;31merror\x1b[0m"), "{color:?}");
    assert!(
        color.contains("\x1b[35mpipeline.setup.run.server\x1b[0m"),
        "{color:?}"
    );
    assert!(color.contains("\x1b[1;33m^^^^^^ build runs on your PC only\x1b[0m"));
    assert!(color.contains("\x1b[1;32mhelp\x1b[0m"));
}

/// The examples in the repository must stay valid and carry the schema
/// line for this version.
#[test]
fn examples_parse_and_point_at_this_schema() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(&examples).unwrap() {
        let file = entry.unwrap().path().join(FILE_NAMES[0]);
        let text = std::fs::read_to_string(&file).unwrap();
        if let Err(problems) = parse(&text) {
            panic!("{}: {problems:#?}", file.display());
        }
        assert_eq!(
            text.lines().next(),
            Some(schema::modeline().as_str()),
            "{}",
            file.display()
        );
        seen += 1;
    }
    assert!(seen >= 4, "expected the rust, go, node and python examples");
}

/// `deploy.schema.json` at the repository root, and the website's copy, are
/// what these types say.
#[test]
fn the_published_schema_is_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [schema::FILE_NAME, schema::WEBSITE_COPY] {
        let committed = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        assert!(
            committed == schema::text(),
            "{file} is out of date; run: just schema"
        );
    }
}

#[test]
fn the_schema_closes_every_object_and_documents_every_field() {
    let schema = schema::generate();
    assert_eq!(schema["$id"], schema::url());
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    let mut objects = vec![&schema];
    objects.extend(schema["definitions"].as_object().unwrap().values());
    for object in objects {
        let Some(properties) = object.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        assert_eq!(object["additionalProperties"], false, "{object}");
        for (name, property) in properties {
            assert!(
                property.get("description").is_some(),
                "`{name}` needs a doc comment: editors show it"
            );
        }
    }
}
