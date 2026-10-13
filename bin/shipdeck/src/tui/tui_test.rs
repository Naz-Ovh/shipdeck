//! Draws every screen and dialog with sample data on a test terminal. Set
//! `SHIPDECK_DUMP=<folder>` to write each frame there as text.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_engine::config::{DeployConfig, Deployable, Discovery};
use shipdeck_engine::deploy::DEPLOY_STEPS;
use shipdeck_engine::deploy::report::{LogKind, OpEvent, OpUpdate, Outcome, StepState, Unit};
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::{Kind, Listing, ReleaseEntry, Status};
use shipdeck_engine::ssh::{Login, Pool};

use super::app::{App, Fetch, Live, Modal, OpKind, Operation, ReleasesView, Screen};
use super::draw;

const RELEASES: [&str; 4] = [
    "v0.3.1-ab12cd3-20261014.153012",
    "v0.3.0-9f8e7d6-20261012.091000",
    "v0.2.9-1a2b3c4-20261001.120000",
    "v0.2.8-5d6e7f8-20260920.080000",
];

fn deployable(slug: &str, name: &str, host: &str) -> Deployable {
    let config: DeployConfig = serde_json::from_value(serde_json::json!({
        "name": name,
        "ssh": { "host": host, "user": "deploy" },
        "build": {
            "cwd": "../../../Server",
            "command": ["cargo", "zigbuild", "--release", "-p", "fugitive-server"],
            "binary": "target/release/server"
        },
        "remote": { "dir": format!("/opt/{slug}"), "service": slug, "args": ["-E=.env"] },
        "bundle": [".env", "files/"],
        "health": { "url": "http://127.0.0.1:6520/", "timeout_secs": 20 }
    }))
    .unwrap();
    Deployable {
        folder: PathBuf::from(format!("scripts/deploy/{slug}")),
        slug: slug.into(),
        config,
    }
}

fn app() -> App {
    let (questions, _) = unbounded_channel();
    let (ops, _) = unbounded_channel();
    let (fetched, _) = unbounded_channel();
    let discovery = Discovery {
        deployables: vec![
            deployable("server-prod", "Fugitive Server (prod)", "203.0.113.10"),
            deployable("proxy-prod", "Fugitive Proxy (prod)", "203.0.113.11"),
            deployable("server-beta", "Fugitive Server (beta)", "203.0.113.12"),
        ],
        broken: vec![(
            PathBuf::from("scripts/deploy/old"),
            "parsing deploy.json: unknown field `colour`".into(),
        )],
    };
    let pool = Arc::new(Pool::new(Login::Prompt, questions));
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
    app.op = Some(Operation::new(1, OpKind::Deploy, 0, &DEPLOY_STEPS));
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
        (
            LogKind::Err,
            "   Compiling fugitive-server v0.3.2 (/home/me/Server/bin/server)",
        ),
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
            "deploy@203.0.113.10 › upload 11.8 MiB to /opt/server-prod/releases/.v0.3.2.tar.gz",
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
        for step in 4..6 {
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
                step: 6,
                state: StepState::Running,
                detail: None,
            },
        );
        send(
            &mut app,
            OpEvent::Step {
                step: 6,
                state: StepState::Failed,
                detail: Some("the service crashed and was restarted 1 time(s)".into()),
            },
        );
        send(
            &mut app,
            OpEvent::Step {
                step: 7,
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
        app.modals.push_back(Modal::ConfirmDeploy { deployable: 0 });
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
        app.modals.pop_front();
        render(&mut app, width, height, "password");
    }
}

#[tokio::test]
async fn draws_an_empty_root() {
    let (questions, _) = unbounded_channel();
    let (ops, _) = unbounded_channel();
    let (fetched, _) = unbounded_channel();
    let pool = Arc::new(Pool::new(Login::Prompt, questions));
    let mut app = App::new(
        PathBuf::from("./scripts/deploy"),
        Discovery::default(),
        pool,
        ops,
        fetched,
    );
    render(&mut app, 100, 30, "empty");
}
