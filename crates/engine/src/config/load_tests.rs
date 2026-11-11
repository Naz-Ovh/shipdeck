use std::path::{Path, PathBuf};

use super::*;
use crate::config::{Deploy, Env};
use shipdeck_protocol::message::SshOverride;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "shipdeck-load-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const DEPLOY: &str = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    process: { start: node dist/main.js, systemd: {} }
";

fn plain() -> ServerInputs {
    ServerInputs::default()
}

fn write_root(dir: &Path, body: &str) -> PathBuf {
    let file = dir.join("shipdeck.yaml");
    std::fs::write(&file, body).unwrap();
    file
}

#[test]
fn the_root_file_names_the_deploy_files() {
    let dir = scratch("root");
    write_root(
        &dir,
        "root: true\ndeploys:\n  production: deploy/production.yaml\n",
    );
    std::fs::create_dir_all(dir.join("deploy")).unwrap();
    std::fs::write(dir.join("deploy/production.yaml"), DEPLOY).unwrap();
    let root = Root::read(&dir, None).unwrap();
    let file = match root.deploys.get("production") {
        Some(Deploy::File(file)) => file.as_path(),
        other => panic!("{other:?}"),
    };
    assert_eq!(file, dir.join("deploy/production.yaml"));
    let deployment = root
        .deployment("production", plain().details(false))
        .unwrap();
    assert_eq!(deployment.slug, "production");
    assert_eq!(deployment.name(), "shop");
    assert_eq!(deployment.dir(), "/opt/shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_written_in_the_root_file_is_read_from_its_folder() {
    let dir = scratch("in-place");
    write_root(
        &dir,
        "root: true\ndeploys:\n  production:\n    name: shop\n    server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }\n    apps:\n      web:\n        source: apps/web\n        static: true\n        files: [apps/web/dist]\n",
    );
    let root = Root::read(&dir, None).unwrap();
    let deployment = root
        .deployment("production", plain().details(false))
        .unwrap();
    assert_eq!(deployment.slug, "production");
    assert_eq!(deployment.name(), "shop");
    assert_eq!(deployment.config_dir, dir);
    assert_eq!(deployment.source("web"), dir.join("apps/web"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_in_place_expands_config_from_the_root_files_folder() {
    let dir = scratch("in-place-config");
    write_root(
        &dir,
        "root: true\ndeploys:\n  production:\n    name: shop\n    server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }\n    release:\n      steps:\n        build:\n          local: cp {{CONFIG}}/note.txt .\n    apps:\n      web:\n        source: .\n        static: true\n        files: [note.txt]\n",
    );
    let root = Root::read(&dir, None).unwrap();
    let deployment = root
        .deployment("production", plain().details(false))
        .unwrap();
    let expanded = deployment
        .expand(
            "cp {{CONFIG}}/note.txt .",
            Some("v1.0.0"),
            Some("1.0.0"),
            None,
        )
        .unwrap();
    assert_eq!(expanded, format!("cp {}/note.txt .", dir.display()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_problem_in_a_deploy_written_in_place_points_at_its_line() {
    let dir = scratch("in-place-line");
    write_root(
        &dir,
        "root: true\ndeploys:\n  production:\n    name: shop\n    server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }\n    apps:\n      web:\n        process: { start: node x }\n",
    );
    let problems = Root::read(&dir, None).unwrap_err();
    let missing = problems
        .problems
        .iter()
        .find(|problem| problem.message.contains("manager"))
        .unwrap_or_else(|| panic!("{problems:#?}"));
    let at = missing.at.expect("a line");
    assert_eq!(at.start.0, 8, "{problems:#?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_root_file_needs_root_true() {
    let dir = scratch("no-root");
    write_root(&dir, "deploys: {}\n");
    let problems = Root::read(&dir, None).unwrap_err();
    assert!(
        problems
            .problems
            .iter()
            .any(|problem| problem.message.contains("root` is missing")
                || problem.message.contains("unknown key `root`")
                || problem.message.contains("is not a root file")
                || problem.message.contains("could not be read")),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn root_false_is_refused() {
    let dir = scratch("root-false");
    write_root(&dir, "root: false\ndeploys: {}\n");
    let problems = Root::read(&dir, None).unwrap_err();
    assert!(
        problems
            .problems
            .iter()
            .any(|problem| problem.message.contains("must be true")
                || problem.message.contains("is not a root file")
                || problem.message.contains("could not be read")),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_name_never_holds_a_dot() {
    let dir = scratch("name-dot");
    std::fs::write(dir.join("prod.yaml"), DEPLOY).unwrap();
    write_root(&dir, "root: true\ndeploys:\n  prod.1: prod.yaml\n");
    let problems = Root::read(&dir, None).unwrap_err();
    assert!(
        problems
            .problems
            .iter()
            .any(|problem| problem.message.contains("is not a deploy name")),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_without_a_root_file_names_what_it_expected() {
    let dir = scratch("missing");
    let problems = Root::read(&dir, None).unwrap_err();
    let problem = &problems.problems[0];
    let text = format!(
        "{} {} {}",
        problem.message,
        problem.label,
        problem.help.clone().unwrap_or_default()
    );
    assert!(text.contains(&dir.display().to_string()), "{text}");
    assert!(text.contains("shipdeck.yaml"), "{text}");
    assert!(text.contains(".shipdeck"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn both_root_file_names_are_an_error() {
    let dir = scratch("both");
    std::fs::write(dir.join("shipdeck.yaml"), "root: true\ndeploys: {}\n").unwrap();
    std::fs::write(dir.join(".shipdeck"), "root: true\ndeploys: {}\n").unwrap();
    let problems = Root::read(&dir, None).unwrap_err();
    assert!(
        problems.problems[0].message.contains("both"),
        "{problems:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shipdeck_yml_is_not_read_as_a_root_file() {
    let dir = scratch("yml");
    std::fs::write(dir.join("shipdeck.yml"), "root: true\ndeploys: {}\n").unwrap();
    assert!(Root::read(&dir, None).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_file_lives_anywhere_and_needs_no_root_file() {
    let dir = scratch("file");
    let file = dir.join("production.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let deployment = load_deploy(&dir, &file, None, plain().details(false)).unwrap();
    assert_eq!(deployment.slug, "shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opening_named_files_reads_no_root_file() {
    let dir = scratch("named");
    let file = dir.join("staging.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let (deploys, problems) = open_all(&dir, None, std::slice::from_ref(&file), &plain());
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(deploys.len(), 1);
    assert_eq!(deploys[0].slug, "shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_old_config_is_refused_by_its_unknown_keys() {
    let old = "\
name: Web
dir: /opt/web
ssh: { host: example.com, user: deploy }
service: { name: web }
";
    let problems = parse_deploy(old).unwrap_err();
    assert!(
        problems.iter().any(|p| p.message.contains("unknown key")),
        "{problems:?}"
    );
}

#[test]
fn a_deploy_can_be_read_back_from_its_settings() {
    let dir = scratch("settings");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let deployment = load_deploy(&dir, &file, Some("shop"), plain().details(false)).unwrap();
    let settings = deployment.settings().unwrap();
    let restored = Deployment::from_settings(
        &settings,
        Path::new("/tmp/project"),
        Path::new("/tmp/project"),
        file,
        "shop".into(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&restored.deploy).unwrap(),
        serde_json::to_value(&deployment.deploy).unwrap()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_file_named_by_config_is_named_as_one() {
    let dir = scratch("config-is-deploy");
    let file = dir.join("production.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let problems = Root::read(&dir, Some(&file)).unwrap_err();
    let text = &problems.problems[0];
    assert!(text.message.contains("a deploy file"), "{text:?}");
    let help = text.help.clone().unwrap_or_default();
    assert!(help.contains("shipdeck deploy"), "{help}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_root_file_named_as_a_deploy_file_is_named_as_one() {
    let dir = scratch("deploy-is-root");
    let file = dir.join("shipdeck.yaml");
    std::fs::write(&file, "root: true\ndeploys: {}\n").unwrap();
    let problems = load_deploy(&dir, &file, None, plain().details(false)).unwrap_err();
    let text = &problems.problems[0];
    assert!(text.message.contains("a root file"), "{text:?}");
    let help = text.help.clone().unwrap_or_default();
    assert!(help.contains("-c"), "{help}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_name_the_root_file_does_not_list_says_what_it_lists() {
    let dir = scratch("unknown-name");
    write_root(&dir, "root: true\ndeploys:\n  production: deploy.yaml\n");
    std::fs::write(dir.join("deploy.yaml"), DEPLOY).unwrap();
    let root = Root::read(&dir, None).unwrap();
    let problems = root
        .deployment("staging", plain().details(false))
        .unwrap_err();
    assert!(
        problems.problems[0].message.contains("shipdeck.yaml"),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

const VARIABLES: &str = "\
name: shop
server:
  ssh: { host: '${SHOP_TEST_HOST}', port: '${SHOP_TEST_PORT}', user: '${SHOP_TEST_USER}' }
  dir: /opt/shop
apps:
  web:
    process: { start: node dist/main.js, systemd: {} }
";

fn variables(vars: &[(&str, &str)], flags: SshOverride) -> ServerInputs {
    ServerInputs::new(Env::from_pairs(vars.iter().copied()), flags, Vec::new())
}

const SET: [(&str, &str); 3] = [
    ("SHOP_TEST_HOST", "203.0.113.7"),
    ("SHOP_TEST_PORT", "2222"),
    ("SHOP_TEST_USER", "ops"),
];

#[test]
fn a_deploy_file_takes_its_server_from_variables() {
    let dir = scratch("variables");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, VARIABLES).unwrap();
    let given = variables(&SET, SshOverride::default());
    let deployment = load_deploy(&dir, &file, None, given.details(false)).unwrap();
    assert_eq!(deployment.endpoint.host, "203.0.113.7");
    assert_eq!(deployment.endpoint.port, 2222);
    assert_eq!(deployment.endpoint.user, "ops");
    assert_eq!(deployment.ssh_from, Vec::<String>::new());
    // The release's settings, and `{{HOST}}`, hold the final values.
    let settings = String::from_utf8(deployment.settings().unwrap()).unwrap();
    assert!(
        settings.contains("203.0.113.7") && !settings.contains("SHOP_TEST"),
        "{settings}"
    );
    let restored =
        Deployment::from_settings(settings.as_bytes(), &dir, &dir, file, "shop".into()).unwrap();
    assert_eq!(restored.endpoint.port, 2222);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unset_variable_points_at_its_line_in_the_deploy_file() {
    let dir = scratch("unset");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, VARIABLES).unwrap();
    let given = variables(
        &[("SHOP_TEST_PORT", "22"), ("SHOP_TEST_USER", "ops")],
        SshOverride::default(),
    );
    let found = load_deploy(&dir, &file, None, given.details(false)).unwrap_err();
    let problem = &found.problems[0];
    assert!(problem.message.contains("SHOP_TEST_HOST"), "{problem:?}");
    assert!(problem.message.contains("server.ssh.host"), "{problem:?}");
    assert_eq!(problem.at.map(|at| at.start.0), Some(3));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_in_the_root_file_takes_its_server_from_variables() {
    let dir = scratch("variables-in-place");
    write_root(
        &dir,
        "root: true\ndeploys:\n  production:\n    name: shop\n    server: { ssh: { host: '${SHOP_TEST_HOST}', user: '${SHOP_TEST_USER}' }, dir: /opt/shop }\n    apps:\n      web:\n        static: true\n        files: [dist]\n",
    );
    let root = Root::read(&dir, None).unwrap();
    let given = variables(&SET, SshOverride::default());
    let deployment = root.deployment("production", given.details(false)).unwrap();
    assert_eq!(deployment.endpoint.host, "203.0.113.7");
    assert_eq!(deployment.endpoint.user, "ops");
    let none = variables(&[], SshOverride::default());
    let found = root
        .deployment("production", none.details(false))
        .unwrap_err();
    assert_eq!(found.problems[0].at.map(|at| at.start.0), Some(5));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_variable_outside_the_three_fields_stays_plain_text() {
    let dir = scratch("elsewhere");
    let file = dir.join("shop.yaml");
    let text = DEPLOY.replace(
        "process: { start: node dist/main.js",
        "process: { start: 'echo ${HOME}'",
    );
    std::fs::write(&file, text).unwrap();
    let deployment = load_deploy(&dir, &file, None, plain().details(false)).unwrap();
    assert!(deployment.start("web").unwrap().1.contains("${HOME}"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn server_dir_does_not_take_variables() {
    let dir = scratch("dir");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, DEPLOY.replace("/opt/shop", "'/opt/${SHOP}'")).unwrap();
    let found = load_deploy(&dir, &file, None, plain().details(false)).unwrap_err();
    assert!(
        found.problems[0].message.contains("server.dir"),
        "{found:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn one_deploy_file_takes_the_flags_and_says_where_they_came_from() {
    let dir = scratch("flags");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, VARIABLES).unwrap();
    let flags = SshOverride {
        host: Some("198.51.100.4".into()),
        ..SshOverride::default()
    };
    let given = variables(
        &[("SHIPDECK_SSH_PORT", "2201"), ("SHOP_TEST_USER", "ops")],
        flags,
    );
    let (deploys, problems) = open_all(&dir, None, std::slice::from_ref(&file), &given);
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(deploys[0].endpoint.host, "198.51.100.4");
    assert_eq!(deploys[0].endpoint.port, 2201);
    assert_eq!(
        deploys[0].ssh_from,
        ["host from --host", "port from SHIPDECK_SSH_PORT"]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn two_deploy_files_refuse_the_flags() {
    let dir = scratch("two-files");
    let (a, b) = (dir.join("a.yaml"), dir.join("b.yaml"));
    std::fs::write(&a, DEPLOY).unwrap();
    std::fs::write(&b, DEPLOY).unwrap();
    let flags = SshOverride {
        host: Some("198.51.100.4".into()),
        ..SshOverride::default()
    };
    let given = variables(&[], flags);
    let (deploys, problems) = open_all(&dir, None, &[a, b], &given);
    assert!(deploys.is_empty());
    assert!(
        problems[0].problems[0].message.contains("2 deploys"),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_root_file_with_two_deploys_refuses_the_variables_unless_the_run_names_one() {
    let dir = scratch("two-deploys");
    write_root(&dir, "root: true\ndeploys:\n  a: a.yaml\n  b: b.yaml\n");
    std::fs::write(dir.join("a.yaml"), DEPLOY).unwrap();
    std::fs::write(dir.join("b.yaml"), DEPLOY).unwrap();
    let vars = [("SHIPDECK_SSH_HOST", "198.51.100.4")];
    // The whole root file: what the TUI opens.
    let given = variables(&vars, SshOverride::default());
    let (deploys, problems) = open_all(&dir, None, &[], &given);
    assert!(deploys.is_empty());
    let message = &problems[0].problems[0].message;
    assert!(
        message.contains("SHIPDECK_SSH_HOST") && message.contains("2 deploys"),
        "{message}"
    );
    // `deploy a`: only that deploy takes the variable.
    let given = ServerInputs::new(
        Env::from_pairs(vars),
        SshOverride::default(),
        vec!["a".into()],
    );
    let (deploys, problems) = open_all(&dir, None, &[], &given);
    assert!(problems.is_empty(), "{problems:#?}");
    let host = |slug: &str| {
        deploys
            .iter()
            .find(|found| found.slug == slug)
            .map(|found| found.endpoint.host.clone())
    };
    assert_eq!(host("a").as_deref(), Some("198.51.100.4"));
    assert_eq!(host("b").as_deref(), Some("example.com"));
    // `deploy a b`.
    let given = ServerInputs::new(
        Env::from_pairs(vars),
        SshOverride::default(),
        vec!["a".into(), "b".into()],
    );
    let (_, problems) = open_all(&dir, None, &[], &given);
    assert!(!problems.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_root_file_with_one_deploy_takes_the_variables_without_a_name() {
    let dir = scratch("one-deploy");
    write_root(&dir, "root: true\ndeploys:\n  a: a.yaml\n");
    std::fs::write(dir.join("a.yaml"), DEPLOY).unwrap();
    let given = variables(&[("SHIPDECK_SSH_USER", "ops")], SshOverride::default());
    let (deploys, problems) = open_all(&dir, None, &[], &given);
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(deploys[0].endpoint.user, "ops");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A root file with `prod` and `staging`, each naming its own host variable.
fn two_servers(name: &str) -> PathBuf {
    let dir = scratch(name);
    write_root(
        &dir,
        "root: true\ndeploys:\n  prod: prod.yaml\n  staging: staging.yaml\n",
    );
    std::fs::write(
        dir.join("prod.yaml"),
        DEPLOY.replace("host: example.com", "host: '${PROD_TEST_HOST}'"),
    )
    .unwrap();
    std::fs::write(
        dir.join("staging.yaml"),
        DEPLOY.replace("host: example.com", "host: '${STAGING_TEST_HOST}'"),
    )
    .unwrap();
    dir
}

fn only_prod(run: &[&str]) -> ServerInputs {
    ServerInputs::new(
        Env::from_pairs([("PROD_TEST_HOST", "203.0.113.20")]),
        SshOverride::default(),
        run.iter().map(|name| (*name).to_string()).collect(),
    )
}

fn find<'a>(deploys: &'a [Deployment], slug: &str) -> &'a Deployment {
    deploys.iter().find(|found| found.slug == slug).unwrap()
}

#[test]
fn a_missing_variable_of_another_deploy_does_not_stop_the_run() {
    let dir = two_servers("other-missing");
    let (deploys, problems) = open_all(&dir, None, &[], &only_prod(&["prod"]));
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(find(&deploys, "prod").unavailable, None);
    assert_eq!(find(&deploys, "prod").endpoint.host, "203.0.113.20");
    let reason = find(&deploys, "staging").unavailable.clone().unwrap();
    assert!(reason.contains("STAGING_TEST_HOST"), "{reason}");
    assert!(reason.contains("server.ssh.host"), "{reason}");
    assert!(
        reason.contains(".env") && reason.contains("--host"),
        "{reason}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_variable_of_the_deploy_in_the_run_is_an_error() {
    let dir = two_servers("own-missing");
    for run in [&["staging"][..], &["prod", "staging"][..]] {
        let (_, problems) = open_all(&dir, None, &[], &only_prod(run));
        let message = &problems[0].problems[0].message;
        assert!(message.contains("STAGING_TEST_HOST"), "{message}");
        assert!(message.contains("the deploy staging"), "{message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_listing_marks_the_deploy_unavailable_and_the_rest_work() {
    let dir = two_servers("listing");
    let (deploys, problems) = open_all(&dir, None, &[], &only_prod(&[]));
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(deploys.len(), 2);
    assert_eq!(find(&deploys, "prod").unavailable, None);
    assert!(find(&deploys, "staging").unavailable.is_some());
    assert_eq!(find(&deploys, "staging").dir(), "/opt/shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn only_a_missing_variable_makes_a_deploy_unavailable() {
    let dir = two_servers("other-errors");
    // A bad variable name, and a key the schema does not know, stay errors
    // even for a deploy the run does not use.
    std::fs::write(
        dir.join("staging.yaml"),
        DEPLOY.replace("host: example.com", "host: '${1X}'"),
    )
    .unwrap();
    let (_, problems) = open_all(&dir, None, &[], &only_prod(&["prod"]));
    assert!(
        problems[0].problems[0]
            .message
            .contains("not a variable name")
    );
    std::fs::write(dir.join("staging.yaml"), format!("{DEPLOY}bogus: 1\n")).unwrap();
    let (_, problems) = open_all(&dir, None, &[], &only_prod(&["prod"]));
    assert!(
        problems[0].problems[0].message.contains("unknown key"),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn deploy_files_named_on_the_command_line_are_always_in_the_run() {
    let dir = two_servers("files");
    let file = dir.join("staging.yaml");
    let (deploys, problems) = open_all(&dir, None, std::slice::from_ref(&file), &only_prod(&[]));
    assert!(deploys.is_empty());
    assert!(
        problems[0].problems[0]
            .message
            .contains("STAGING_TEST_HOST")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_flag_replaces_a_missing_variable_of_the_deploy_it_names() {
    let dir = two_servers("flag");
    let flags = SshOverride {
        host: Some("198.51.100.9".into()),
        ..SshOverride::default()
    };
    let given = ServerInputs::new(Env::empty(), flags, vec!["staging".into()]);
    let (deploys, problems) = open_all(&dir, None, &[], &given);
    // prod is not in the run, so it is only unavailable.
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(find(&deploys, "staging").endpoint.host, "198.51.100.9");
    assert!(find(&deploys, "prod").unavailable.is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}
