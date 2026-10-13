//! Dialogs on top of the screens: confirmations, password prompts and the
//! first-connection host key check.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::Kind;

use super::app::{App, Live, Modal};
use super::theme::{self, ACCENT, ERR, INK, MUTED, SURFACE, TEXT, VIOLET, WARN};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let Some(modal) = app.modals.front() else {
        return;
    };
    let (title, color, mut lines, buttons) = match modal {
        Modal::ConfirmDeploy { deployable } => confirm_deploy(app, *deployable),
        Modal::ConfirmRollback {
            deployable,
            target,
            kind,
            current,
        } => confirm_rollback(app, *deployable, target, *kind, current.as_deref()),
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
            vec![("y", "quit anyway"), ("n", "keep running")],
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
            vec![("⏎", "log in"), ("esc", "cancel")],
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
            vec![("y", "trust and connect"), ("n", "refuse")],
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
    Vec<(&'static str, &'static str)>,
);

fn confirm_deploy(app: &App, index: usize) -> Dialog {
    let deployable = &app.deployables[index];
    let config = &deployable.config;
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
    let missing: Vec<String> = deployable
        .bundle_status()
        .into_iter()
        .filter(|(_, exists)| !exists)
        .map(|(entry, _)| entry)
        .collect();

    let mut lines = vec![
        Line::from(Span::styled(config.name.clone(), theme::bold(ACCENT))),
        Line::default(),
        row(
            "to",
            format!("{}:{}", config.ssh.label(), config.remote.dir),
        ),
        row("service", config.remote.service.clone()),
        row("build", config.build.command.join(" ")),
        row("live now", live),
        Line::default(),
        Line::from(Span::styled(
            "The new release goes live after a restart (about 1–2 s of downtime). \
             If it fails its health check, the previous release is put back automatically.",
            theme::muted(),
        )),
    ];
    if !missing.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!(
                "Missing bundle files: {}. The deploy will stop at Pack.",
                missing.join(", ")
            ),
            Style::new().fg(ERR),
        )));
    }
    (
        "Deploy".into(),
        ACCENT,
        lines,
        vec![("y", "deploy"), ("n", "cancel")],
    )
}

fn confirm_rollback(
    app: &App,
    index: usize,
    target: &str,
    kind: Kind,
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
    if kind == Kind::Compressed {
        lines.push(Line::from(Span::styled(
            "This release is compressed; it is unpacked first.",
            Style::new().fg(VIOLET),
        )));
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
        vec![("y", "roll back"), ("n", "cancel")],
    )
}

fn row(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<13}"), Style::new().fg(MUTED)),
        Span::styled(value, theme::text()),
    ])
}
