use super::*;
use crate::config::{Pm2, Systemd};

#[test]
fn systemd_commands_are_quoted_for_the_shell() {
    for action in [Control::Stop, Control::Start, Control::Restart] {
        assert_eq!(
            systemctl(action, &sh_quote("my-api")),
            format!("systemctl {} 'my-api'", action.verb())
        );
    }
    let systemd = Manager::Systemd(Systemd::default());
    assert_eq!(
        log_script(Some(&systemd), "my-api", "my-api", "sudo -n "),
        "sudo -n journalctl -u 'my-api' -f -n 200 --no-pager"
    );
}

#[test]
fn pm2_logs_need_no_sudo() {
    assert_eq!(
        log_script(Some(&Manager::Pm2(Pm2::default())), "my-api", "web", ""),
        "pm2 logs 'web' --raw --lines 200"
    );
}
