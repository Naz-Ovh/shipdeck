use crate::config::fixtures::*;

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
