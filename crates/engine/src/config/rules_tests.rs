use crate::config::load::parse_deploy;

fn messages(yaml: &str) -> Vec<String> {
    match parse_deploy(yaml) {
        Ok(_) => Vec::new(),
        Err(problems) => problems.into_iter().map(|p| p.message).collect(),
    }
}

const SAMPLE: &str = "\
name: shop
server:
  ssh: { host: example.com, user: deploy }
  dir: /opt/shop
release:
  steps:
    build:
      local: npm run build
    backup:
      keep: { open: 5, compressed: 15 }
apps:
  web:
    source: web
    files: [web/dist]
    process:
      start: node dist/main.js
      systemd: {}
";

#[test]
fn a_deploy_needs_a_process_or_a_static_site() {
    let yaml = SAMPLE.replace(
        "    process:\n      start: node dist/main.js\n      systemd: {}",
        "",
    );
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("one of `process` or `static")),
        "{found:?}"
    );
}

#[test]
fn the_process_type_holds_one_manager_or_none() {
    let one: crate::config::types::DeployFile = crate::config::load::parse_deploy(SAMPLE).unwrap();
    let process = one.apps["web"].process.as_ref().unwrap();
    assert!(matches!(
        process.manager,
        Some(crate::config::types::KeepUp::Systemd(_))
    ));
    // Two manager keys cannot be read as a `Process` at all, so no value of
    // the type can hold two.
    let two = SAMPLE.replace("      systemd: {}", "      systemd: {}\n      pm2: {}");
    let process_json =
        serde_json::json!({ "start": "node dist/main.js", "systemd": {}, "pm2": {} });
    let read: Result<crate::config::types::Process, _> = serde_json::from_value(process_json);
    assert!(
        read.is_err(),
        "two manager keys must not read as one Process"
    );
    let found = messages(&two);
    assert!(
        found.iter().any(|m| m.contains("exactly one manager")),
        "{found:?}"
    );
}

#[test]
fn a_process_takes_exactly_one_manager() {
    let yaml = SAMPLE.replace("      systemd: {}", "      systemd: {}\n      pm2: {}");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("exactly one manager")),
        "{found:?}"
    );
    let yaml = SAMPLE.replace("      systemd: {}", "");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("needs one manager")),
        "{found:?}"
    );
}

#[test]
fn shipdeck_own_systemd_properties_are_refused() {
    let yaml = SAMPLE.replace(
        "      systemd: {}",
        "      systemd:\n        options:\n          Service: { ExecStart: /bin/true }",
    );
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("shipdeck writes `ExecStart` itself")),
        "{found:?}"
    );
}

#[test]
fn systemd_instances_are_reserved() {
    let yaml = SAMPLE.replace("      systemd: {}", "      systemd: { instances: 3 }");
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("planned, not supported yet")),
        "{found:?}"
    );
}

#[test]
fn pm2_reserved_keys_and_ecosystem_rules() {
    let yaml = SAMPLE.replace(
        "      systemd: {}",
        "      pm2:\n        options: { name: other, cwd: /tmp }",
    );
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("shipdeck sets the PM2 `name`")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("shipdeck sets the PM2 `cwd`")),
        "{found:?}"
    );

    let yaml = SAMPLE.replace(
        "      start: node dist/main.js\n      systemd: {}",
        "      start: node dist/main.js\n      pm2: { ecosystem: ecosystem.config.js }",
    );
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("both say what runs")),
        "{found:?}"
    );
}

