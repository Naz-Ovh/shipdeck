use clap::Parser;

use super::*;
use crate::{Cli, Command, ServerCommand};

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

fn releases_login(args: &[&str]) -> LoginArgs {
    let Some(Command::Server(ServerCommand::Releases { ssh, .. })) = parse(args).command else {
        panic!("releases is a server command");
    };
    ssh.login
}

fn env(pairs: &[(&str, &str)]) -> Env {
    Env::from_pairs(pairs.iter().copied())
}

#[test]
fn the_key_file_flag_replaces_key() {
    let cli = parse(&["shipdeck", "releases", "--key-file", "id_ed25519"]);
    let Some(Command::Server(ServerCommand::Releases { ssh, .. })) = cli.command else {
        panic!("releases is a server command");
    };
    assert_eq!(
        ssh.login.key_file.as_deref(),
        Some(std::path::Path::new("id_ed25519"))
    );
    assert!(Cli::try_parse_from(["shipdeck", "releases", "--key", "id"]).is_err());
}

#[test]
fn a_missing_key_file_is_an_error() {
    let args = releases_login(&["shipdeck", "releases", "--key-file", "/no/such/key"]);
    let err = login(args, &Env::empty()).unwrap_err();
    assert!(err.to_string().contains("/no/such/key"));
}

#[test]
fn key_file_conflicts_with_the_other_ways() {
    for other in [["--key-text", "k"], ["--password", "p"]] {
        let args = [
            "shipdeck",
            "releases",
            "--key-file",
            "id",
            other[0],
            other[1],
        ];
        assert!(Cli::try_parse_from(args).is_err(), "{other:?}");
    }
}

#[test]
fn password_and_key_text_have_no_short_form() {
    for short in ["-p", "-k", "-K", "-w"] {
        assert!(
            Cli::try_parse_from(["shipdeck", "releases", short, "x"]).is_err(),
            "{short}"
        );
    }
}

#[test]
fn the_server_flags_have_short_forms() {
    let args = releases_login(&[
        "shipdeck",
        "releases",
        "-H",
        "shop-prod",
        "-P",
        "2222",
        "-U",
        "ops",
    ]);
    assert_eq!(args.host.as_deref(), Some("shop-prod"));
    assert_eq!(args.port.as_deref(), Some("2222"));
    assert_eq!(args.user.as_deref(), Some("ops"));
    let long = releases_login(&[
        "shipdeck", "releases", "--host", "h", "--port", "1", "--user", "u",
    ]);
    assert_eq!(long.ssh().host.as_deref(), Some("h"));
    assert_eq!(long.ssh().port.as_deref(), Some("1"));
    assert_eq!(long.ssh().user.as_deref(), Some("u"));
}

#[test]
fn the_short_flags_do_not_clash_with_each_other_or_with_help_and_version() {
    // clap's own check of the whole command: a clash panics here.
    <Cli as clap::CommandFactory>::command().debug_assert();
    let cli = parse(&[
        "shipdeck",
        "-C",
        "dir",
        "-c",
        "root.yaml",
        "-E",
        "ci.env",
        "releases",
        "-H",
        "h",
    ]);
    assert_eq!(
        cli.env_file.as_deref(),
        Some(std::path::Path::new("ci.env"))
    );
    assert!(Cli::try_parse_from(["shipdeck", "-h"]).is_err());
    assert!(Cli::try_parse_from(["shipdeck", "-V"]).is_err());
}

#[test]
fn env_file_is_global_like_cwd_and_config() {
    let before = parse(&["shipdeck", "--env-file", "a.env", "releases"]);
    let after = parse(&["shipdeck", "releases", "--env-file", "a.env"]);
    assert_eq!(before.env_file, after.env_file);
}

#[test]
fn the_login_variables_come_from_the_env_and_a_flag_wins() {
    let none = releases_login(&["shipdeck", "releases"]);
    assert!(matches!(
        login(none.clone(), &Env::empty()).unwrap(),
        Login::Auto
    ));
    assert!(matches!(
        login(none.clone(), &env(&[(PASSWORD_ENV, "pw")])).unwrap(),
        Login::Password(password) if password == "pw"
    ));
    // SHIPDECK_SSH_KEY comes before SHIPDECK_PASSWORD.
    assert!(matches!(
        login(none, &env(&[(PASSWORD_ENV, "pw"), (KEY_ENV, "key text")])).unwrap(),
        Login::KeyText(text) if text == "key text"
    ));
    let flag = releases_login(&["shipdeck", "releases", "--password", "typed"]);
    assert!(matches!(
        login(flag, &env(&[(KEY_ENV, "key text")])).unwrap(),
        Login::Password(password) if password == "typed"
    ));
}
