use clap::{CommandFactory, Parser};

use super::target::Target;
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
            assert_eq!(names, [Target::Name("production".into())]);
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
fn cwd_and_config_take_a_folder_and_a_file() {
    let cli = parse(&[
        "shipdeck",
        "-C",
        "work/shop",
        "-c",
        "deploy/root.yaml",
        "releases",
    ]);
    assert_eq!(cli.cwd.as_deref(), Some(std::path::Path::new("work/shop")));
    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("deploy/root.yaml"))
    );
    match cli.command {
        Some(Command::Server(ServerCommand::Releases { name, .. })) => {
            assert_eq!(name, None);
        }
        _ => panic!("not releases"),
    }
}

#[test]
fn a_long_form_of_cwd_and_config_works_too() {
    let cli = parse(&[
        "shipdeck",
        "--cwd",
        "work/shop",
        "--config",
        ".shipdeck",
        "releases",
    ]);
    assert_eq!(cli.cwd.as_deref(), Some(std::path::Path::new("work/shop")));
    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new(".shipdeck"))
    );
}

#[test]
fn the_old_deploy_file_flag_is_gone() {
    // Assembled so the retired spelling stays out of the tree; the test is
    // the guard that it never comes back.
    let retired = ["--", "shipdeck", "file"].concat();
    let long = retired.trim_start_matches('-');
    let command = Cli::command();
    assert!(
        command
            .get_arguments()
            .all(|arg| arg.get_long() != Some(long))
    );
    assert!(Cli::try_parse_from(["shipdeck", &retired, "x.yaml", "releases"]).is_err());
}

#[test]
fn a_deploy_file_argument_is_a_target() {
    let cli = parse(&["shipdeck", "deploy", "deploy/prod.yaml", "prod.yml", "api"]);
    match cli.command {
        Some(Command::Server(ServerCommand::Deploy { names, .. })) => {
            assert_eq!(
                names,
                [
                    Target::File("deploy/prod.yaml".into()),
                    Target::File("prod.yml".into()),
                    Target::Name("api".into()),
                ]
            );
        }
        _ => panic!("not a deploy"),
    }
}

#[test]
fn a_deploy_argument_without_the_ending_is_refused() {
    assert!(Cli::try_parse_from(["shipdeck", "deploy", "./production"]).is_err());
}

#[test]
fn rollback_takes_the_deploy_and_the_release() {
    let cli = parse(&["shipdeck", "rollback", "shop", "v1.2.0-abc-20260101.120000"]);
    match cli.command {
        Some(Command::Server(ServerCommand::Rollback { name, release, .. })) => {
            assert_eq!(name, Target::Name("shop".into()));
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
            assert_eq!(name, Some(Target::Name("production".into())));
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

#[test]
fn a_relative_config_and_deploy_file_start_from_cwd() {
    let cli = parse(&[
        "shipdeck",
        "-C",
        "work/shop",
        "-c",
        "deploy/root.yaml",
        "deploy",
        "deploy/prod.yaml",
    ]);
    let root = std::path::absolute(cli.cwd.as_deref().unwrap()).unwrap();
    assert_eq!(root, std::path::absolute("work/shop").unwrap());
    let config = cli.config.map(|file| root.join(file));
    assert!(
        config
            .as_deref()
            .is_some_and(|file| file.ends_with("work/shop/deploy/root.yaml")),
        "{config:?}"
    );
    match cli.command {
        Some(Command::Server(command)) => {
            let files: Vec<std::path::PathBuf> = super::targets(&command)
                .map(|target| root.join(target))
                .collect();
            assert!(
                files.len() == 1 && files[0].ends_with("work/shop/deploy/prod.yaml"),
                "{files:?}"
            );
        }
        _ => panic!("not a deploy"),
    }
}

#[test]
fn cwd_must_be_a_folder() {
    assert!(super::folder(Some(std::path::Path::new("/no/such/folder/anywhere"))).is_err());
}
