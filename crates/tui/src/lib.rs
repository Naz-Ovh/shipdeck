//! The terminal UI: one loop that draws, then waits for a key, an SSH
//! question, an operation update or a fetch result.

mod app;
mod chrome;
mod command;
mod deployables;
mod logs;
mod modal;
mod operation;
mod releases;
mod theme;
mod wrap;

pub use app::{App, Fetched};

use std::time::Duration;

use anyhow::Result;
use crossterm::event::EventStream;
use futures::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use tokio::sync::mpsc::UnboundedReceiver;

use shipdeck_engine::deploy::report::OpUpdate;
use shipdeck_engine::ssh::Question;

use app::Screen;

pub struct Inbox {
    pub questions: UnboundedReceiver<Question>,
    pub ops: UnboundedReceiver<OpUpdate>,
    pub fetched: UnboundedReceiver<Fetched>,
}

pub async fn run(mut app: App, mut inbox: Inbox) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &mut inbox).await;
    ratatui::restore();
    app.pool.close_all().await;
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    inbox: &mut Inbox,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(80));

    while !app.quit {
        terminal.draw(|frame| draw(frame, app))?;
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => app.on_event(event),
                Some(Err(err)) => return Err(err.into()),
                None => break,
            },
            Some(question) = inbox.questions.recv() => app.on_question(question),
            Some(update) = inbox.ops.recv() => {
                app.on_op(update);
                // Cargo and uploads can send many updates; apply them all before redrawing.
                while let Ok(update) = inbox.ops.try_recv() {
                    app.on_op(update);
                }
            }
            Some(fetched) = inbox.fetched.recv() => {
                app.on_fetched(fetched);
                // A log arrives a line at a time; show them all at once.
                while let Ok(fetched) = inbox.fetched.try_recv() {
                    app.on_fetched(fetched);
                }
            }
            _ = tick.tick() => app.on_tick(),
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &mut App) {
    let [header, _, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    chrome::header(frame, header, app);
    match app.screen {
        _ if app.logs.is_some() => logs::draw(frame, body, app),
        Screen::Deployables => deployables::draw(frame, body, app),
        Screen::Operation => operation::draw(frame, body, app),
        Screen::Releases => releases::draw(frame, body, app),
    }
    match &app.command {
        Some(line) => command::draw(frame, footer, line),
        None => chrome::footer(frame, footer, app),
    }
    modal::draw(frame, frame.area(), app);
}

#[cfg(test)]
#[path = "tui_test.rs"]
mod tests;
