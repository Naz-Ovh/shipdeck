use super::{download_body, install_script, upload_body};

#[test]
fn the_installer_runs_as_root_inside_one_lock() {
    let script = install_script(&download_body("x86_64-unknown-linux-gnu"), "5.2.1");
    let root = script.find("id -u").expect("root check");
    let lock = script.find("flock -x 9").expect("lock");
    let download = script.find("curl -fsSL").expect("download");
    assert!(root < lock && lock < download);
    assert!(script.contains("umask 022"));
    assert!(script.contains("trap cleanup EXIT"));
    assert!(script.contains(".install.lock.d"));
    assert!(script.contains("holds_dir_lock=yes"));
    assert!(script.contains("secure=no"));
    assert!(script.contains("/opt must not be writable"));
}

#[test]
fn the_installer_verifies_before_publishing() {
    let script = install_script(&download_body("x86_64-unknown-linux-gnu"), "5.2.1");
    let checksum = script
        .find("does not match its checksum")
        .expect("checksum");
    let version = script
        .find("does not report version")
        .expect("version check");
    let rename = script
        .find("mv \"$stage/publish\" \"$target\"")
        .expect("rename");
    assert!(checksum < version && version < rename);
    assert!(script.contains("v5.2.1"));
    assert!(script.contains("sha256sum"));
    assert!(script.contains("tar -xzf"));
}

#[test]
fn the_installer_replaces_and_cleans_versioned_directories() {
    let script = install_script(&download_body("x86_64-unknown-linux-gnu"), "5.2.1");
    assert!(script.contains("rm -rf -- \"$target\""));
    assert!(script.contains("for old in \"$root\"/v*"));
    assert!(script.contains("\"$root\"/v*) rm -rf -- \"$old\""));
    assert!(script.contains("root='/opt/shipdeck/agent'"));
}

#[test]
fn the_archive_paths_and_links_are_checked() {
    let script = download_body("x86_64-unknown-linux-gnu");
    assert!(script.contains("tar -tzf"));
    assert!(script.contains(r"/^\// || /(^|\/)\.\.(\/|$)/"));
    assert!(script.contains("tar -tvzf"));
    assert!(script.contains("holds a link or special file"));
    assert!(script.contains("find \"$stage/unpack\" -type l"));
}

#[test]
fn the_release_asset_and_its_checksum_are_fixed() {
    let script = download_body("x86_64-unknown-linux-gnu");
    assert!(script.contains("https://github.com/Naz-Ovh/shipdeck/releases/download/v"));
    assert!(script.contains("shipdeck-agent-x86_64-unknown-linux-gnu.tar.gz"));
    assert!(script.contains("shipdeck-agent-x86_64-unknown-linux-gnu.tar.gz.sha256"));
}

#[test]
fn a_development_binary_is_copied_before_it_is_checked_or_run() {
    let script = install_script(
        &upload_body("/tmp/private/agent.partial", "abc123"),
        "5.2.1",
    );
    let copy = script
        .find("install -m 0755 -o root -g root")
        .expect("copy");
    let hash = script
        .find("sha256sum \"$stage/publish/agent\"")
        .expect("hash");
    let run = script.rfind("--version").expect("version probe");
    assert!(copy < hash && hash < run);
    assert!(!script.contains("\"/tmp/private/agent.partial\" --version"));
    assert!(script.contains("checksum did not match"));
}