#[test]
fn a_static_app_refuses_the_keys_the_design_lists() {
    let static_app = "\
apps:
  web:
    source: web
    files: [web/dist]
    static: true
    nginx:
      domains: [example.com]
      root: dist
      index: index.html
";
    let plain = format!(
        "name: shop\nserver: {{ ssh: {{ host: example.com, user: deploy }}, dir: /opt/shop }}\n{static_app}"
    );
    assert!(messages(&plain).is_empty(), "{:?}", messages(&plain));

    for (change, why) in [
        ("    binary: dist/web\n", "binary"),
        ("    shared: [cache]\n", "shared"),
        (
            "    steps:\n      setup:\n        server: npm ci\n",
            "steps.setup",
        ),
        (
            "    steps:\n      backup:\n        include: [cache]\n",
            "steps.backup.include",
        ),
    ] {
        let yaml = format!(
            "name: shop\nserver: {{ ssh: {{ host: example.com, user: deploy }}, dir: /opt/shop }}\n{static_app}{change}"
        );
        let found = messages(&yaml);
        assert!(
            found
                .iter()
                .any(|m| m.contains(why) && m.contains("static")),
            "{why}: {found:?}"
        );
    }
}

#[test]
fn nginx_needs_the_right_fields_per_kind() {
    let static_app = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    static: true
    files: [dist]
    nginx:
      domains: [example.com]
";
    let found = messages(static_app);
    assert!(
        found.iter().any(|m| m.contains("needs `root`")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("needs `index`")),
        "{found:?}"
    );

    let process = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    files: [dist]
    process: { start: node x, systemd: {} }
    nginx: { domains: [example.com] }
";
    let found = messages(process);
    assert!(
        found.iter().any(|m| m.contains("needs `proxy_pass`")),
        "{found:?}"
    );

    let wrong = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    files: [dist]
    static: true
    nginx:
      domains: [example.com]
      root: dist
      index: index.html
      proxy_pass: http://127.0.0.1:3000
";
    let found = messages(wrong);
    assert!(
        found
            .iter()
            .any(|m| m.contains("for a process app, not a static")),
        "{found:?}"
    );
}

#[test]
fn an_app_file_outside_the_source_needs_to() {
    let yaml = SAMPLE.replace(
        "    files: [web/dist]",
        "    files:\n      - {path: ./web/dist, to: dist}\n      - other/file",
    );
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("outside the app's `source`, so it needs `to`")),
        "{found:?}"
    );
}

#[test]
fn a_reserved_placeholder_and_a_process_only_name_are_refused() {
    let yaml = SAMPLE.replace("name: shop", "name: \"{{APP}}\"");
    let found = messages(&yaml);
    assert!(
        super::super::load::parse_deploy(&yaml)
            .unwrap_err()
            .iter()
            .any(|p| p
                .help
                .as_deref()
                .is_some_and(|h| h.contains("only valid inside an app"))),
        "{found:?}"
    );

    let yaml = SAMPLE.replace(
        "start: node dist/main.js",
        "start: node dist/{{INSTANCE}}.js",
    );
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("planned, not supported yet")),
        "{found:?}"
    );

    let yaml = SAMPLE.replace(
        "start: node dist/main.js",
        "start: node {{PROCESS}}/main.js",
    );
    assert!(messages(&yaml).is_empty(), "{:?}", messages(&yaml));

    let static_app = SAMPLE.replace(
        "    process:\n      start: node dist/main.js\n      systemd: {}",
        "    static: true\n    steps:\n      done:\n        local: echo {{PROCESS}}",
    );
    let found = messages(&static_app);
    assert!(
        super::super::load::parse_deploy(&static_app)
            .unwrap_err()
            .iter()
            .any(|p| p
                .help
                .as_deref()
                .is_some_and(|h| h.contains("only valid inside a process app"))),
        "{found:?}"
    );
}

#[test]
fn release_names_are_known_from_build_on() {
    let yaml = SAMPLE.replace(
        "    process:",
        "    steps:\n      preflight:\n        local: echo {{RELEASE}}\n    process:",
    );
    let found = messages(&yaml);
    assert!(
        super::super::load::parse_deploy(&yaml)
            .unwrap_err()
            .iter()
            .any(|p| p
                .help
                .as_deref()
                .is_some_and(|h| h.contains("named at the start of `build`"))),
        "{found:?}"
    );
    let later = SAMPLE.replace(
        "    process:",
        "    steps:\n      done:\n        local: echo {{RELEASE}}\n    process:",
    );
    assert!(messages(&later).is_empty(), "{:?}", messages(&later));
}

