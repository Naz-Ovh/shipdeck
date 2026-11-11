use std::path::{Path, PathBuf};

use super::*;
use crate::config::Deploy;

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
    let deployment = root.deployment("production").unwrap();
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
    let deployment = root.deployment("production").unwrap();
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
    let deployment = root.deployment("production").unwrap();
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
    let deployment = load_deploy(&dir, &file, None).unwrap();
    assert_eq!(deployment.slug, "shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opening_named_files_reads_no_root_file() {
    let dir = scratch("named");
    let file = dir.join("staging.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let (deploys, problems) = open_all(&dir, None, std::slice::from_ref(&file));
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
    let deployment = load_deploy(&dir, &file, Some("shop")).unwrap();
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
    let problems = load_deploy(&dir, &file, None).unwrap_err();
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
    let problems = root.deployment("staging").unwrap_err();
    assert!(
        problems.problems[0].message.contains("shipdeck.yaml"),
        "{problems:#?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
