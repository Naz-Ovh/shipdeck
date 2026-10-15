//! The frame around every screen: brand bar, tabs with the running
//! operation, and the key hints at the bottom.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use shipdeck_engine::deploy::report::{Outcome, StepState};

use super::app::{App, Modal, OpKind, Screen, Tone};
use super::theme::{self, ACCENT, ERR, INK, MUTED, OK, SURFACE, WARN};

pub fn header(frame: &mut Frame, area: Rect, app: &App) {
    let [top, tabs] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let login = app.pool.login().describe();
    let [brand_area, login_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(login.len() as u16 + 8),
    ])
    .areas(top);
    let brand = Line::from(vec![
        Span::styled(
            " ◆ shipdeck ",
            Style::new().fg(INK).bg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("root ", theme::faint()),
        Span::styled(app.root.display().to_string(), theme::muted()),
    ]);
    frame.render_widget(brand, brand_area);
    frame.render_widget(
        Line::from(vec![
            Span::styled("login ", theme::faint()),
            Span::styled(login, theme::muted()),
        ])
        .right_aligned(),
        login_area,
    );

    let mut spans = Vec::new();
    for (number, screen) in Screen::ALL.iter().enumerate() {
        let available = match screen {
            Screen::Deployables => true,
            Screen::Operation => app.op.is_some(),
            Screen::Releases => app.releases.is_some() || app.selected().is_some(),
        };
        let label = format!(" {} {} ", number + 1, screen.title());
        let style = if *screen == app.screen {
            Style::new().fg(INK).bg(ACCENT).add_modifier(Modifier::BOLD)
        } else if available {
            Style::new().fg(MUTED).bg(SURFACE)
        } else {
            theme::faint().bg(SURFACE)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    let [tabs_area, status_area] =
        Layout::horizontal([Constraint::Length(48), Constraint::Min(0)]).areas(tabs);
    frame.render_widget(Line::from(spans), tabs_area);
    frame.render_widget(operation_status(app).right_aligned(), status_area);
}

/// `⠋ Deploying server-prod · Upload 45%`, or how the last one ended.
fn operation_status(app: &App) -> Line<'static> {
    let Some(op) = &app.op else {
        return Line::default();
    };
    let slug = app
        .deployables
        .get(op.deployable)
        .map(|d| d.slug.clone())
        .unwrap_or_default();
    let verb = match op.kind {
        OpKind::Deploy => "Deploying",
        OpKind::Rollback { .. } => "Rolling back",
    };
    match &op.finished {
        None => {
            let step = op.steps.iter().find(|s| s.state == StepState::Running);
            let mut text = format!("{verb} {slug}");
            if let Some(step) = step {
                text.push_str(&format!(" · {}", step.name));
                if let Some(p) = step.progress.as_ref().filter(|p| p.total > 0) {
                    text.push_str(&format!(" {}%", (p.done * 100 / p.total).min(100)));
                }
            }
            Line::from(vec![
                Span::styled(
                    format!("{} ", theme::spinner(app.tick)),
                    theme::bold(ACCENT),
                ),
                Span::styled(text, theme::text()),
            ])
        }
        Some((outcome, _)) => {
            let (icon, color, text) = match outcome {
                Outcome::Succeeded { .. } => ("✔", OK, format!("{slug} is live")),
                Outcome::Failed { .. } => {
                    ("✘", ERR, format!("{slug}: stopped, live release untouched"))
                }
                Outcome::RolledBack { .. } => ("↩", WARN, format!("{slug}: rolled back")),
                Outcome::Kept { .. } => ("▲", WARN, format!("{slug}: kept after a failure")),
                Outcome::RollbackFailed { .. } => ("‼", ERR, format!("{slug}: rollback failed")),
            };
            Line::from(vec![
                Span::styled(format!("{icon} "), theme::bold(color)),
                Span::styled(text, Style::new().fg(color)),
            ])
        }
    }
}

pub fn footer(frame: &mut Frame, area: Rect, app: &App) {
    let hints: &[(&str, &str)] = match app.modals.front() {
        Some(Modal::Secret { .. }) => &[("⏎", "submit"), ("esc", "cancel"), ("ctrl+u", "clear")],
        Some(Modal::Setup { .. }) => &[("c", "copy"), ("⏎", "close")],
        Some(Modal::AskRollback { .. }) => &[("y", "roll back now"), ("n", "keep it live")],
        Some(_) => &[("y", "yes"), ("n", "no")],
        None => match app.screen {
            Screen::Deployables => &[
                ("↑↓", "select"),
                ("⏎", "deploy"),
                ("r", "releases"),
                ("s", "refresh"),
                ("u", "server setup"),
                ("tab", "screens"),
                ("q", "quit"),
            ],
            Screen::Operation => &[
                ("↑↓", "scroll"),
                ("pgup/pgdn", "page"),
                ("G", "follow"),
                ("r", "releases"),
                ("esc", "back"),
                ("q", "quit"),
            ],
            Screen::Releases => &[
                ("↑↓", "select"),
                ("⏎", "roll back"),
                ("r", "refresh"),
                ("esc", "back"),
                ("q", "quit"),
            ],
        },
    };
    let mut spans = Vec::new();
    for (key, label) in hints {
        spans.push(theme::key(key));
        spans.push(Span::styled(format!(" {label}   "), theme::muted()));
    }
    frame.render_widget(Line::from(spans), area);

    if let Some(toast) = &app.toast {
        let color = match toast.tone {
            Tone::Info => ACCENT,
            Tone::Warn => WARN,
        };
        frame.render_widget(
            Line::from(vec![
                Span::styled("● ", theme::bold(color)),
                Span::styled(toast.text.clone(), Style::new().fg(color)),
                Span::raw(" "),
            ])
            .right_aligned(),
            area,
        );
    }
}
