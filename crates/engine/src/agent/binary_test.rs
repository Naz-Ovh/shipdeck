use super::{
    Arch, Libc, arch, asset_name, asset_url_with, field, install_script_with, libc_from,
    parse_glibc_version, target,
};

#[test]
fn reads_the_glibc_version_from_getconf() {
    assert_eq!(parse_glibc_version("glibc 2.31\n"), Some((2, 31)));
    assert_eq!(parse_glibc_version("glibc 2.17"), Some((2, 17)));
}

#[test]
fn reads_the_glibc_version_from_ldd() {
    assert_eq!(
        parse_glibc_version("ldd (Ubuntu GLIBC 2.31-0ubuntu9.9) 2.31"),
        Some((2, 31))
    );
}

#[test]
fn ignores_text_without_a_version() {
    assert_eq!(parse_glibc_version("musl libc"), None);
    assert_eq!(parse_glibc_version(""), None);
}

#[test]
fn compares_versions_as_numbers() {
    assert!(parse_glibc_version("glibc 2.9") < Some((2, 31)));
    assert!(parse_glibc_version("glibc 2.31") >= Some((2, 31)));
}

#[test]
fn ubuntu_twenty_and_newer_get_gnu() {
    assert_eq!(
        target(Arch::X86_64, Libc::Glibc(2, 31)),
        "x86_64-unknown-linux-gnu"
    );
    assert_eq!(
        target(Arch::Aarch64, Libc::Glibc(2, 39)),
        "aarch64-unknown-linux-gnu"
    );
}

#[test]
fn older_glibc_and_musl_get_musl() {
    assert_eq!(
        target(Arch::X86_64, Libc::Glibc(2, 28)),
        "x86_64-unknown-linux-musl"
    );
    assert_eq!(
        target(Arch::Aarch64, Libc::Musl),
        "aarch64-unknown-linux-musl"
    );
}

#[test]
fn maps_the_machine_names() {
    assert_eq!(arch("x86_64\n").unwrap(), Arch::X86_64);
    assert_eq!(arch("amd64").unwrap(), Arch::X86_64);
    assert_eq!(arch("aarch64").unwrap(), Arch::Aarch64);
    assert_eq!(arch("arm64").unwrap(), Arch::Aarch64);
}

#[test]
fn names_the_cpu_it_cannot_serve() {
    let err = arch("armv7l").unwrap_err().to_string();
    assert!(err.contains("32-bit ARM"), "{err}");
    let err = arch("riscv64").unwrap_err().to_string();
    assert!(err.contains("riscv64"), "{err}");
}

#[test]
fn reads_the_facts_lines() {
    let facts = "os=Linux\nmachine=x86_64\nlibc=glibc 2.31\nldd=ldd (Ubuntu GLIBC 2.31) 2.31\n";
    assert_eq!(field(facts, "os"), Some("Linux"));
    assert_eq!(field(facts, "machine"), Some("x86_64"));
    assert_eq!(field(facts, "missing"), None);
    assert_eq!(libc_from(facts).unwrap(), Libc::Glibc(2, 31));
}

#[test]
fn musl_is_what_libc_leaves() {
    let facts = "os=Linux\nmachine=aarch64\nlibc=\nldd=musl libc (x86_64)\n";
    assert_eq!(libc_from(facts).unwrap(), Libc::Musl);
}

#[test]
fn an_unknown_libc_is_an_error() {
    let facts = "os=Linux\nmachine=x86_64\nlibc=\nldd=\n";
    assert!(libc_from(facts).is_err());
}

#[test]
fn builds_the_release_names() {
    assert_eq!(
        asset_name("x86_64-unknown-linux-gnu"),
        "shipdeck-agent-x86_64-unknown-linux-gnu.tar.gz"
    );
    let url = asset_url_with(None, "aarch64-unknown-linux-musl");
    assert!(url.starts_with("https://github.com/Naz-Ovh/shipdeck/releases/download/v"));
    assert!(url.ends_with("/shipdeck-agent-aarch64-unknown-linux-musl.tar.gz"));
}

#[test]
fn a_mirror_base_url_replaces_the_release_url() {
    assert_eq!(
        asset_url_with(Some("http://127.0.0.1:8000/"), "x86_64-unknown-linux-gnu"),
        "http://127.0.0.1:8000/shipdeck-agent-x86_64-unknown-linux-gnu.tar.gz"
    );
    assert_eq!(
        asset_url_with(Some("  "), "x86_64-unknown-linux-gnu"),
        asset_url_with(None, "x86_64-unknown-linux-gnu")
    );
}

#[test]
fn the_install_script_downloads_verifies_and_unpacks() {
    let script = install_script_with(
        "https://example.test/shipdeck-agent-x86_64-unknown-linux-gnu.tar.gz",
        "/home/deploy/.shipdeck/bin/shipdeck-agent-0.7.1",
    );
    assert!(script.contains("curl -fsSL --connect-timeout 10 --max-time 45"));
    assert!(script.contains("wget -q --timeout=15 --tries=2"));
    assert!(script.contains("sha256sum"));
    assert!(script.contains("tar -xzf"));
    assert!(script.contains("trap cleanup EXIT"));
    assert!(script.contains("'/home/deploy/.shipdeck/bin/shipdeck-agent-0.7.1'"));
}

#[test]
fn an_odd_path_is_quoted() {
    let script = install_script_with("https://example.test/agent.tar.gz", "/home/it's an agent");
    assert!(script.contains(r"'/home/it'\''s an agent'"), "{script}");
    assert!(!script.contains("/home/it's an agent'"), "{script}");
}
