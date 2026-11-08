use super::{binary_path, launch_command, probe_script, version_dir};

#[test]
fn versioned_paths_are_fixed_under_opt() {
    assert_eq!(version_dir("5.2.1"), "/opt/shipdeck/agent/v5.2.1");
    assert_eq!(binary_path("5.2.1"), "/opt/shipdeck/agent/v5.2.1/agent");
}

#[test]
fn the_probe_checks_ownership_before_running_the_binary() {
    let script = probe_script("/opt/shipdeck/agent/v5.2.1/agent");
    let owner = script.find("stat -c %U").expect("owner check");
    let run = script.find("--version").expect("version probe");
    assert!(owner < run);
    assert!(script.contains("[ ! -L \"$path\" ]"));
    assert!(script.contains("022"));
    assert!(script.contains("while [ \"$dir\" != / ]"));
}

#[test]
fn launch_uses_the_full_path_and_sudo() {
    let command = launch_command(
        "sudo -n -- ",
        "/opt/shipdeck/agent/v5.2.1/agent",
        "deploy",
        "/home/deploy",
    );
    assert!(command.starts_with("sh -c "));
    assert!(command.contains("exec sudo -n -- env"));
    assert!(command.contains("SHIPDECK_OPERATOR="));
    assert!(command.contains("deploy"));
    assert!(command.contains("SHIPDECK_OPERATOR_HOME="));
    assert!(command.contains("/home/deploy"));
    assert!(command.contains("/opt/shipdeck/agent/v5.2.1/agent"));
    assert!(command.contains(".shipdeck-agent.log"));
}

#[test]
fn a_root_launch_has_no_sudo_prefix() {
    let command = launch_command("", "/opt/shipdeck/agent/v5.2.1/agent", "root", "/root");
    assert!(!command.contains("sudo"));
    assert!(command.contains("exec env SHIPDECK_OPERATOR="));
    assert!(command.contains("root"));
}

#[test]
fn the_upload_receiver_runs_with_root_and_the_same_identity() {
    let command = super::receiver_command(
        "sudo -n -- ",
        "/opt/shipdeck/agent/v5.2.1/agent",
        "deploy",
        "/home/deploy",
        "/tmp/.shipdeck-upload-token/pending",
    );
    assert!(command.contains("exec sudo -n -- env"));
    assert!(command.contains("SHIPDECK_OPERATOR="));
    assert!(command.contains("--receive"));
    assert!(command.contains("/tmp/.shipdeck-upload-token/pending"));
    assert!(command.contains(".shipdeck-agent.log"));
}
