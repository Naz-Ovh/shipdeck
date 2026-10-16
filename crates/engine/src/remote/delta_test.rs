use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;
use crate::bundle::{self, Bundle};
use crate::config::{Deployable, parse};
use crate::release::{Manifest, ReleaseName};
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

const HASH: &str = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn reads_hashes_only_for_the_paths_sent() {
    let sent = strings(&["dist/main.js", "a b.txt ", "bin/tool", "x"]);
    let other = "f".repeat(64);
    let stdout = format!(
        "{HASH}  ./dist/main.js\n\
         {HASH}  ./a b.txt \n\
         {other} *./bin/tool\n\
         {HASH}  ./not-sent.js\n\
         {}  ./x\n\
         sha256sum: ./gone: No such file or directory\n\
         {HASH}  dist/main.js\n",
        HASH.to_uppercase()
    );
    let hashes = parse_hashes(&stdout, &sent);
    assert_eq!(hashes.len(), 3, "{hashes:?}");
    assert_eq!(hashes["dist/main.js"], HASH);
    assert_eq!(hashes["a b.txt "], HASH);
    assert_eq!(hashes["bin/tool"], other);
}

#[test]
fn the_hash_script_skips_links_and_removes_its_list() {
    let script = hash_script("/opt/my-api", "v1.0.0-abc1234-20261014.120000");
    assert!(
        script.starts_with("cd '/opt/my-api/current' || exit 1\n"),
        "{script}"
    );
    assert!(
        script.contains(r#"if [ -f "$f" ] && [ ! -L "$f" ]; then set -- "$@" "$f"; fi"#),
        "{script}"
    );
    assert!(script.contains(r#"sha256sum -- "$@""#), "{script}");
    assert!(script.contains("rm -f '/opt/my-api/releases/.v1.0.0-abc1234-20261014.120000.paths'"));
}

#[test]
fn the_unpack_script_copies_each_mode_then_checks_the_receipt() {
    let id = "v1.0.0-abc1234-20261014.120000";
    let script = unpack_script("/opt/my-api", id, &[0o644, 0o755]);
    for mode in ["0644", "0755"] {
        assert!(
            script.contains(&format!(
                "(cd '/opt/my-api/current' && tar -cf - --null --no-recursion -T \
                 '/opt/my-api/releases/.{id}.reuse.{mode}') | (cd .{id}.partial && tar -xpf -) || ok=no"
            )),
            "{script}"
        );
        assert!(script.contains(&format!("chmod {mode} \"$@\"")), "{script}");
    }
    assert!(script.contains(&format!(
        "(cd .{id}.partial && sha256sum -c '/opt/my-api/releases/.{id}.sha256') > .{id}.checked || ok=no"
    )));
    // The release is moved into place only after the check.
    assert!(
        script.ends_with(&format!("mv .{id}.partial {id}")),
        "{script}"
    );

    let whole = unpack_script("/opt/my-api", id, &[]);
    assert!(!whole.contains("tar -cf"), "{whole}");
    assert!(whole.contains("sha256sum -c"), "{whole}");
}

#[test]
fn discard_removes_what_was_sent_to_unpack() {
    let script = super::super::discard_script("/opt/my-api", "v1");
    for name in [
        "releases/.v1.tar.zst",
        "releases/.v1.paths",
        "releases/.v1.sha256",
        "releases/.v1.checked",
        "releases/.v1.reuse.*",
    ] {
        assert!(script.contains(name), "{name}: {script}");
    }
}

#[test]
fn mismatches_are_named_once_without_their_dot() {
    let stdout = "mismatch=./a.txt\nmismatch=./a.txt\nmismatch=./dir/b: c\nverified=no\n";
    assert_eq!(mismatches(&pairs(stdout)), ["a.txt", "dir/b: c"]);
}

// ── the scripts, run with this machine's sh, tar, zstd and sha256sum ──────

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-delta-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, body: &str, mode: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// Runs `script` with `sh -c`: its exit code and stdout.
fn sh(script: &str) -> (i32, String) {
    let out = Command::new("sh").arg("-c").arg(script).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// The scripts need GNU tar, zstd and sha256sum, as the server does.
fn has_server_tools() -> bool {
    let tar = Command::new("tar").arg("--version").output();
    let gnu = tar.is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("GNU tar"));
    gnu && ["zstd", "sha256sum"]
        .iter()
        .all(|tool| sh(&format!("command -v {tool}")).0 == 0)
}

const ID: &str = "v1.1.0-def5678-20261015.120000";

/// A server folder whose live release has `same.txt`, `old.js` and a
/// link, and a project whose next release keeps `same.txt` (now
/// executable) and changes `old.js`.
fn server_and_project(root: &Path) -> (PathBuf, Deployable) {
    let server = root.join("srv");
    let live = server.join("releases/v1.0.0-abc1234-20261014.120000");
    write(&live.join("same.txt"), "same", 0o644);
    write(&live.join("lib/old.js"), "old", 0o644);
    std::os::unix::fs::symlink("old.js", live.join("lib/link.js")).unwrap();
    std::os::unix::fs::symlink(
        "releases/v1.0.0-abc1234-20261014.120000",
        server.join("current"),
    )
    .unwrap();
    // An old mtime, to see the copy keeps it.
    let touched = sh(&format!(
        "touch -d '2026-01-02 03:04:05' {}",
        sh_quote(&live.join("same.txt").display().to_string())
    ));
    assert_eq!(touched.0, 0);

    let project = root.join("app");
    write(&project.join("same.txt"), "same", 0o755);
    write(&project.join("lib/old.js"), "new", 0o644);
    std::os::unix::fs::symlink("old.js", project.join("lib/link.js")).unwrap();
    let yaml = "
name: Demo
project: ../app
ssh: { host: 203.0.113.10, user: deploy }
service: { name: demo, dir: /opt/demo, start: ./same.txt }
pipeline:
  pack:
    files:
      include:
        project: [same.txt, lib]
";
    let config = parse(yaml).unwrap();
    let deploy = root.join("deploy");
    std::fs::create_dir_all(&deploy).unwrap();
    let deployable = Deployable {
        folder: deploy,
        slug: "demo".into(),
        endpoint: Endpoint::from_config(&config.ssh, HostConfig::default()).unwrap(),
        config,
        yaml: yaml.into(),
    };
    (server, deployable)
}

/// Asks the server's `current` for hashes, as `current_hashes` does, and
/// packs the release against them.
fn hash_and_pack(server: &Path, deployable: &Deployable) -> Bundle {
    let dir = server.display().to_string();
    let contents = bundle::gather(deployable, None, |_, _| {}).unwrap();
    let candidates = contents.candidates();
    std::fs::write(
        server.join(format!("releases/.{ID}.paths")),
        nul_list(&candidates),
    )
    .unwrap();
    let (code, stdout) = sh(&hash_script(&dir, ID));
    assert_eq!(code, 0, "{stdout}");
    assert!(!server.join(format!("releases/.{ID}.paths")).exists());
    let on_server = parse_hashes(&stdout, &candidates);
    let name = ReleaseName::new("1.1.0", "def5678", false, chrono::Utc::now());
    bundle::pack(&contents, &Manifest::new(&name, "t"), &on_server, |_, _| {}).unwrap()
}

/// Puts what the upload step sends where unpack looks for it.
fn upload(server: &Path, bundle: &Bundle) {
    let releases = server.join("releases");
    std::fs::write(releases.join(format!(".{ID}.tar.zst")), &bundle.data).unwrap();
    std::fs::write(releases.join(format!(".{ID}.sha256")), &bundle.receipt).unwrap();
    for (mode, list) in &bundle.reuse {
        std::fs::write(releases.join(format!(".{ID}.reuse.{mode:04o}")), list).unwrap();
    }
}

fn leftovers(server: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(server.join("releases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[test]
fn a_delta_unpacks_with_the_rest_copied_from_current_and_checked() {
    if !has_server_tools() {
        eprintln!("skipped: needs GNU tar, zstd and sha256sum");
        return;
    }
    let root = scratch("unpack");
    let (server, deployable) = server_and_project(&root);
    let bundle = hash_and_pack(&server, &deployable);
    // same.txt is reused; old.js changed; the link always goes.
    assert_eq!((bundle.reused, bundle.sent()), (1, 2));
    upload(&server, &bundle);

    let (code, stdout) = sh(&unpack_script(&server.display().to_string(), ID, &[0o755]));
    assert_eq!(code, 0, "{stdout}");
    let release = server.join("releases").join(ID);
    assert_eq!(
        std::fs::read_to_string(release.join("same.txt")).unwrap(),
        "same"
    );
    assert_eq!(
        std::fs::read_to_string(release.join("lib/old.js")).unwrap(),
        "new"
    );
    let same = std::fs::symlink_metadata(release.join("same.txt")).unwrap();
    assert!(same.is_file(), "copied, not linked");
    assert_eq!(same.mode() & 0o777, 0o755, "the new release's mode");
    let live = std::fs::metadata(server.join("current/same.txt")).unwrap();
    assert_eq!(same.mtime(), live.mtime(), "the copy keeps the mtime");
    assert_ne!(same.ino(), live.ino(), "never a hard link");
    assert_eq!(live.mode() & 0o777, 0o644, "the live release is untouched");
    assert_eq!(
        std::fs::read_link(release.join("lib/link.js")).unwrap(),
        PathBuf::from("old.js")
    );
    assert!(release.join("release.json").is_file());
    assert!(leftovers(&server).is_empty(), "{:?}", leftovers(&server));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_file_that_changed_in_current_fails_the_check_and_leaves_nothing() {
    if !has_server_tools() {
        eprintln!("skipped: needs GNU tar, zstd and sha256sum");
        return;
    }
    let root = scratch("mismatch");
    let (server, deployable) = server_and_project(&root);
    let bundle = hash_and_pack(&server, &deployable);
    upload(&server, &bundle);
    // Someone edits the live release between the hashing and the copy.
    std::fs::write(server.join("current/same.txt"), "edited").unwrap();

    let (code, stdout) = sh(&unpack_script(&server.display().to_string(), ID, &[0o755]));
    assert_ne!(code, 0);
    let values = pairs(&stdout);
    assert!(values.contains(&("verified", "no")), "{stdout}");
    assert_eq!(mismatches(&values), ["same.txt"]);
    assert!(!server.join("releases").join(ID).exists());
    assert!(leftovers(&server).is_empty(), "{:?}", leftovers(&server));

    // Sent whole, the same release unpacks and checks out.
    let contents = bundle::gather(&deployable, None, |_, _| {}).unwrap();
    let name = ReleaseName::new("1.1.0", "def5678", false, chrono::Utc::now());
    let whole = bundle::pack(
        &contents,
        &Manifest::new(&name, "t"),
        &Default::default(),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(whole.reused, 0);
    upload(&server, &whole);
    let (code, stdout) = sh(&unpack_script(&server.display().to_string(), ID, &[]));
    assert_eq!(code, 0, "{stdout}");
    let release = server.join("releases").join(ID);
    assert_eq!(
        std::fs::read_to_string(release.join("same.txt")).unwrap(),
        "same"
    );
    assert!(leftovers(&server).is_empty(), "{:?}", leftovers(&server));
    std::fs::remove_dir_all(root).unwrap();
}
