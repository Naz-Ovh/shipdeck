use crate::config::fixtures::*;
use crate::config::*;

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
