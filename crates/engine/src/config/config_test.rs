use super::fixtures::*;
use super::*;

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
fn refuses_crlf_line_endings() {
    let problems = parse(&SAMPLE.replace('\n', "\r\n")).unwrap_err();
    assert_eq!(problems.len(), 1);
    assert!(problems[0].message.contains("CRLF"), "{}", problems[0]);
    let first = SAMPLE.lines().next().unwrap().chars().count();
    assert_eq!(problems[0].at, Some(Span::point(1, first)));

    let lone = parse("project: .\nname: my-api\r").unwrap_err();
    assert_eq!(lone[0].at, Some(Span::point(2, 12)));
}

#[test]
fn user_may_come_from_ssh_config() {
    let config =
        parse(&SAMPLE.replace("{ host: example.com, user: deploy }", "{ host: prod }")).unwrap();
    assert_eq!(config.ssh.user, None);
}
