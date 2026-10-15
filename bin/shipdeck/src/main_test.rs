use clap::Parser;

use super::{Cli, Command, OnFailureArg};

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("shipdeck").chain(args.iter().copied()))
}

#[test]
fn a_root_alone_opens_the_tui() {
    let cli = parse(&["./deploy", "--key", "~/.ssh/id_ed25519"]).unwrap();
    assert!(cli.command.is_none());
    assert_eq!(cli.root.unwrap().to_str(), Some("./deploy"));
    assert!(cli.login.key.is_some());
}

#[test]
fn deploy_takes_names_and_rolls_back_by_default() {
    let cli = parse(&["deploy", "./deploy", "my-api", "my-web"]).unwrap();
    let Some(Command::Deploy {
        names,
        on_failure,
        ssh,
        ..
    }) = cli.command
    else {
        panic!("not a deploy");
    };
    assert_eq!(names, ["my-api", "my-web"]);
    assert_eq!(on_failure, OnFailureArg::Rollback);
    assert!(!ssh.trust_new_host);
}

#[test]
fn deploy_flags_go_after_the_command() {
    let cli = parse(&[
        "deploy",
        "./deploy",
        "my-api",
        "--on-failure",
        "keep",
        "--trust-new-host",
        "--key",
        "/keys/deploy",
    ])
    .unwrap();
    let Some(Command::Deploy {
        on_failure, ssh, ..
    }) = cli.command
    else {
        panic!("not a deploy");
    };
    assert_eq!(on_failure, OnFailureArg::Keep);
    assert!(ssh.trust_new_host);
    assert_eq!(cli.login.key.unwrap().to_str(), Some("/keys/deploy"));
}

#[test]
fn deploy_needs_a_name() {
    assert!(parse(&["deploy", "./deploy"]).is_err());
}

#[test]
fn rollback_needs_a_target() {
    assert!(parse(&["rollback", "./deploy", "my-api"]).is_err());
    let cli = parse(&[
        "rollback",
        "./deploy",
        "my-api",
        "--to",
        "v0.3.0-9f8e7d6-20261012.091000",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Rollback { to, .. }) if to == "v0.3.0-9f8e7d6-20261012.091000"
    ));
}

#[test]
fn an_unknown_on_failure_value_is_refused() {
    assert!(parse(&["deploy", "./deploy", "my-api", "--on-failure", "yes"]).is_err());
}

#[test]
fn key_and_password_still_conflict() {
    assert!(
        parse(&[
            "releases",
            "./deploy",
            "my-api",
            "--key",
            "k",
            "--password",
            "p"
        ])
        .is_err()
    );
}

#[test]
fn the_schema_needs_no_root() {
    assert!(parse(&["--schema"]).unwrap().schema);
}
