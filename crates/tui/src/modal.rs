//! Dialogs on top of the screens: confirmations, password prompts and the
//! first-connection host key check.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::Archive;

use std::time::Instant;

use shipdeck_engine::config::{ROLLBACK_ANSWER_SECS, Rollback};
use shipdeck_engine::deploy::report::RollbackQuestion;

use super::app::{App, Live, Modal};
use super::theme::{self, ACCENT, ERR, INK, MUTED, SURFACE, TEXT, VIOLET, WARN};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let Some(modal) = app.modals.front() else {
        return;
    };
    let (title, color, mut lines, buttons) = match modal {
        Modal::ConfirmDeploy {
            deployable,
            missing,
        } => confirm_deploy(app, *deployable, missing.as_deref()),
        Modal::AskRollback {
            deployable,
            question,
            deadline,
            ..
        } => ask_rollback(app, *deployable, question, *deadline),
        Modal::ConfirmRollback {
            deployable,
            target,
            from,
            current,
        } => confirm_rollback(app, *deployable, target, *from, current.as_deref()),
        Modal::ConfirmQuit => (
            "Quit now?".to_string(),
            WARN,
            vec![
                Line::from(Span::styled(
                    "An operation is still running.",
                    theme::bold(TEXT),
                )),
                Line::default(),
                Line::from(Span::styled(
                    "Quitting stops it midway. If it is past the switch, the server may be left \
                     on the new release without a health check.",
                    theme::text(),
                )),
            ],
            vec![("y", "quit anyway".into()), ("n", "keep running".into())],
        ),
        Modal::Secret {
            title,
            prompt,
            input,
            ..
        } => (
            title.clone(),
            ACCENT,
            vec![
                Line::from(Span::styled(prompt.clone(), theme::text())),
                Line::default(),
                Line::from(vec![
                    Span::styled(" ", Style::new().bg(SURFACE)),
                    Span::styled(
                        "•".repeat(input.chars().count()),
                        Style::new().fg(TEXT).bg(SURFACE),
                    ),
                    Span::styled("▏", Style::new().fg(ACCENT).bg(SURFACE)),
                    Span::styled(
                        " ".repeat(40usize.saturating_sub(input.chars().count())),
                        Style::new().bg(SURFACE),
                    ),
                ]),
                Line::default(),
                Line::from(Span::styled(
                    "Kept in memory for this run only; never written to disk.",
                    theme::faint(),
                )),
            ],
            vec![("⏎", "log in".into()), ("esc", "cancel".into())],
        ),
        Modal::TrustHost {
            host,
            algorithm,
            fingerprint,
            ..
        } => (
            "New server".to_string(),
            WARN,
            vec![
                Line::from(vec![
                    Span::styled("First connection to ", theme::text()),
                    Span::styled(host.clone(), theme::bold(TEXT)),
                ]),
                Line::default(),
                Line::from(Span::styled(
                    format!("{algorithm} key fingerprint"),
                    theme::muted(),
                )),
                Line::from(Span::styled(fingerprint.clone(), theme::bold(VIOLET))),
                Line::default(),
                Line::from(Span::styled(
                    "Compare it with `ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub` on the server. \
                     Trusting saves it to ~/.ssh/known_hosts.",
                    theme::muted(),
                )),
            ],
            vec![("y", "trust and connect".into()), ("n", "refuse".into())],
        ),
    };

    lines.push(Line::default());
    let mut spans = Vec::new();
    for (key, label) in buttons {
        spans.push(theme::key(key));
        spans.push(Span::styled(format!(" {label}    "), theme::text()));
    }
    lines.push(Line::from(spans));

    let width = area.width.saturating_sub(4).min(76);
    let text_width = width.saturating_sub(4).max(1) as usize;
    let height: u16 = lines
        .iter()
        .map(|l| (l.width().max(1).div_ceil(text_width)) as u16)
        .sum::<u16>()
        + 2;
    let height = height.min(area.height.saturating_sub(2));
    let rect = area.centered(Constraint::Length(width), Constraint::Length(height));

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(dialog(title, color)),
        rect,
    );
}

fn dialog<'a>(title: String, color: Color) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(color))
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(INK)
                .bg(color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(Color::Rgb(17, 24, 39)))
}

