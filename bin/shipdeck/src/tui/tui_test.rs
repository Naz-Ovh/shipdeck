//! Draws every screen and dialog with sample data on a test terminal. Set
//! `SHIPDECK_DUMP=<folder>` to write each frame there as text.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc::unbounded_channel;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use shipdeck_engine::config::{DeployConfig, Deployable, Discovery, parse};
use shipdeck_engine::deploy::deploy_step_names;

use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpUpdate, Outcome, RollbackQuestion, StepState, Unit,
};
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::{Kind, Listing, ReleaseEntry, Status};
use shipdeck_engine::setup;
use shipdeck_engine::ssh::ssh_config::HostConfig;
use shipdeck_engine::ssh::{Endpoint, Login, Pool};

use super::app::{App, Fetch, Live, Modal, OpKind, Operation, ReleasesView, Screen};
use super::draw;

const RELEASES: [&str; 4] = [
    "v0.3.1-ab12cd3-20261014.153012",
    "v0.3.0-9f8e7d6-20261012.091000",
    "v0.2.9-1a2b3c4-20261001.120000",
    "v0.2.8-5d6e7f8-20260920.080000",
];

fn deployable(slug: &str, name: &str, host: &str) -> Deployable {
    let yaml = format!(
        "
name: {name}
project: ../../../Server
ssh: {{ host: {alias}, user: deploy }}
service: {{ name: {slug}, dir: /opt/{slug}, start: ./server -E=.env }}
pipeline:
  build:
    run:
      local: cargo zigbuild --release -p my-api
  pack:
    binary: target/release/server
    files:
      include:
        deploy: [.env, files/]
  health: {{ url: \"http://127.0.0.1:6520/\", timeout_secs: 20 }}
",
        alias = slug.split('-').next().unwrap()
    );
    with_config(slug, host, parse(&yaml).unwrap())
}

/// A Node app: build output instead of a binary, and commands in most steps.
fn node_app() -> Deployable {
    let config = parse(
        "
name: Web (prod)
project: ../../../web
ssh: { host: web, user: deploy }
service:
  name: web
  dir: /opt/web
  start: node dist/main.js
  env_file: .env
  shared: [uploads]
pipeline:
  preflight:
    requires: [npm]
    run:
      server: pg_isready -q
      local: npm test
  build:
    run:
      local: [npm ci, npm run build]
  pack:
    files:
      include:
        project: [dist, package.json, package-lock.json]
        deploy: [.env]
      exclude:
        project: [\"**/*.map\"]
  setup:
    run:
      server: [npm ci --omit=dev, npm run migrate]
    check:
      server: node --check dist/main.js
  health:
    url: http://127.0.0.1:3000/health
    check:
      local: curl -fsS https://web.example.com/health
  done:
    run:
      local: ./notify.sh \"web $SHIPDECK_VERSION is live\"
  failed:
    run:
      local: ./notify.sh \"failed at $SHIPDECK_STEP\"
",
    )
    .unwrap();
    with_config("web-prod", "203.0.113.13", config)
}

fn with_config(slug: &str, host: &str, config: DeployConfig) -> Deployable {
    let endpoint = Endpoint::from_config(
        &config.ssh,
        HostConfig {
            host_name: Some(host.into()),
            user: Some("root".into()),
            identity_file: Some(PathBuf::from("/home/me/.ssh/id_deploy")),
            ..HostConfig::default()
        },
    )
    .unwrap();
    Deployable {
        folder: PathBuf::from(format!("scripts/deploy/{slug}")),
        slug: slug.into(),
        config,
        endpoint,
    }
}

fn app() -> App {
    let (questions, _) = unbounded_channel();
    let (ops, _) = unbounded_channel();
    let (fetched, _) = unbounded_channel();
    let discovery = Discovery {
        deployables: vec![
            deployable("api-prod", "My API (prod)", "203.0.113.10"),
            deployable("proxy-prod", "My Proxy (prod)", "203.0.113.11"),
            deployable("api-beta", "My API (beta)", "203.0.113.12"),
            node_app(),
        ],
        problems: Vec::new(),
    };
    let pool = Arc::new(Pool::new(Login::Auto, questions));
    let mut app = App::new(
        PathBuf::from("./scripts/deploy"),
        discovery,
        pool,
        ops,
        fetched,
    );
    app.live = vec![
        Live::Ready(Status {
            dir_exists: true,
            current: Some(RELEASES[0].into()),
            active: "active".into(),
        }),
        Live::Error("connecting to deploy@203.0.113.11 timed out after 15s".into()),
        Live::Loading,
        Live::Loading,
    ];
    app
}

fn render(app: &mut App, width: u16, height: u16, name: &str) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    if let Ok(dir) = std::env::var("SHIPDECK_DUMP") {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        std::fs::write(format!("{dir}/{name}-{width}x{height}.txt"), text).unwrap();
    }
}

fn send(app: &mut App, event: OpEvent) {
    app.on_op(OpUpdate { op: 1, event });
}

fn running_deploy(app: &mut App) {
    let steps = deploy_step_names(&app.deployables[0].config);
    app.op = Some(Operation::new(1, OpKind::Deploy, 0, &steps));
    app.screen = Screen::Operation;
    send(
        app,
        OpEvent::Release("v0.3.2-77aa001-20261014.160501".into()),
    );
    for (step, detail) in [
        (0, "x86_64 · glibc 2.39 · 41.2 GiB free"),
        (1, "31.0 MiB · glibc ≥ 2.34 · 1m04s"),
        (2, "5 files · 31.2 MiB → 11.8 MiB"),
    ] {
        send(
            app,
            OpEvent::Step {
                step,
                state: StepState::Running,
                detail: None,
            },
        );
        send(
            app,
            OpEvent::Step {
                step,
                state: StepState::Done,
                detail: Some(detail.into()),
            },
        );
    }
    send(
        app,
        OpEvent::Step {
            step: 3,
            state: StepState::Running,
            detail: None,
        },
    );
    send(
        app,
        OpEvent::Progress {
            step: 3,
            done: 7_340_032,
            total: 12_373_196,
            unit: Unit::Bytes,
            label: "4.2 MiB/s".into(),
        },
    );
    for (kind, text) in [
        (LogKind::Step, "Preflight: checking the server"),
        (
            LogKind::Command,
            "connect deploy@203.0.113.10 (password, asked when connecting)",
        ),
        (LogKind::Info, "server: x86_64 · glibc 2.39 · 41.2 GiB free"),
        (LogKind::Step, "Build: compiling on this machine"),
        (LogKind::Err, "   Compiling my-api v0.3.2 (/home/me/my-api)"),
        (
            LogKind::Err,
            "    Finished `release` profile [optimized] target(s) in 1m 04s",
        ),
        (
            LogKind::Warn,
            "the source has uncommitted changes; the release is marked dirty",
        ),
        (LogKind::Step, "Upload: sending the release"),
        (
            LogKind::Command,
            "deploy@203.0.113.10 › upload 11.8 MiB to /opt/api-prod/releases/.v0.3.2.tar.zst",
        ),
    ] {
        send(
            app,
            OpEvent::Log {
                kind,
                text: text.into(),
            },
        );
    }
}

fn listing() -> Listing {
    let entries = RELEASES
        .iter()
        .enumerate()
        .map(|(i, id)| ReleaseEntry {
            id: id.to_string(),
            name: ReleaseName::parse(id),
            kind: if i < 2 { Kind::Open } else { Kind::Compressed },
            current: i == 0,
            size_bytes: Some(if i < 2 { 31_000_000 } else { 11_000_000 }),
            manifest: None,
        })
        .collect();
    Listing {
        current: Some(RELEASES[0].into()),
        entries,
    }
}

#[tokio::test]
async fn draws_every_screen_at_common_and_small_sizes() {
    for (width, height) in [(160, 46), (100, 30), (60, 18)] {
        let mut app = app();
        render(&mut app, width, height, "deployables");

        running_deploy(&mut app);
        render(&mut app, width, height, "deploy-running");

        send(
            &mut app,
            OpEvent::Step {
                step: 3,
                state: StepState::Done,
                detail: Some("11.8 MiB in 2.8s".into()),
            },
        );
        for step in 4..8 {
            send(
                &mut app,
                OpEvent::Step {
                    step,
                    state: StepState::Running,
                    detail: None,
                },
            );
            send(
                &mut app,
                OpEvent::Step {
                    step,
                    state: StepState::Done,
                    detail: None,
                },
            );
        }
        send(
            &mut app,
            OpEvent::Step {
                step: 8,
                state: StepState::Running,
                detail: None,
            },
        );
        send(
            &mut app,
            OpEvent::Step {
                step: 8,
                state: StepState::Failed,
                detail: Some("the service crashed and was restarted 1 time(s)".into()),
            },
        );
        send(
            &mut app,
            OpEvent::Step {
                step: 9,
                state: StepState::Skipped,
                detail: Some("rolled back".into()),
            },
        );
        send(
            &mut app,
            OpEvent::Finished(Outcome::RolledBack {
                error: "the service crashed and was restarted 1 time(s) by systemd".into(),
                to: RELEASES[0].into(),
            }),
        );
        render(&mut app, width, height, "deploy-rolled-back");

        app.releases = Some(ReleasesView {
            deployable: 0,
            listing: Fetch::Ready(listing()),
            table: ratatui::widgets::TableState::default().with_selected(Some(2)),
        });
        app.screen = Screen::Releases;
        render(&mut app, width, height, "releases");

        app.modals.push_back(Modal::ConfirmRollback {
            deployable: 0,
            target: RELEASES[2].into(),
            kind: Kind::Compressed,
            current: Some(RELEASES[0].into()),
        });
        render(&mut app, width, height, "confirm-rollback");
        app.modals.clear();

        app.screen = Screen::Deployables;
        app.modals.push_back(Modal::ConfirmDeploy {
            deployable: 0,
            missing: Some(
                "pipeline.pack.files.include.deploy in scripts/deploy/api-prod: .env (not found)"
                    .into(),
            ),
        });
        render(&mut app, width, height, "confirm-deploy");
        app.modals.clear();

        app.modals.push_back(Modal::TrustHost {
            host: "[203.0.113.10]:2222".into(),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:8vrt1yc2dRu4YFMqB5bIC4HImPqycsuGwrSVv8kl7cY".into(),
            reply: None,
        });
        app.modals.push_back(Modal::Secret {
            title: "SSH password".into(),
            prompt: "Password for deploy@203.0.113.10".into(),
            input: "hunter".into(),
            reply: None,
        });
        render(&mut app, width, height, "trust-host");
        let mut setup = setup::command(&app.deployables[0], &Login::Auto);
        setup.command.push_str(
            " --key 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPR1CM562x94ekkDx8f2 me@laptop'",
        );
        setup.key = Some(PathBuf::from("/home/me/.ssh/id_deploy"));
        app.modals.push_front(Modal::Setup {
            deployable: 0,
            setup,
        });
        render(&mut app, width, height, "setup");
        app.modals.pop_front();
        app.modals.pop_front();
        render(&mut app, width, height, "password");
    }
}

#[tokio::test]
async fn draws_a_node_pipeline_and_the_rollback_question() {
    for (width, height) in [(160, 46), (100, 30), (60, 18)] {
        let mut app = app();
        app.list.select(Some(3));
        render(&mut app, width, height, "node-app");

        let steps = deploy_step_names(&app.deployables[3].config);
        assert_eq!(
            steps,
            [
                "Preflight",
                "Build",
                "Pack",
                "Upload",
                "Unpack",
                "Setup",
                "Switch",
                "Restart",
                "Health",
                "Done",
                "Failed"
            ]
        );
        app.op = Some(Operation::new(1, OpKind::Deploy, 3, &steps));
        app.screen = Screen::Operation;
        for (step, detail) in [
            (0, "x86_64 · glibc 2.39 · 41.2 GiB free"),
            (1, "412 files · 12.0s"),
            (2, "412 files · 3.1 MiB → 0.9 MiB"),
            (3, "0.9 MiB in 0.6s"),
            (
                4,
                "releases/v1.4.0-77aa001-20261014.160501 · 1 shared folder linked",
            ),
        ] {
            send(
                &mut app,
                OpEvent::Step {
                    step,
                    state: StepState::Done,
                    detail: Some(detail.into()),
                },
            );
        }
        send(
            &mut app,
            OpEvent::Step {
                step: 5,
                state: StepState::Running,
                detail: None,
            },
        );
        send(
            &mut app,
            OpEvent::Progress {
                step: 5,
                done: 0,
                total: 3,
                unit: Unit::Items,
                label: "npm ci --omit=dev".into(),
            },
        );
        send(
            &mut app,
            OpEvent::Log {
                kind: LogKind::Command,
                text:
                    "deploy@web › npm ci --omit=dev  (in releases/v1.4.0-77aa001-20261014.160501)"
                        .into(),
            },
        );
        render(&mut app, width, height, "node-setup");

        for step in 5..8 {
            send(
                &mut app,
                OpEvent::Step {
                    step,
                    state: StepState::Done,
                    detail: None,
                },
            );
        }
        send(
            &mut app,
            OpEvent::Step {
                step: 8,
                state: StepState::Failed,
                detail: Some("not healthy after 20s: the health url said: 502".into()),
            },
        );
        let (reply, answer) = tokio::sync::oneshot::channel();
        send(
            &mut app,
            OpEvent::AskRollback(RollbackQuestion {
                release: "v1.4.0-77aa001-20261014.160501".into(),
                previous: "v1.3.9-5d6e7f8-20261013.090000".into(),
                step: "Health",
                error: "not healthy after 20s: the health url said: 502".into(),
                seconds: 30,
                reply,
            }),
        );
        assert!(matches!(
            app.modals.front(),
            Some(Modal::AskRollback { .. })
        ));
        render(&mut app, width, height, "ask-rollback");

        // `n` keeps the new release and answers the deploy.
        app.on_event(Event::Key(KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert!(app.modals.is_empty());
        assert_eq!(answer.await, Ok(false));

        send(
            &mut app,
            OpEvent::Step {
                step: 9,
                state: StepState::Skipped,
                detail: Some("kept as it is".into()),
            },
        );
        send(
            &mut app,
            OpEvent::Step {
                step: 10,
                state: StepState::Done,
                detail: Some("1 command".into()),
            },
        );
        send(
            &mut app,
            OpEvent::Finished(Outcome::Kept {
                release: "v1.4.0-77aa001-20261014.160501".into(),
                error: "not healthy after 20s: the health url said: 502".into(),
            }),
        );
        render(&mut app, width, height, "node-kept");
    }
}

#[tokio::test]
async fn an_unanswered_question_closes_when_its_time_is_up() {
    let mut app = app();
    let steps = deploy_step_names(&app.deployables[0].config);
    app.op = Some(Operation::new(1, OpKind::Deploy, 0, &steps));
    let (reply, _answer) = tokio::sync::oneshot::channel();
    send(
        &mut app,
        OpEvent::AskRollback(RollbackQuestion {
            release: RELEASES[0].into(),
            previous: RELEASES[1].into(),
            step: "Restart",
            error: "restart failed".into(),
            seconds: 0,
            reply,
        }),
    );
    assert_eq!(app.modals.len(), 1);
    app.on_tick();
    assert!(app.modals.is_empty());
}

#[tokio::test]
async fn draws_an_empty_root() {
    let (questions, _) = unbounded_channel();
    let (ops, _) = unbounded_channel();
    let (fetched, _) = unbounded_channel();
    let pool = Arc::new(Pool::new(Login::Auto, questions));
    let mut app = App::new(
        PathBuf::from("./scripts/deploy"),
        Discovery::default(),
        pool,
        ops,
        fetched,
    );
    render(&mut app, 100, 30, "empty");
}
