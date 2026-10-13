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
    let config: DeployConfig = serde_json::from_value(serde_json::json!({
        "name": "Demo",
        "ssh": { "host": "example.com", "user": "deploy" },
        "build": { "command": ["true"], "binary": "out/demo" },
        "remote": { "dir": "/opt/demo", "service": "demo", "args": ["-E=.env"] },
        "bundle": bundle,
    }))
    .unwrap();
    Deployable {
        folder: folder.to_path_buf(),
        slug: "demo".into(),
        config,
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

    let deployable = deployable(&dir, &[".env", "./files/"]);
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
    let deployable = deployable(&dir, &[".env"]);
    let err = pack(&deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).unwrap_err();
    assert!(format!("{err:#}").contains(".env"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bundle_entry_cannot_replace_release_files() {
    let dir = scratch("reserved");
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out/demo"), b"bin").unwrap();
    std::fs::write(dir.join("release.json"), b"{}").unwrap();
    let deployable = deployable(&dir, &["release.json"]);
    assert!(pack(&deployable, &Manifest::new(&release(), "t"), "", |_, _| {}).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn renders_the_default_unit() {
    let dir = scratch("unit");
    let deployable = deployable(&dir, &[]);
    let unit = unit::render_for(&deployable, &release()).unwrap();
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
    let err = unit::render("Hello {{nope}}", &[("name", "x".into())]).unwrap_err();
    assert!(err.to_string().contains("nope"));
    assert_eq!(
        unit::render("A={{ name }}!", &[("name", "x".into())]).unwrap(),
        "A=x!"
    );
}
