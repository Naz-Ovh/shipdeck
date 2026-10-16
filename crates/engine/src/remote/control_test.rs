use super::*;
use crate::remote::sudo_commands;

/// What `sh` passes to sudo, quotes removed: the words the rule must name.
fn words(script: &str) -> String {
    script.replace('\'', "")
}

#[test]
fn what_runs_with_sudo_is_what_the_rule_allows() {
    let allowed: Vec<String> = sudo_commands("/opt/my-api", "my-api")
        .into_iter()
        .chain(control_commands("my-api"))
        .collect();
    for action in [Control::Stop, Control::Start, Control::Restart] {
        let shell = systemctl(action, &sh_quote("my-api"));
        assert_eq!(shell, format!("systemctl {} 'my-api'", action.verb()));
        assert!(allowed.contains(&words(&shell)), "{shell}");
    }
    let follow = log_script(Manager::Systemd, "my-api", "my-api", "sudo -n ");
    assert_eq!(
        follow,
        "sudo -n journalctl -u 'my-api' -f -n 200 --no-pager"
    );
    assert!(allowed.contains(&words(follow.trim_start_matches("sudo -n "))));
}

#[test]
fn control_commands_stay_out_of_what_a_deploy_needs() {
    // Preflight fails a deploy when one of sudo_commands is missing, so a
    // server set up before these must not need them to deploy.
    let deploy = sudo_commands("/opt/my-api", "my-api");
    for command in control_commands("my-api") {
        assert!(!deploy.contains(&command), "{command}");
    }
}

#[test]
fn pm2_logs_need_no_sudo() {
    assert_eq!(
        log_script(Manager::Pm2, "my-api", "web", ""),
        "pm2 logs 'web' --raw --lines 200"
    );
}
