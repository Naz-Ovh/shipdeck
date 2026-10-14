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

#[test]
fn a_workspace_root_gives_its_workspace_version() {
    let dir = scratch("root");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"2.1.0\"\n",
    )
    .unwrap();
    assert_eq!(read_version(&dir.join("Cargo.toml")).unwrap(), "2.1.0");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reads_package_json_pyproject_and_plain_files() {
    let dir = scratch("kinds");
    std::fs::write(
        dir.join("package.json"),
        r#"{ "name": "web", "version": "1.4.0-beta.1" }"#,
    )
    .unwrap();
    assert_eq!(
        read_version(&dir.join("package.json")).unwrap(),
        "1.4.0-beta.1"
    );

    std::fs::write(
        dir.join("pyproject.toml"),
        "[project]\nname = \"api\"\nversion = \"0.9.0\"\n",
    )
    .unwrap();
    assert_eq!(read_version(&dir.join("pyproject.toml")).unwrap(), "0.9.0");

    std::fs::write(dir.join("VERSION"), "\n  3.0.1  \n").unwrap();
    assert_eq!(read_version(&dir.join("VERSION")).unwrap(), "3.0.1");

    std::fs::write(dir.join("package.json"), r#"{ "name": "web" }"#).unwrap();
    assert!(read_version(&dir.join("package.json")).is_err());
    std::fs::write(dir.join("VERSION"), "\n").unwrap();
    assert!(read_version(&dir.join("VERSION")).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
