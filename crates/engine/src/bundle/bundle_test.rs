use std::collections::BTreeMap;
use std::io::Read;

use chrono::{TimeZone, Utc};
use flate2::read::GzDecoder;

use super::*;
use crate::config::{DeployConfig, Deployable};
use crate::release::{Manifest, ReleaseName};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: PathBuf, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A deployable in `folder/deploy` whose project is `folder/app`.
fn deployable(folder: &Path, service: &str, pack: &str) -> Deployable {
    let yaml = format!(
        "
name: Demo
project: ../app
ssh: {{ host: example.com, user: deploy }}
service: {{ name: demo, dir: /opt/demo, {service} }}
pipeline:
  pack:
{pack}
"
    );
    let config: DeployConfig = crate::config::parse(&yaml).unwrap();
    let endpoint = crate::ssh::Endpoint::from_config(
        &config.ssh,
        crate::ssh::ssh_config::HostConfig::default(),
    )
    .unwrap();
    let deploy = folder.join("deploy");
    std::fs::create_dir_all(&deploy).unwrap();
    std::fs::create_dir_all(folder.join("app")).unwrap();
    std::fs::write(deploy.join("deploy.yaml"), &yaml).unwrap();
    Deployable {
        folder: deploy,
        slug: "demo".into(),
        config,
        endpoint,
    }
}

fn release() -> ReleaseName {
    ReleaseName::new(
        "0.3.1",
        "ab12cd3",
        false,
        Utc.with_ymd_and_hms(2026, 10, 14, 15, 30, 12).unwrap(),
    )
}

/// Each archive entry: mode, body, and link target for links.
type Entries = BTreeMap<String, (u32, Vec<u8>, Option<String>)>;

fn unpack(data: &[u8]) -> Entries {
    let mut archive = tar::Archive::new(GzDecoder::new(data));
    let mut files = BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mode = entry.header().mode().unwrap();
        let link = entry
            .link_name()
            .unwrap()
            .map(|t| t.to_string_lossy().into_owned());
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        files.insert(path, (mode, body, link));
    }
    files
}

fn packed(deployable: &Deployable) -> (Bundle, Entries) {
    let bundle = pack(
        deployable,
        &Manifest::new(&release(), "tester"),
        "[Unit]\n",
        |_, _| {},
    )
    .unwrap();
    let files = unpack(&bundle.data);
    (bundle, files)
}

fn pack_error(deployable: &Deployable) -> String {
    let err = pack(deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).unwrap_err();
    format!("{err:#}")
}

