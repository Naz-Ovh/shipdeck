use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::release_plan;
use crate::config::load::parse_deploy;
use crate::release::ReleaseName;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

fn deployment(yaml: &str, project: &Path) -> crate::config::Deployment {
    let deploy = parse_deploy(yaml).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    crate::config::Deployment {
        slug: "prod".into(),
        file: project.join("deploy.yaml"),
        project: project.to_path_buf(),
        config_dir: project.to_path_buf(),
        yaml: yaml.to_string(),
        endpoint,
        ssh_from: Vec::new(),
        unavailable: None,
        deploy,
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

#[test]
fn files_land_where_the_design_says() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "package.json", "{}");
    write(&root, "packages/a/index.js", "x");
    write(&root, "packages/a/big.map", "x");
    write(&root, "services/api/dist/server.js", "x");
    write(&root, "services/api/bin/api", "x");
    write(&root, "prod/api.conf", "x");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  files: [package.json, packages]
  exclude: [\"**/*.map\"]
  shared: [storage]
apps:
  api:
    source: services/api
    files:
      - services/api/dist
      - { path: ./prod/api.conf, to: api.conf }
    binary: services/api/bin/api
    shared: [uploads]
    process: { start: node dist/main.js, systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1.4.0", "abc1234", false, chrono::DateTime::UNIX_EPOCH);
    let plan = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap();
    let paths: Vec<&str> = plan
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    for expected in [
        "package.json",
        "packages/a/index.js",
        "apps/api/dist/server.js",
        "apps/api/api.conf",
        "apps/api/api",
        ".shipdeck/release.json",
        ".shipdeck/settings.json",
        ".shipdeck/shop-api.service",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }
    assert!(
        !paths
            .iter()
            .any(|path| path.to_ascii_lowercase().ends_with(".map"))
    );
    let binary = plan
        .entries
        .iter()
        .find(|entry| entry.path == "apps/api/api")
        .unwrap();
    assert_eq!(binary.mode, 0o755);
    let links: Vec<&str> = plan.links.iter().map(|link| link.path.as_str()).collect();
    assert_eq!(links, ["apps/api/uploads", "storage"]);
    let storage = plan
        .links
        .iter()
        .find(|link| link.path == "storage")
        .unwrap();
    assert_eq!(storage.target, "/opt/shop/shared/storage");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn two_different_files_for_one_path_are_refused() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "a/one", "1");
    write(&root, "b/two", "2");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  files:
    - { path: ./a/one, to: same }
    - { path: ./b/two, to: same }
apps:
  web:
    process: { start: node x, systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1", "a", false, chrono::DateTime::UNIX_EPOCH);
    let error = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap_err();
    assert!(format!("{error:#}").contains("two files"), "{error:#}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_pattern_that_matches_nothing_is_reported() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-miss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  files: [dist]
apps:
  web:
    process: { start: node x, systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1", "a", false, chrono::DateTime::UNIX_EPOCH);
    let plan = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap();
    assert_eq!(plan.missing.len(), 1);
    assert!(plan.missing[0].contains("dist"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn nothing_may_be_packed_into_shipdeck_or_a_shared_folder() {
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  files:
    - { path: ./x/.shipdeck, to: .shipdeck }
apps:
  web:
    process: { start: node x, systemd: {} }
";
    let problems = crate::config::parse_deploy(yaml).unwrap_err();
    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("belongs to shipdeck")),
        "{problems:?}"
    );
}

#[test]
fn a_binary_with_to_lands_in_the_app_folder() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-binary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "target/release/api", "binary");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  api:
    binary: { path: target/release/api, to: bin/api }
    process: { systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1", "a", false, chrono::DateTime::UNIX_EPOCH);
    let plan = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap();
    let binary = plan
        .entries
        .iter()
        .find(|entry| entry.path == "apps/api/bin/api")
        .unwrap_or_else(|| panic!("no binary: {:?}", plan.paths()));
    assert_eq!(binary.mode, 0o755);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_binary_folder_is_refused() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-bin-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("target/release/api")).unwrap();
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  api:
    binary: target/release/api
    process: { systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1", "a", false, chrono::DateTime::UNIX_EPOCH);
    let error = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap_err();
    assert!(format!("{error:#}").contains("is a folder"), "{error:#}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_binary_and_a_file_at_one_path_are_refused() {
    let root = std::env::temp_dir().join(format!("shipdeck-plan-bin-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "bin/api", "binary");
    write(&root, "other/api", "file");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  api:
    files: [{ path: ./other/api, to: api }]
    binary: { path: bin/api, to: api }
    process: { systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::new("1", "a", false, chrono::DateTime::UNIX_EPOCH);
    let error = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap_err();
    assert!(format!("{error:#}").contains("two files"), "{error:#}");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_pm2_file_gets_its_placeholders() {
    let root = std::env::temp_dir().join(format!("shipdeck-pm2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "app/main.js", "x");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    source: app
    files: [app/main.js]
    process:
      start: node main.js
      env: { DATA: '{{ROOT}}/data' }
      pm2:
        options:
          instances: 2
          log_dir: '{{APP_CURRENT}}/logs' 
";
    let deployment = deployment(yaml, &root);
    let release = ReleaseName::parse("v1.2.3-abc1234-20260101.120000").unwrap();
    let text = crate::bundle::pm2::render(&deployment, "web", &release).unwrap();
    assert!(
        text.contains("\"/opt/shop/current/apps/web/logs\""),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_inline_manager_ends_up_in_the_release() {
    let root = std::env::temp_dir().join(format!("shipdeck-unit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    process:
      start: node main.js
      env: { DATA: '{{ROOT}}/data' }
      systemd: { user: web }
";
    let deployment = deployment(yaml, &root);
    let release = ReleaseName::parse("v1.2.3-abc1234-20260101.120000").unwrap();
    let text = crate::bundle::unit::render(&deployment, "web", &release, None)
        .unwrap()
        .unwrap();
    assert!(
        text.contains("Environment=\"DATA=/opt/shop/data\""),
        "{text}"
    );
    assert!(text.contains("User=web"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_binary_names_its_full_path() {
    let root = std::env::temp_dir().join(format!("shipdeck-nobinary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "app/main.js", "x");
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    source: app
    files: [app/main.js]
    binary: target/release/api
    process: { systemd: {} }
";
    let deployment = deployment(yaml, &root);
    let name = ReleaseName::parse("v1.2.3-abc1234-20260101.120000").unwrap();
    let error = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains(&root.display().to_string()),
        "no full path: {text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