#[test]
fn a_process_name_with_at_is_reserved() {
    let yaml = SAMPLE.replace("      systemd: {}", "      name: api@1\n      systemd: {}");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("systemd reserves")),
        "{found:?}"
    );
}

#[test]
fn shared_folders_may_not_overlap_or_touch_shipdeck() {
    let yaml = SAMPLE.replace("apps:", "  shared: [storage, storage/cache]\napps:");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("which overlap")),
        "{found:?}"
    );
    let yaml = SAMPLE.replace("apps:", "  shared: [.shipdeck]\napps:");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("belongs to shipdeck")),
        "{found:?}"
    );
}

#[test]
fn keep_open_counts_the_live_release() {
    let yaml = SAMPLE.replace("open: 5", "open: 0");
    let found = messages(&yaml);
    assert!(found.iter().any(|m| m.contains("at least 1")), "{found:?}");
}

#[test]
fn an_empty_command_is_refused() {
    let yaml = SAMPLE.replace("local: npm run build", "local: \"  \"");
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("command is empty")),
        "{found:?}"
    );
}

#[test]
fn a_binary_takes_both_forms() {
    let plain = SAMPLE.replace(
        "    files: [web/dist]",
        "    files: [web/dist]\n    binary: target/release/api",
    );
    assert!(messages(&plain).is_empty(), "{:?}", messages(&plain));
    let mapped = SAMPLE.replace(
        "    files: [web/dist]",
        "    files: [web/dist]\n    binary: { path: target/release/api, to: bin/api }",
    );
    assert!(messages(&mapped).is_empty(), "{:?}", messages(&mapped));
}

#[test]
fn a_binary_and_a_file_take_a_map_with_no_to() {
    let binary = SAMPLE.replace(
        "    files: [web/dist]",
        "    files: [web/dist]\n    binary: { path: target/release/api }",
    );
    assert!(messages(&binary).is_empty(), "{:?}", messages(&binary));
    let file = SAMPLE.replace("    files: [web/dist]", "    files: [{ path: web/dist }]");
    assert!(messages(&file).is_empty(), "{:?}", messages(&file));
    // A path outside the project still needs `to`.
    let outside = SAMPLE.replace(
        "    files: [web/dist]",
        "    files: [{ path: ../elsewhere }]",
    );
    let found = messages(&outside);
    assert!(found.iter().any(|m| m.contains("needs `to`")), "{found:?}");
}

#[test]
fn a_binary_to_is_a_release_path() {
    for to in ["/bin/api", "../api", ".shipdeck/api"] {
        let yaml = SAMPLE.replace(
            "    files: [web/dist]",
            &format!("    files: [web/dist]\n    binary: {{ path: target/release/api, to: {to} }}"),
        );
        let found = messages(&yaml);
        assert!(
            found
                .iter()
                .any(|m| m.contains("binary.to") || m.contains("belongs to shipdeck")),
            "{to}: {found:?}"
        );
    }
}

#[test]
fn a_binary_pattern_is_refused() {
    let yaml = SAMPLE.replace(
        "    files: [web/dist]",
        "    files: [web/dist]\n    binary: target/release/*",
    );
    let found = messages(&yaml);
    assert!(
        found.iter().any(|m| m.contains("must name one file")),
        "{found:?}"
    );
}

#[test]
fn a_static_app_refuses_a_binary_in_both_forms() {
    for binary in [
        "target/release/web",
        "{ path: target/release/web, to: bin/web }",
    ] {
        let yaml = format!(
            "name: shop\nserver: {{ ssh: {{ host: example.com, user: deploy }}, dir: /opt/shop }}\napps:\n  web:\n    static: true\n    files: [dist]\n    binary: {binary}\n"
        );
        let found = messages(&yaml);
        assert!(
            found
                .iter()
                .any(|m| m.contains("does not apply to a static app")),
            "{binary}: {found:?}"
        );
    }
}
