use super::*;

const HASH: &str = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
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
