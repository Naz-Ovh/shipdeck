use crate::config::*;

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
