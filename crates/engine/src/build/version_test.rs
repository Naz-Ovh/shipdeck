use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-version-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reads_a_plain_package_version() {
    let dir = scratch("plain");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"a\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    assert_eq!(cargo_version(&dir.join("Cargo.toml")).unwrap(), "1.2.3");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn follows_workspace_inheritance() {
    let dir = scratch("workspace");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"bin/app\"]\n\n[workspace.package]\nversion = \"0.4.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("bin/app")).unwrap();
    let manifest = dir.join("bin/app/Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"app\"\nversion.workspace = true\n",
    )
    .unwrap();
    assert_eq!(cargo_version(&manifest).unwrap(), "0.4.0");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_version_is_an_error() {
    let dir = scratch("missing");
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
    assert!(cargo_version(&dir.join("Cargo.toml")).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
