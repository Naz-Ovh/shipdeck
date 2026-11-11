use clap::Parser;

use super::{Cli, Command, ManagerArg, OnFailureArg, ServerCommand};

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

#[test]
fn deploy_takes_a_deploy_name() {
    let cli = parse(&["shipdeck", "deploy", "production"]);
    match cli.command {
        Some(Command::Server(ServerCommand::Deploy {
            names,
            on_failure,
            allow_unchanged,
            ..
        })) => {
            assert_eq!(names, ["production"]);
            assert_eq!(on_failure, None);
            assert!(!allow_unchanged);
        }
        _ => panic!("not a deploy"),
    }
}

#[test]
fn on_failure_takes_rollback_or_keep() {
    let cli = parse(&["shipdeck", "deploy", "shop", "--on-failure", "keep"]);
    match cli.command {
        Some(Command::Server(ServerCommand::Deploy { on_failure, .. })) => {
            assert_eq!(on_failure, Some(OnFailureArg::Keep));
        }
        _ => panic!("not a deploy"),
    }
    assert!(Cli::try_parse_from(["shipdeck", "deploy", "shop", "--on-failure", "maybe"]).is_err());
}

#[test]
fn a_deploy_file_replaces_the_project_file() {
    let cli = parse(&["shipdeck", "--shipdeckfile", "deploy/prod.yaml", "releases"]);
    assert_eq!(
        cli.shipdeckfile.as_deref(),
        Some(std::path::Path::new("deploy/prod.yaml"))
    );
    match cli.command {
        Some(Command::Server(ServerCommand::Releases { name, .. })) => {
            assert_eq!(name, None);
        }
        _ => panic!("not releases"),
    }
}

#[test]
fn rollback_takes_the_deploy_and_the_release() {
    let cli = parse(&["shipdeck", "rollback", "shop", "v1.2.0-abc-20260101.120000"]);
    match cli.command {
        Some(Command::Server(ServerCommand::Rollback { name, release, .. })) => {
            assert_eq!(name, "shop");
            assert_eq!(release, "v1.2.0-abc-20260101.120000");
        }
        _ => panic!("not a rollback"),
    }
}

#[test]
fn tree_takes_every_option() {
    let cli = parse(&[
        "shipdeck",
        "tree",
        "production",
        "--app",
        "web",
        "--depth",
        "3",
        "--build",
        "--json",
    ]);
    match cli.command {
        Some(Command::Server(ServerCommand::Tree {
            name,
            app,
            depth,
            build,
            json,
        })) => {
            assert_eq!(name.as_deref(), Some("production"));
            assert_eq!(app.as_deref(), Some("web"));
            assert_eq!(depth, Some(3));
            assert!(build && json);
        }
        _ => panic!("not a tree"),
    }
}

#[test]
fn init_takes_a_preset_and_a_manager() {
    let cli = parse(&[
        "shipdeck",
        "init",
        "my-api",
        "--preset",
        "nodejs",
        "--manager",
        "pm2",
        "--host",
        "prod",
    ]);
    match cli.command {
        Some(Command::Init(args)) => {
            assert_eq!(args.name.as_deref(), Some("my-api"));
            assert_eq!(args.manager, Some(ManagerArg::Pm2));
            assert_eq!(args.host.as_deref(), Some("prod"));
        }
        _ => panic!("not init"),
    }
}

#[test]
fn every_server_command_takes_one_deploy() {
    for (command, variant) in [
        (vec!["releases"], "releases"),
        (vec!["server-info"], "server-info"),
        (vec!["work"], "work"),
        (vec!["events"], "events"),
        (vec!["setup"], "setup"),
    ] {
        let mut args = vec!["shipdeck"];
        args.extend(command);
        args.push("shop");
        let cli = parse(&args);
        assert!(matches!(cli.command, Some(Command::Server(_))), "{variant}");
    }
}
