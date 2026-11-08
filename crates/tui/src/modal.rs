//! Dialogs on top of the screens: confirmations, password prompts and the
//! first-connection host key check.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::Control;

use std::time::Instant;

use shipdeck_engine::config::{ROLLBACK_ANSWER_SECS, Rollback};
use shipdeck_engine::deploy::report::{RollbackQuestion, UnchangedQuestion};

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
        Modal::AskUnchanged {
            deployable,
            question,
            ..
        } => ask_unchanged(app, *deployable, question),
        Modal::ConfirmRollback {
            deployable,
            target,
            from_backup,
            current,
        } => confirm_rollback(app, *deployable, target, *from_backup, current.as_deref()),
        Modal::ConfirmControl { deployable, action } => confirm_control(app, *deployable, *action),
        Modal::ConfirmCleanup {
            deployable,
            artifacts,
        } => confirm_cleanup(app, *deployable, artifacts),
        Modal::SetupAccess { deployable } => setup_access(app, *deployable),
        Modal::SetupPassword { input, .. } => setup_password(input),
        Modal::Notice { title, text } => notice(title, text),
        Modal::ConfirmQuit => confirm_quit(),
        Modal::Secret {
            title,
            prompt,
            input,
            ..
        } => secret(title, prompt, input),
        Modal::TrustHost {
            host,
            algorithm,
            fingerprint,
            ..
        } => trust_host(host, algorithm, fingerprint),
    };

    lines.push(Line::default());
    let mut spans = Vec::new();
    for (key, label) in buttons {
        spans.push(theme::key(key));
        spans.push(Span::styled(format!(" {label}    "), theme::text()));
    }
    lines.push(Line::from(spans));

    let width = area.width.saturating_sub(4).min(76);
    let text_width = usize::from(width.saturating_sub(4).max(1));
    // A dialog taller than the screen saturates instead of wrapping.
    let height = lines.iter().fold(0_u16, |height, line| {
        let rows = line.width().max(1).div_ceil(text_width);
        height.saturating_add(u16::try_from(rows).unwrap_or(u16::MAX))
    });
    let height = height.saturating_add(2).min(area.height.saturating_sub(2));
    let rect = area.centered(Constraint::Length(width), Constraint::Length(height));

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(dialog(&title, color)),
        rect,
    );
}

fn notice(title: &str, text: &str) -> Dialog {
    (
        title.to_string(),
        ERR,
        text.lines()
            .map(|line| Line::from(Span::styled(line.to_string(), theme::text())))
            .collect(),
        vec![("⏎", "close".into())],
    )
}

fn confirm_cleanup(app: &App, deployable: usize, artifacts: &[String]) -> Dialog {
    let name = app
        .deployables
        .get(deployable)
        .map_or("this deployable", |found| found.slug.as_str());
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{name} left {} unfinished file(s):", artifacts.len()),
            theme::bold(TEXT),
        )),
        Line::default(),
    ];
    for artifact in artifacts.iter().take(12) {
        lines.push(Line::from(Span::styled(artifact.clone(), theme::text())));
    }
    if artifacts.len() > 12 {
        lines.push(Line::from(Span::styled(
            format!("… and {} more", artifacts.len() - 12),
            theme::muted(),
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Only these files go. The live release, other releases, and their backups stay.",
        theme::text(),
    )));
    (
        "Remove unfinished files?".to_string(),
        WARN,
        lines,
        vec![("y", "remove them".into()), ("n", "keep them".into())],
    )
}

fn setup_access(app: &App, deployable: usize) -> Dialog {
    let name = app
        .deployables
        .get(deployable)
        .map_or("this deployable", |found| found.slug.as_str());
    (
        format!("Prepare the server for {name}"),
        ACCENT,
        vec![
            Line::from(Span::styled(
                "shipdeck installs what it needs, creates the project folder, and grants                  password-free sudo.",
                theme::text(),
            )),
            Line::default(),
            Line::from(vec![
                theme::key("f"),
                Span::styled(
                    "  full sudo: every command, for anything on this server",
                    theme::text(),
                ),
            ]),
            Line::from(vec![
                theme::key("c"),
                Span::styled(
                    "  commands only: exactly the sudo commands shipdeck runs",
                    theme::text(),
                ),
            ]),
            Line::default(),
            Line::from(Span::styled(
                "The sudo password is asked next and never saved.",
                theme::muted(),
            )),
        ],
        vec![("f", "full sudo".into()), ("c", "commands".into())],
    )
}

fn setup_password(input: &str) -> Dialog {
    (
        "Sudo password".to_string(),
        ACCENT,
        vec![
            Line::from(Span::styled(
                "Enter the sudo password for this server. Leave it empty when sudo needs none.",
                theme::text(),
            )),
            Line::default(),
            Line::from(vec![
                Span::styled("  ", theme::muted()),
                Span::styled("•".repeat(input.chars().count()), theme::bold(TEXT)),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]),
        ],
        vec![("⏎", "continue".into()), ("esc", "cancel".into())],
    )
}

