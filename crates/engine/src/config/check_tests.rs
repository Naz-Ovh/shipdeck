fn messages(yaml: &str) -> Vec<String> {
    match super::super::load::parse_deploy(yaml) {
        Ok(_) => Vec::new(),
        Err(problems) => problems.into_iter().map(|p| p.message).collect(),
    }
}

fn sample() -> &'static str {
    "\
name: shop
server:
  ssh: { host: example.com, user: deploy }
  dir: /opt/shop
release:
  steps:
    build:
      local: npm run build
apps:
  web:
    files: [dist]
    process:
      start: node dist/main.js
      systemd: {}
"
}

#[test]
fn a_clean_minimal_deploy_reads() {
    assert!(messages(sample()).is_empty(), "{:?}", messages(sample()));
}

#[test]
fn an_unknown_key_names_the_close_one() {
    let yaml = sample().replace("  dir: /opt/shop", "  dir: /opt/shop\n  dirt: /opt/x");
    let found = messages(&yaml);
    assert!(
        found
            .iter()
            .any(|m| m.contains("unknown key `server.dirt`")),
        "{found:?}"
    );
    assert!(
        super::super::load::parse_deploy(&yaml)
            .unwrap_err()
            .iter()
            .any(|p| p.help.as_deref().is_some_and(|h| h.contains("dir"))),
        "the hint should name `dir`"
    );
}

#[test]
fn a_command_that_reads_as_a_map_suggests_quotes() {
    let yaml = sample().replace("local: npm run build", "local:\n      - a: b");
    let problems = super::super::load::parse_deploy(&yaml).unwrap_err();
    assert!(
        problems.iter().any(|p| p.message.contains("command map")
            && p.help.as_deref().is_some_and(|h| h.contains("quotes"))),
        "{problems:?}"
    );
}

#[test]
fn a_runner_map_is_one_command() {
    let yaml = sample().replace("local: npm run build", "local:\n      - bash: echo hi");
    let deploy = super::super::load::parse_deploy(&yaml).unwrap();
    let command = &deploy.release.steps.build.local.0[0];
    assert!(matches!(
        command,
        super::super::types::Command::Runner { .. }
    ));
}

#[test]
fn required_keys_are_named() {
    let yaml = "name: shop\nserver:\n  ssh: { host: example.com }\n";
    let found = messages(yaml);
    assert!(
        found.iter().any(|m| m.contains("`server.dir` is missing")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|m| m.contains("`apps` is missing")),
        "{found:?}"
    );
}

#[test]
fn crlf_is_refused() {
    let found = messages(&sample().replace('\n', "\r\n"));
    assert!(found[0].contains("CRLF"), "{found:?}");
}