type Dialog = (
    String,
    Color,
    Vec<Line<'static>>,
    Vec<(&'static str, String)>,
);

fn confirm_deploy(app: &App, index: usize, missing: Option<&str>) -> Dialog {
    let deployable = &app.deployables[index];
    let config = &deployable.config;
    let pipeline = &config.pipeline;
    let live = match &app.live[index] {
        Live::Ready(status) => status
            .current
            .as_deref()
            .map(|id| {
                ReleaseName::parse(id)
                    .map(|n| n.tag())
                    .unwrap_or_else(|| id.to_string())
            })
            .unwrap_or_else(|| "nothing yet".into()),
        _ => "unknown until preflight".into(),
    };
    let build = match pipeline.build.run.local.0.as_slice() {
        [] => "nothing to build".to_string(),
        [one] => one.trim().to_string(),
        [first, rest @ ..] => format!("{} (+{} more)", first.trim(), rest.len()),
    };
    let failing = match pipeline.failed.rollback {
        Rollback::Ask => format!(
            "If it fails after the switch, shipdeck asks before putting the previous release back, \
             and does so on its own after {ROLLBACK_ANSWER_SECS} s."
        ),
        Rollback::Auto => {
            "If it fails after the switch, the previous release is put back at once.".into()
        }
    };

    let mut lines = vec![
        Line::from(Span::styled(config.name.clone(), theme::bold(ACCENT))),
        Line::default(),
        row(
            "to",
            format!("{}:{}", deployable.endpoint.label(), config.service.dir),
        ),
        row("service", config.service.name.clone()),
        row("build", build),
        row("live now", live),
        Line::default(),
        Line::from(Span::styled(
            format!(
                "The new release goes live after a restart (about 1–2 s of downtime). {failing}"
            ),
            theme::muted(),
        )),
    ];
    if let Some(missing) = missing {
        lines.push(Line::default());
        let when = if pipeline.preflight.run.local.is_empty() {
            "The deploy will stop at Preflight, before building."
        } else {
            "pipeline.preflight.run.local must create them, or the deploy stops at Preflight."
        };
        lines.push(Line::from(Span::styled(
            format!("{missing}. {when}"),
            Style::new().fg(ERR),
        )));
    }
    (
        "Deploy".into(),
        ACCENT,
        lines,
        vec![("y", "deploy".into()), ("n", "cancel".into())],
    )
}

/// How the rollback question names the two releases: by version, with the
/// commit and build time when both have the same version.
fn names(new: &str, old: &str) -> (String, String) {
    let short = |id: &str| {
        ReleaseName::parse(id)
            .map(|n| format!("v{}", n.version))
            .unwrap_or_else(|| id.to_string())
    };
    if short(new) != short(old) {
        return (short(new), short(old));
    }
    let long = |id: &str| {
        ReleaseName::parse(id)
            .map(|n| format!("v{} ({} {})", n.version, n.git, n.stamp.format("%H:%M:%S")))
            .unwrap_or_else(|| id.to_string())
    };
    (long(new), long(old))
}

fn ask_rollback(app: &App, index: usize, question: &RollbackQuestion, deadline: Instant) -> Dialog {
    let slug = &app.deployables[index].slug;
    let (new, old) = names(&question.release, &question.previous);
    let left = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
    let left = left.min(question.seconds);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{new} failed at {}: ", question.step),
                theme::bold(TEXT),
            ),
            Span::styled(question.error.clone(), Style::new().fg(ERR)),
        ]),
        Line::from(Span::styled(
            format!("{old} was live before it."),
            theme::text(),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled(format!("Roll back to {old}?"), theme::bold(WARN)),
            Span::styled(format!("      rolling back in {left}s"), theme::muted()),
        ]),
    ];
    (
        format!("{slug} · {} failed", question.step.to_lowercase()),
        ERR,
        lines,
        vec![
            ("y", "roll back now".into()),
            ("n", format!("keep {new} live")),
        ],
    )
}

fn confirm_rollback(
    app: &App,
    index: usize,
    target: &str,
    from: Option<Archive>,
    current: Option<&str>,
) -> Dialog {
    let config = &app.deployables[index].config;
    let tag = |id: &str| ReleaseName::parse(id).map(|n| n.tag()).unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(config.name.clone(), theme::bold(ACCENT))),
        Line::default(),
        row("roll back to", format!("{target}  {}", tag(target))),
        row(
            "live now",
            current
                .map(|c| format!("{c}  {}", tag(c)))
                .unwrap_or_else(|| "nothing".into()),
        ),
        Line::default(),
    ];
    let first = match from {
        Some(Archive::Backup) => Some(
            "This release is a backup: it is unpacked and set up again with pipeline.setup first.",
        ),
        Some(Archive::Whole) => Some("This release is compressed; it is unpacked first."),
        None => None,
    };
    if let Some(first) = first {
        lines.push(Line::from(Span::styled(first, Style::new().fg(VIOLET))));
    }
    lines.push(Line::from(Span::styled(
        "The service restarts on that release. If it fails its health check, the current \
         release is put back.",
        theme::muted(),
    )));
    (
        "Roll back".into(),
        WARN,
        lines,
        vec![("y", "roll back".into()), ("n", "cancel".into())],
    )
}

fn row(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<13}"), Style::new().fg(MUTED)),
        Span::styled(value, theme::text()),
    ])
}