fn confirm_quit() -> Dialog {
    (
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
    )
}

fn secret(title: &str, prompt: &str, input: &str) -> Dialog {
    let bullets = input.chars().count();
    (
        title.to_string(),
        ACCENT,
        vec![
            Line::from(Span::styled(prompt.to_string(), theme::text())),
            Line::default(),
            Line::from(vec![
                Span::styled(" ", Style::new().bg(SURFACE)),
                Span::styled("•".repeat(bullets), Style::new().fg(TEXT).bg(SURFACE)),
                Span::styled("▏", Style::new().fg(ACCENT).bg(SURFACE)),
                Span::styled(
                    " ".repeat(40usize.saturating_sub(bullets)),
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
    )
}

fn trust_host(host: &str, algorithm: &str, fingerprint: &str) -> Dialog {
    (
        "New server".to_string(),
        WARN,
        vec![
            Line::from(vec![
                Span::styled("First connection to ", theme::text()),
                Span::styled(host.to_string(), theme::bold(TEXT)),
            ]),
            Line::default(),
            Line::from(Span::styled(
                format!("{algorithm} key fingerprint"),
                theme::muted(),
            )),
            Line::from(Span::styled(fingerprint.to_string(), theme::bold(VIOLET))),
            Line::default(),
            Line::from(Span::styled(
                "Compare it with `ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub` on the server. \
                 Trusting saves it to ~/.ssh/known_hosts.",
                theme::muted(),
            )),
        ],
        vec![("y", "trust and connect".into()), ("n", "refuse".into())],
    )
}

fn dialog<'a>(title: &str, color: Color) -> Block<'a> {
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
        Live::Ready(status) => status.current.as_deref().map_or_else(
            || "nothing yet".into(),
            |id| ReleaseName::parse(id).map_or_else(|| id.to_string(), |n| n.tag()),
        ),
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
        ReleaseName::parse(id).map_or_else(|| id.to_string(), |n| format!("v{}", n.version))
    };
    if short(new) != short(old) {
        return (short(new), short(old));
    }
    let long = |id: &str| {
        ReleaseName::parse(id).map_or_else(
            || id.to_string(),
            |n| format!("v{} ({} {})", n.version, n.git, n.stamp.format("%H:%M:%S")),
        )
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

fn confirm_control(app: &App, index: usize, action: Control) -> Dialog {
    let deployable = &app.deployables[index];
    let service = &deployable.config.service;
    let verb = action.verb();
    let what = match action {
        Control::Stop => {
            "It stays stopped until it is started again, a deploy restarts it or the server reboots."
        }
        _ => "It is down for a moment while it restarts.",
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{} ", capitalized(verb)), theme::bold(TEXT)),
            Span::styled(service.name.clone(), theme::bold(VIOLET)),
            Span::styled(
                format!(" on {}?", deployable.endpoint.label()),
                theme::bold(TEXT),
            ),
        ]),
        Line::default(),
        Line::from(Span::styled(what, theme::muted())),
    ];
    (
        format!("{} · {verb}", deployable.slug),
        WARN,
        lines,
        vec![("y", verb.to_string()), ("n", "cancel".into())],
    )
}

/// `stop` → `Stop`.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn ask_unchanged(app: &App, index: usize, question: &UnchangedQuestion) -> Dialog {
    let slug = &app.deployables[index].slug;
    let lines = vec![
        Line::from(vec![
            Span::styled("Nothing changed since ", theme::bold(TEXT)),
            Span::styled(question.live.clone(), theme::bold(VIOLET)),
            Span::styled(" (live).", theme::bold(TEXT)),
        ]),
        Line::default(),
        Line::from(Span::styled(
            "Deploying again makes a duplicate release.",
            theme::text(),
        )),
        Line::default(),
        Line::from(Span::styled("Deploy anyway?", theme::bold(WARN))),
    ];
    (
        format!("{slug} · nothing changed"),
        WARN,
        lines,
        vec![("y", "deploy anyway".into()), ("n", "stop here".into())],
    )
}

fn confirm_rollback(
    app: &App,
    index: usize,
    target: &str,
    from_backup: bool,
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
            current.map_or_else(|| "nothing".into(), |c| format!("{c}  {}", tag(c))),
        ),
        Line::default(),
    ];
    if from_backup {
        lines.push(Line::from(Span::styled(
            "This release is a backup: it is unpacked and set up again with pipeline.setup first.",
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
        vec![("y", "roll back".into()), ("n", "cancel".into())],
    )
}

fn row(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<13}"), Style::new().fg(MUTED)),
        Span::styled(value, theme::text()),
    ])
}
