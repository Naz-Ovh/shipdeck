use super::{check_target, finalize_script};

#[test]
fn rejects_non_executables() {
    assert!(check_target(b"not an executable", "x86_64-unknown-linux-gnu").is_err());
}

#[test]
fn checks_before_installing() {
    let script = finalize_script("/tmp/a b/part", "/tmp/a b/agent", "abcdef");
    let verify = script.find("checksum mismatch").expect("checksum check");
    let chmod = script.find("chmod 0700").expect("permissions");
    let rename = script.find("mv ").expect("rename");
    assert!(verify < chmod && chmod < rename);
    assert!(script.contains("'/tmp/a b/part'"));
}

#[cfg(target_os = "linux")]
#[test]
fn accepts_local_elf_and_rejects_other_cpu() {
    let executable = std::env::current_exe().expect("test executable");
    let data = std::fs::read(executable).expect("read test executable");
    let local = format!("{}-unknown-linux-gnu", std::env::consts::ARCH);
    check_target(&data, &local).expect("the local CPU");
    let other = if std::env::consts::ARCH == "x86_64" {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    assert!(check_target(&data, other).is_err());
}
