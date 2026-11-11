//! Draws every screen and dialog with sample data on a test terminal.

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc::unbounded_channel;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use shipdeck_engine::config::Deployment;
use shipdeck_engine::config::parse_deploy;
use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpUpdate, Outcome, RollbackQuestion, StepState,
};
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::{Kind, Listing, ReleaseEntry, Status};
use shipdeck_engine::ssh::ssh_config::HostConfig;
use shipdeck_engine::ssh::{Endpoint, Login};

use shipdeck_protocol::connection::Connection;

use super::app::{App, Fetch, Live, Modal, OpKind, Operation, ReleasesView, Screen, Tone};
use super::draw;

const RELEASES: [&str; 3] = [
    "v0.3.1-ab12cd3-20261014.153012",
    "v0.3.0-9f8e7d6-20261012.091000",
    "v0.2.9-1a2b3c4-20261001.120000",
];

fn deployment(slug: &str) -> Deployment {
    let yaml = format!(
        "\
name: {slug}
server: {{ ssh: {{ host: {slug}, user: deploy }}, dir: /opt/{slug} }}
release:
  steps:
    preflight:
      local: npm test
    build:
      local: npm run build
apps:
  web:
    source: web
    files: [web/dist]
    process:
      start: node dist/main.js
      env: {{ NODE_ENV: production }}
      systemd: {{}}
    steps:
      health:
        url: http://127.0.0.1:3000/
  admin:
    source: admin
    files: [admin/dist]
    static: true
"
    );
    let deploy = parse_deploy(&yaml).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    Deployment {
        slug: slug.into(),
        file: PathBuf::from(format!("deploy/{slug}.yaml")),
        project: PathBuf::from("."),
        config_dir: PathBuf::from("."),
        yaml,
        endpoint,
        deploy,
    }
}

pub(super) fn app() -> App {
    let mut app = base_app();
    app.live[0] = Live::Ready(Status {
        dir_exists: true,
        current: Some(RELEASES[1].into()),
        active: "active".into(),
    });
    app
}

fn base_app() -> App {
    let (engine_read, _app_write) = tokio::io::duplex(64 * 1024);
    let (_app_read, engine_write) = tokio::io::duplex(64 * 1024);
    let connection = Connection::start(engine_read, engine_write);
    let (tx, _rx) = unbounded_channel();
    App::new(
        PathBuf::from("."),
        vec![deployment("shop"), deployment("api")],
        Login::Auto,
        connection.handle(),
        tx,
    )
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(120, 40)).unwrap()
}

