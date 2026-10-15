use super::*;

/// A release after `npm ci`: what was shipped, plus what setup made.
fn release() -> Vec<ReleaseFile> {
    let dir = |path: &str| ReleaseFile {
        path: path.into(),
        is_dir: true,
    };
    let file = |path: &str| ReleaseFile {
        path: path.into(),
        is_dir: false,
    };
    vec![
        file("release.json"),
        file("my-api.service"),
        file(".env"),
        file("package.json"),
        dir("dist"),
        file("dist/main.js"),
        file("dist/main.js.map"),
        dir("node_modules"),
        dir("node_modules/express"),
        file("node_modules/express/index.js"),
        dir("data"),
        file("data/keys.json"),
        file("data/cache.bin"),
        file("uploads"),
    ]
}

fn deployed() -> BTreeSet<String> {
    [
        "release.json",
        "my-api.service",
        ".env",
        "package.json",
        "dist",
        "dist/main.js",
        "dist/main.js.map",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn own() -> Vec<String> {
    vec!["release.json".into(), "my-api.service".into()]
}

fn picked(files: BackupFiles) -> Picked {
    pick(&release(), &deployed(), &own(), &["uploads".into()], &files).unwrap()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn takes_what_was_deployed_and_leaves_what_setup_made() {
    let picked = picked(BackupFiles::default());
    assert_eq!(
        picked.paths,
        [
            ".env",
            "dist",
            "dist/main.js",
            "dist/main.js.map",
            "my-api.service",
            "package.json",
            "release.json",
        ]
    );
    assert!(picked.unmatched.is_empty());
}

#[test]
fn include_brings_in_a_generated_folder_and_exclude_leaves_out() {
    let picked = picked(BackupFiles {
        deployed: true,
        include: strings(&["data"]),
        exclude: strings(&["**/*.map", "data/cache.bin"]),
    });
    assert!(picked.paths.contains(&"data/keys.json".to_string()));
    assert!(picked.paths.contains(&"data".to_string()));
    assert!(!picked.paths.contains(&"data/cache.bin".to_string()));
    assert!(!picked.paths.contains(&"dist/main.js.map".to_string()));
    assert!(!picked.paths.iter().any(|p| p.starts_with("node_modules")));
}

#[test]
fn shipdecks_own_files_stay_and_shared_folders_never_go_in() {
    let picked = picked(BackupFiles {
        deployed: false,
        include: strings(&["."]),
        exclude: strings(&["*.service", "release.json"]),
    });
    assert!(picked.paths.contains(&"release.json".to_string()));
    assert!(picked.paths.contains(&"my-api.service".to_string()));
    assert!(
        picked
            .paths
            .contains(&"node_modules/express/index.js".to_string())
    );
    assert!(!picked.paths.contains(&"uploads".to_string()));
}

#[test]
fn not_deployed_leaves_only_what_include_names() {
    let picked = picked(BackupFiles {
        deployed: false,
        include: strings(&["data/keys.json", "missing/**"]),
        exclude: Vec::new(),
    });
    assert_eq!(
        picked.paths,
        ["data/keys.json", "my-api.service", "release.json"]
    );
    assert_eq!(picked.unmatched, ["missing/**"]);
}

#[test]
fn reads_find_records() {
    let text = "d.\0d./dist\0f./dist/a b.js\0f./.env\0junk\0";
    assert_eq!(
        parse_files(text),
        [
            ReleaseFile {
                path: "dist".into(),
                is_dir: true
            },
            ReleaseFile {
                path: "dist/a b.js".into(),
                is_dir: false
            },
            ReleaseFile {
                path: ".env".into(),
                is_dir: false
            },
        ]
    );
}
