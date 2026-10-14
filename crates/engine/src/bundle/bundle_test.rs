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

fn deployable(folder: &Path, bundle: &[&str]) -> Deployable {
    let yaml = format!(
        "
name: Demo
ssh: {{ host: example.com, user: deploy }}
build: {{ command: [\"true\"], binary: out/demo }}
remote: {{ dir: /opt/demo, service: demo, args: [-E=.env] }}
bundle: [{}]
",
        bundle.join(", ")
    );
    deployable_from(folder, &yaml)
}

fn deployable_from(folder: &Path, yaml: &str) -> Deployable {
    let config: DeployConfig = crate::config::parse(yaml).unwrap();
    let endpoint = crate::ssh::Endpoint::from_config(
        &config.ssh,
        crate::ssh::ssh_config::HostConfig::default(),
    )
    .unwrap();
    Deployable {
        folder: folder.to_path_buf(),
        slug: "demo".into(),
        config,
        endpoint,
    }
}

const NODE: &str = "
name: Web
ssh: { host: example.com, user: deploy }
build:
  cwd: app
  command: npm run build
  files: [dist, package.json, node_modules]
remote: { dir: /opt/web, service: web, start: node dist/main.js }
bundle: [.env]
";

fn release() -> ReleaseName {
    ReleaseName::new(
        "0.3.1",
        "ab12cd3",
        false,
        Utc.with_ymd_and_hms(2026, 10, 14, 15, 30, 12).unwrap(),
    )
}

fn unpack(data: &[u8]) -> BTreeMap<String, (u32, Vec<u8>)> {
    let mut archive = tar::Archive::new(GzDecoder::new(data));
    let mut files = BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mode = entry.header().mode().unwrap();
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        files.insert(path, (mode, body));
    }
    files
}

#[test]
fn packs_binary_bundle_manifest_and_unit_with_modes() {
    let dir = scratch("pack");
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out/demo"), b"\x7fELF fake").unwrap();
    std::fs::write(dir.join(".env"), b"SECRET=1").unwrap();
    std::fs::create_dir_all(dir.join("files/sub")).unwrap();
    std::fs::write(dir.join("files/sub/a.txt"), b"a").unwrap();

    let deployable = deployable(&dir, &["\".env\"", "./files/"]);
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

    assert_eq!(bundle.files, 5);
    assert_eq!(last.0, last.1);
    assert_eq!(last.1, bundle.input_bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_bundle_entry_is_an_error() {
    let dir = scratch("missing");
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out/demo"), b"bin").unwrap();
    let deployable = deployable(&dir, &["\".env\""]);
    let err = pack(&deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).unwrap_err();
    assert!(format!("{err:#}").contains(".env: missing"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn renders_the_default_unit() {
    let dir = scratch("unit");
    let deployable = deployable(&dir, &[]);
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
    assert!(err.to_string().contains("no build.binary"), "{err}");
}

#[test]
fn packs_build_files_from_the_build_folder_and_keeps_inner_links() {
    let dir = scratch("node");
    std::fs::create_dir_all(dir.join("app/out")).unwrap();
    std::fs::write(dir.join("app/out/main.js"), b"console.log(1)").unwrap();
    // `dist` itself is a link: it is followed.
    std::os::unix::fs::symlink("out", dir.join("app/dist")).unwrap();
    std::fs::write(dir.join("app/package.json"), b"{}").unwrap();
    std::fs::create_dir_all(dir.join("app/node_modules/.bin")).unwrap();
    std::fs::create_dir_all(dir.join("app/node_modules/tool")).unwrap();
    std::fs::write(dir.join("app/node_modules/tool/cli.js"), b"#!").unwrap();
    // A link inside a shipped folder stays a link.
    std::os::unix::fs::symlink("../tool/cli.js", dir.join("app/node_modules/.bin/tool")).unwrap();
    std::fs::write(dir.join(".env"), b"A=1").unwrap();

    let deployable = deployable_from(&dir, NODE);
    let bundle = pack(&deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).unwrap();

    let mut archive = tar::Archive::new(GzDecoder::new(bundle.data.as_slice()));
    let mut links = BTreeMap::new();
    let mut paths = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        if entry.header().entry_type().is_symlink() {
            let target = entry.link_name().unwrap().unwrap();
            links.insert(path.clone(), target.to_string_lossy().into_owned());
        }
        paths.push(path);
    }
    assert!(paths.contains(&"dist/main.js".to_string()), "{paths:?}");
    assert!(paths.contains(&"package.json".to_string()));
    assert!(paths.contains(&".env".to_string()));
    assert!(!paths.iter().any(|p| p == "demo" || p.starts_with("out")));
    assert_eq!(links["node_modules/.bin/tool"], "../tool/cli.js");
    assert!(!links.contains_key("dist"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_build_files_are_named() {
    let dir = scratch("node-missing");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join(".env"), b"A=1").unwrap();
    let deployable = deployable_from(&dir, NODE);
    let err = pack(&deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).unwrap_err();
    let err = format!("{err:#}");
    assert!(err.contains("3 of 3 build.files entries"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exec_start_uses_where_the_server_has_the_program() {
    let dir = scratch("exec");
    let node = deployable_from(&dir, NODE);
    assert_eq!(
        unit::exec_start(&node, Some("/usr/bin/node")),
        "/usr/bin/node dist/main.js"
    );
    // Before preflight the name is all there is.
    assert_eq!(unit::exec_start(&node, None), "node dist/main.js");
    let unit = unit::render_for(&node, &release(), Some("/usr/local/bin/node")).unwrap();
    assert!(!unit.contains("EnvironmentFile"));
    let with_file = deployable_from(
        &dir,
        &NODE.replace(
            "start: node dist/main.js",
            "start: node dist/main.js, env_file: ./.env",
        ),
    );
    let with_file = unit::render_for(&with_file, &release(), None).unwrap();
    assert!(
        with_file.contains("EnvironmentFile=/opt/web/current/.env\n"),
        "{with_file}"
    );
    assert!(
        unit.contains("ExecStart=/usr/local/bin/node dist/main.js\n"),
        "{unit}"
    );

    let python = deployable_from(
        &dir,
        &NODE.replace("node dist/main.js", ".venv/bin/python -m app"),
    );
    assert_eq!(
        unit::exec_start(&python, None),
        "/opt/web/current/.venv/bin/python -m app"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