fn text(app: &mut App) -> String {
    let mut terminal = terminal();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<Vec<_>>()
        .join("")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The releases the other screen tests show.
pub(super) fn listing() -> Listing {
    Listing {
        current: Some(RELEASES[0].into()),
        entries: RELEASES
            .iter()
            .map(|id| ReleaseEntry {
                id: (*id).into(),
                name: ReleaseName::parse(id),
                kind: Kind::Open,
                current: *id == RELEASES[0],
                size_bytes: Some(1024),
                backup: true,
                manifest: None,
            })
            .collect(),
    }
}

/// One key press.
pub(super) fn press(app: &mut App, code: KeyCode) {
    app.on_event(&Event::Key(key(code)));
}

/// Draws at one size and returns the frame's text.
pub(super) fn render(app: &mut App, width: u16, height: u16, _name: &str) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn the_deployables_screen_shows_each_deploy_and_its_apps() {
    let mut app = app();
    let screen = text(&mut app);
    assert!(screen.contains("shop"), "{screen}");
    assert!(screen.contains("api"), "{screen}");
    assert!(screen.contains("admin, web"), "apps are listed: {screen}");
    assert!(
        screen.contains("v0.3.0"),
        "the live release is shown: {screen}"
    );
    assert!(screen.contains("/opt/shop"), "{screen}");
}

#[tokio::test]
async fn moving_and_quitting_work() {
    let mut app = app();
    assert_eq!(app.selected(), Some(0));
    app.on_event(&Event::Key(key(KeyCode::Down)));
    assert_eq!(app.selected(), Some(1));
    app.on_event(&Event::Key(key(KeyCode::Char('q'))));
    assert!(app.quit);
}

#[tokio::test]
async fn the_operation_screen_draws_steps_and_logs() {
    let mut app = app();
    let steps: Vec<String> = shipdeck_engine::deploy::deploy_plan()
        .iter()
        .map(|step| step.name().to_string())
        .collect();
    app.op = Some(Operation::new(1, OpKind::Deploy, 0, &steps));
    app.screen = Screen::Operation;
    app.on_op(OpUpdate {
        op: 1,
        event: OpEvent::Step {
            step: 0,
            state: StepState::Done,
            detail: Some("checked".into()),
        },
    });
    app.on_op(OpUpdate {
        op: 1,
        event: OpEvent::Log {
            kind: LogKind::Info,
            text: "hello from the engine".into(),
        },
    });
    let screen = text(&mut app);
    assert!(screen.contains("Preflight"), "{screen}");
    assert!(screen.contains("hello from the engine"), "{screen}");
}

#[tokio::test]
async fn the_releases_screen_lists_and_marks_the_live_one() {
    let mut app = app();
    let mut table = ratatui::widgets::TableState::default();
    table.select(Some(0));
    app.releases = Some(ReleasesView {
        deployable: 0,
        listing: Fetch::Ready(Listing {
            current: Some(RELEASES[0].into()),
            entries: RELEASES
                .iter()
                .map(|id| ReleaseEntry {
                    id: (*id).into(),
                    name: ReleaseName::parse(id),
                    kind: Kind::Open,
                    current: *id == RELEASES[0],
                    size_bytes: Some(1024),
                    backup: true,
                    manifest: None,
                })
                .collect(),
        }),
        table,
        details: None,
    });
    app.screen = Screen::Releases;
    let screen = text(&mut app);
    assert!(screen.contains("Releases"), "{screen}");
    assert!(screen.contains("v0.3.1"), "{screen}");
}

#[tokio::test]
async fn a_finished_operation_shows_its_outcome() {
    let mut app = app();
    let steps: Vec<String> = shipdeck_engine::deploy::deploy_plan()
        .iter()
        .map(|step| step.name().to_string())
        .collect();
    app.op = Some(Operation::new(1, OpKind::Deploy, 0, &steps));
    app.screen = Screen::Operation;
    app.on_op(OpUpdate {
        op: 1,
        event: OpEvent::Finished(Outcome::RolledBack {
            error: "health failed".into(),
            to: RELEASES[1].into(),
        }),
    });
    let screen = text(&mut app);
    assert!(
        screen.contains("rolled back") || screen.contains("Rolled back"),
        "{screen}"
    );
}

#[tokio::test]
async fn dialogs_draw() {
    let mut app = app();
    app.modals.push_back(Modal::ConfirmDeploy {
        deployable: 0,
        missing: None,
    });
    let screen = text(&mut app);
    assert!(screen.contains("Deploy"), "{screen}");

    let mut app = crate::tests::app();
    app.modals.push_back(Modal::AskRollback {
        op: 1,
        deployable: 0,
        question: RollbackQuestion {
            release: RELEASES[0].into(),
            previous: RELEASES[1].into(),
            step: "Health".into(),
            error: "http 502".into(),
            seconds: 30,
            reply: {
                let (tx, _rx) = tokio::sync::oneshot::channel();
                tx
            },
        },
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
    });
    let screen = text(&mut app);
    assert!(screen.contains("Roll back"), "{screen}");
    assert!(screen.contains("http 502"), "{screen}");
}

#[tokio::test]
async fn a_toast_is_shown_until_it_expires() {
    let mut app = app();
    app.toast = Some(super::app::Toast {
        text: "connected".into(),
        tone: Tone::Info,
        until: std::time::Instant::now() + std::time::Duration::from_secs(4),
    });
    let screen = text(&mut app);
    assert!(screen.contains("connected"), "{screen}");
}

#[tokio::test]
async fn the_command_line_completes() {
    let mut app = app();
    app.on_event(&Event::Key(key(KeyCode::Char(':'))));
    for c in "log".chars() {
        app.on_event(&Event::Key(key(KeyCode::Char(c))));
    }
    app.on_event(&Event::Key(key(KeyCode::Tab)));
    assert_eq!(app.command.as_ref().unwrap().input, "logs");
}

#[tokio::test]
async fn every_screen_draws_without_a_connection() {
    let mut app = base_app();
    for screen in Screen::ALL {
        app.screen = screen;
        let _ = text(&mut app);
    }
}