#[test]
fn packs_binary_files_manifest_and_unit_with_modes() {
    let dir = scratch("pack");
    write(dir.join("app/out/demo"), "\x7fELF fake");
    write(dir.join("app/files/sub/a.txt"), "a");
    write(dir.join("deploy/.env"), "SECRET=1");
    let deployable = deployable(
        &dir,
        "",
        "    binary: out/demo\n    files:\n      include:\n        project: [./files/]\n        deploy: [\".env\"]",
    );
    let manifest = Manifest::new(&release(), "tester");
    let mut last = (0, 0);
    let bundle = pack(&deployable, &manifest, "[Unit]\n", |done, total| {
        last = (done, total)
    })
    .unwrap();

    let files = unpack(&bundle.data);
    assert_eq!(files["demo"].0, 0o755);
    assert_eq!(files[".env"].0, 0o600);
    assert_eq!(files["files/sub/a.txt"].1, b"a");
    assert!(files.contains_key("files/sub"));
    assert_eq!(files["demo.service"].1, b"[Unit]\n");
    let written: Manifest = serde_json::from_slice(&files["release.json"].1).unwrap();
    assert_eq!(written, manifest);
    // deploy.yaml never ships, even next to .env.
    assert!(!files.contains_key("deploy.yaml"));

    assert_eq!(bundle.files, 5);
    assert_eq!(last.0, last.1);
    assert_eq!(last.1, bundle.input_bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ships_the_whole_project_minus_excludes_and_never_git() {
    let dir = scratch("whole");
    for path in [
        "app/main.py",
        "app/pkg/mod.py",
        "app/pkg/__pycache__/mod.pyc",
        "app/__pycache__/main.pyc",
        "app/tests/test_main.py",
        "app/.venv/bin/python",
        "app/.git/HEAD",
        "app/pkg/.git",
        "app/dist/a.map",
        "app/dist/keep.map",
        "app/src/a.test.js",
        "app/src/a.js",
        "deploy/.env",
        "deploy/service.template",
    ] {
        write(dir.join(path), "x");
    }
    let deployable = deployable(
        &dir,
        "start: python3 main.py",
        "    files:
      include:
        project: [., \"!src/**/*.test.js\"]
        deploy: [.]
      exclude:
        project: [tests, \"**/__pycache__\", .venv, \"**/*.map\", \"!dist/keep.map\"]",
    );
    let (_, files) = packed(&deployable);
    let shipped: Vec<&str> = files
        .keys()
        .map(String::as_str)
        .filter(|p| !p.ends_with(".service") && *p != "release.json")
        .collect();
    assert_eq!(
        shipped,
        [
            ".env",
            "dist",
            "dist/keep.map",
            "main.py",
            "pkg",
            "pkg/mod.py",
            "src",
            "src/a.js"
        ]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn globs_pick_files_and_a_deploy_file_replaces_the_project_one() {
    let dir = scratch("globs");
    write(dir.join("app/dist/main.js"), "main");
    write(dir.join("app/dist/vendor/lib.js"), "lib");
    write(dir.join("app/dist/style.css"), "css");
    write(dir.join("app/config.json"), "dev");
    write(dir.join("deploy/config.json"), "prod");
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:
      include:
        project: [\"dist/**/*.{js,json}\", config.json]
        deploy: [config.json]",
    );
    let (bundle, files) = packed(&deployable);
    assert!(files.contains_key("dist/main.js"));
    assert!(files.contains_key("dist/vendor/lib.js"));
    assert!(!files.contains_key("dist/style.css"));
    assert_eq!(files["config.json"].1, b"prod");
    assert_eq!(bundle.replaced, ["config.json"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn follows_a_named_link_and_keeps_inner_links() {
    let dir = scratch("links");
    write(dir.join("app/out/main.js"), "console.log(1)");
    // `dist` itself is a link: it is followed.
    std::os::unix::fs::symlink("out", dir.join("app/dist")).unwrap();
    write(dir.join("app/node_modules/tool/cli.js"), "#!");
    std::fs::create_dir_all(dir.join("app/node_modules/.bin")).unwrap();
    // A link inside a shipped folder stays a link.
    std::os::unix::fs::symlink("../tool/cli.js", dir.join("app/node_modules/.bin/tool")).unwrap();
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:\n      include:\n        project: [dist, node_modules]",
    );
    let (_, files) = packed(&deployable);
    assert!(files.contains_key("dist/main.js"), "{:?}", files.keys());
    assert_eq!(files["dist"].2, None);
    assert_eq!(
        files["node_modules/.bin/tool"].2.as_deref(),
        Some("../tool/cli.js")
    );
    assert!(!files.keys().any(|p| p.starts_with("out")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn names_what_the_build_did_not_produce() {
    let dir = scratch("missing");
    write(dir.join("app/package.json"), "{}");
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:\n      include:\n        project: [dist, package.json, \"build/*.js\"]\n        deploy: [.env]",
    );
    let err = pack_error(&deployable);
    assert!(err.contains("dist (not found)"), "{err}");
    assert!(err.contains("build/*.js (matched nothing)"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_clashes_with_the_release_and_shared_folders() {
    let dir = scratch("clash");
    write(dir.join("app/out/demo"), "bin");
    write(dir.join("app/demo"), "a file named like the binary");
    write(dir.join("app/uploads/a.png"), "png");
    let binary = deployable(
        &dir,
        "",
        "    binary: out/demo\n    files:\n      include:\n        project: [\"dem?\"]",
    );
    assert!(pack_error(&binary).contains("demo would be in the release twice"));

    let shared = deployable(
        &dir,
        "start: ./out/demo, shared: [uploads]",
        "    files:\n      include:\n        project: [\"**/*.png\"]",
    );
    let err = pack_error(&shared);
    assert!(
        err.contains("uploads/a.png is packed, but service.shared"),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn renders_the_default_unit() {
    let dir = scratch("unit");
    let deployable = deployable(
        &dir,
        "start: ./demo -E=.env",
        "    binary: target/release/demo",
    );
    let unit = unit::render_for(&deployable, &release(), None).unwrap();
    assert!(unit.contains("Description=Demo v0.3.1 (ab12cd3)"));
    assert!(unit.contains("ExecStart=/opt/demo/current/demo -E=.env"));
    assert!(unit.contains("WorkingDirectory=/opt/demo/current"));
    assert!(unit.contains("Environment=APP_RELEASE=v0.3.1-ab12cd3-20261014.153012"));
    assert!(unit.contains("User=deploy"));
    assert!(unit.contains("ReadWritePaths=/opt/demo/shared"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_placeholder_is_an_error() {
    let err = unit::render("Hello {{nope}}", &[("name", Some("x".into()))]).unwrap_err();
    assert!(err.to_string().contains("nope"));
    assert_eq!(
        unit::render("A={{ name }}!", &[("name", Some("x".into()))]).unwrap(),
        "A=x!"
    );
    let err = unit::render("{{binary}}", &[("binary", None)]).unwrap_err();
    assert!(err.to_string().contains("no pipeline.pack.binary"), "{err}");
}

#[test]
fn exec_start_uses_where_the_server_has_the_program() {
    let dir = scratch("exec");
    let pack = "    files:\n      include:\n        project: [dist]";
    let node = deployable(&dir, "start: node dist/main.js", pack);
    assert_eq!(
        unit::exec_start(&node, Some("/usr/bin/node")),
        "/usr/bin/node dist/main.js"
    );
    // Before preflight the name is all there is.
    assert_eq!(unit::exec_start(&node, None), "node dist/main.js");
    let unit = unit::render_for(&node, &release(), Some("/usr/local/bin/node")).unwrap();
    assert!(!unit.contains("EnvironmentFile"));
    assert!(
        unit.contains("ExecStart=/usr/local/bin/node dist/main.js\n"),
        "{unit}"
    );

    let with_file = deployable(
        &dir,
        "start: node dist/main.js, env_file: ./.env, env: { NODE_ENV: production }",
        pack,
    );
    let with_file = unit::render_for(&with_file, &release(), None).unwrap();
    assert!(
        with_file.contains(
            "EnvironmentFile=/opt/demo/current/.env\nEnvironment=\"NODE_ENV=production\"\n"
        ),
        "{with_file}"
    );

    let python = deployable(&dir, "start: .venv/bin/python -m app", pack);
    assert_eq!(
        unit::exec_start(&python, None),
        "/opt/demo/current/.venv/bin/python -m app"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
