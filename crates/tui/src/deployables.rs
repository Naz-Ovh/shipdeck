//! The deploys of the project, with live status and the selected one's
//! settings.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{HighlightSpacing, List, ListItem, Paragraph, Wrap};

use shipdeck_engine::config::{Command, Deployment, Program, RollbackMode};
use shipdeck_engine::human;
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::ssh::Login;
use shipdeck_protocol::message::agent::ServerFacts;
use shipdeck_protocol::message::request::WorkReport;

use super::app::{App, Live};
use super::theme::{self, ACCENT, ERR, MUTED, OK, SURFACE, TEXT, VIOLET, WARN};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.deployables.is_empty() {
        return empty(frame, area, app);
    }
    let [list_area, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);

    let items: Vec<ListItem> = app
        .deployables
        .iter()
        .zip(&app.live)
        .map(|(deployment, live)| item(deployment, live, app.tick))
        .collect();
    let list = List::new(items)
        .block(theme::panel(
            format!("Deploys · {}", app.deployables.len()),
            true,
        ))
        .highlight_style(Style::new().bg(SURFACE))
        .highlight_symbol(Line::from(Span::styled("▌", theme::bold(ACCENT))))
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(list, list_area, &mut app.list);

    if let Some(index) = app.selected() {
        details(
            frame,
            right,
            &app.deployables[index],
            &app.live[index],
            &app.login,
            app.tick,
            app.facts[index].as_ref(),
            app.work[index].as_ref(),
        );
    }
}

fn item<'a>(deployment: &'a Deployment, live: &Live, tick: u64) -> ListItem<'a> {
    let (dot, dot_color) = match live {
        Live::Ready(status) if status.current.is_some() => {
            ("●", theme::service_color(&status.active))
        }
        Live::Ready(_) => ("○", MUTED),
        Live::Loading => (theme::spinner(tick), ACCENT),
        Live::Error(_) => ("●", ERR),
        Live::Disconnected => ("○", theme::FAINT),
    };
    let summary = live_summary(live, tick);
    let apps = deployment.app_names().join(", ");
    ListItem::new(Text::from(vec![
        Line::from(vec![
            Span::styled(format!("{dot} "), theme::bold(dot_color)),
            Span::styled(deployment.name().to_string(), theme::bold(TEXT)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(deployment.endpoint.label(), theme::muted()),
            Span::styled(format!("  {apps}"), theme::faint()),
        ]),
        Line::from([vec![Span::raw("  ")], summary].concat()),
        Line::default(),
    ]))
}

/// One line about what is live: version and service state, or why not.
fn live_summary(live: &Live, tick: u64) -> Vec<Span<'static>> {
    match live {
        Live::Disconnected => vec![Span::styled("not connected · c connects", theme::faint())],
        Live::Loading => vec![Span::styled(
            format!("{} connecting…", theme::spinner(tick)),
            Style::new().fg(ACCENT),
        )],
        Live::Error(err) => vec![Span::styled(
            format!("✘ {}", err.lines().next().unwrap_or_default()),
            Style::new().fg(ERR),
        )],
        Live::Ready(status) if !status.dir_exists => vec![Span::styled(
            "not deployed yet: the first deploy creates its folder",
            theme::muted(),
        )],
        Live::Ready(status) => match &status.current {
            None => vec![Span::styled("nothing deployed yet", theme::muted())],
            Some(id) => {
                let label = ReleaseName::parse(id).map_or_else(|| id.clone(), |n| n.tag());
                let state = if status.active.is_empty() {
                    "unknown"
                } else {
                    &status.active
                };
                vec![
                    Span::styled(label, Style::new().fg(VIOLET)),
                    Span::styled(" · ", theme::faint()),
                    Span::styled(
                        state.to_string(),
                        Style::new().fg(theme::service_color(state)),
                    ),
                ]
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn details(
    frame: &mut Frame,
    area: Rect,
    deployment: &Deployment,
    live: &Live,
    login: &Login,
    tick: u64,
    facts: Option<&ServerFacts>,
    work: Option<&WorkReport>,
) {
    let mut lines = vec![
        Line::from(Span::styled(
            deployment.name().to_string(),
            theme::bold(ACCENT).add_modifier(Modifier::UNDERLINED),
        )),
        Line::from(Span::styled(
            deployment.file.display().to_string(),
            theme::faint(),
        )),
    ];

    section(&mut lines, "Target");
    let endpoint = &deployment.endpoint;
    let mut ssh = vec![Span::styled(endpoint.label(), theme::text())];
    if endpoint.is_alias() {
        ssh.push(Span::styled(
            format!("  → {}", endpoint.address()),
            theme::muted(),
        ));
        ssh.push(Span::styled("  ~/.ssh/config", theme::faint()));
    }
    field(&mut lines, "ssh", ssh);
    let login = match (login, &endpoint.identity) {
        (Login::Key(path), _) => format!("key {} (--key)", path.display()),
        (Login::KeyText(_), _) => "key text (--key-text or SHIPDECK_SSH_KEY)".to_string(),
        (Login::Password(_), _) => "password (--password)".to_string(),
        (Login::Auto, Some(path)) => {
            format!("key {} (~/.ssh/config), else password", path.display())
        }
        (Login::Auto, None) => "password, asked when connecting".to_string(),
    };
    field(
        &mut lines,
        "login",
        vec![Span::styled(login, theme::text())],
    );
    field(
        &mut lines,
        "folder",
        vec![Span::styled(deployment.dir().to_string(), theme::text())],
    );
    field(&mut lines, "live", live_summary(live, tick));

    apps_section(&mut lines, deployment);

    server_section(&mut lines, facts);
    work_section(&mut lines, work);
    steps_section(&mut lines, deployment);
    policy_section(&mut lines, deployment);

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(theme::panel(deployment.slug.clone(), false)),
        area,
    );
}

fn apps_section(lines: &mut Vec<Line<'static>>, deployment: &Deployment) {
    section(lines, "Apps");
    for (name, app) in deployment.apps() {
        let kind = if app.process.is_some() {
            let manager = deployment
                .manager(name)
                .map_or("no manager", |manager| manager.label());
            format!("process · {manager} · {}", deployment.process_name(name))
        } else {
            "static".to_string()
        };
        field(
            lines,
            name,
            vec![
                Span::styled(kind, theme::text()),
                Span::styled(
                    format!("  {}", deployment.source(name).display()),
                    theme::faint(),
                ),
            ],
        );
        if let Some((program, rest)) = deployment.start(name) {
            let program = match program {
                Program::Tool(name) => name,
                Program::Absolute(path) | Program::InRelease(path) => path,
            };
            let start = if rest.is_empty() {
                program
            } else {
                format!("{program} {rest}")
            };
            field(lines, "", vec![Span::styled(start, theme::muted())]);
        }
        if let Some(url) = &app.steps.health.url {
            field(
                lines,
                "",
                vec![Span::styled(format!("health {url}"), theme::faint())],
            );
        }
        if !app.shared.is_empty() {
            field(
                lines,
                "",
                vec![Span::styled(
                    format!("shared: {}", app.shared.join(", ")),
                    theme::faint(),
                )],
            );
        }
    }
}

/// The server's own numbers, once the agent has answered.
fn server_section(lines: &mut Vec<Line<'static>>, facts: Option<&ServerFacts>) {
    let Some(facts) = facts else {
        return;
    };
    section(lines, "Server");
    field(
        lines,
        "system",
        vec![Span::styled(
            format!("{} · kernel {}", facts.os, facts.kernel),
            theme::text(),
        )],
    );
    field(
        lines,
        "cpu",
        vec![Span::styled(
            format!("{} · {} cores", facts.arch, facts.cpu_count),
            theme::text(),
        )],
    );
    field(
        lines,
        "memory",
        vec![Span::styled(
            format!(
                "{} free of {}",
                human::bytes(facts.memory_available),
                human::bytes(facts.memory_total)
            ),
            theme::text(),
        )],
    );
    field(
        lines,
        "disk",
        vec![Span::styled(
            format!(
                "{} free of {} ({})",
                human::bytes(facts.disk_available),
                human::bytes(facts.disk_total),
                facts.disk_path
            ),
            theme::text(),
        )],
    );
    field(
        lines,
        "uptime",
        vec![Span::styled(
            human::duration(std::time::Duration::from_secs(facts.uptime_seconds)),
            theme::text(),
        )],
    );
    field(
        lines,
        "agent",
        vec![Span::styled(facts.agent_version.clone(), theme::text())],
    );
}

/// Unfinished files, an unconfirmed switch, and the newest saved events.
fn work_section(lines: &mut Vec<Line<'static>>, work: Option<&WorkReport>) {
    let Some(work) = work else {
        return;
    };
    if work.unfinished.is_empty() && work.uncertain.is_none() && work.events.is_empty() {
        return;
    }
    section(lines, "Work");
    if let Some(active) = &work.active {
        field(
            lines,
            "service",
            vec![Span::styled(
                active.clone(),
                Style::new().fg(theme::service_color(active)),
            )],
        );
    }
    if let Some(release) = &work.uncertain {
        field(
            lines,
            "uncertain",
            vec![Span::styled(
                format!("activation of {release} was never confirmed"),
                Style::new().fg(WARN),
            )],
        );
    }
    if !work.unfinished.is_empty() {
        field(
            lines,
            "unfinished",
            vec![Span::styled(
                format!("{} file(s) · x cleans up", work.unfinished.len()),
                Style::new().fg(WARN),
            )],
        );
    }
    for event in work.events.iter().rev().take(5).rev() {
        field(
            lines,
            "event",
            vec![Span::styled(
                format!(
                    "#{} {} {} {}",
                    event.sequence, event.created_at, event.kind, event.detail
                ),
                theme::faint(),
            )],
        );
    }
}

/// The release and app commands, step by step.
fn steps_section(lines: &mut Vec<Line<'static>>, deployment: &Deployment) {
    section(lines, "Steps");
    let mut last = "";
    let mut rows = 0;
    for step in shipdeck_engine::deploy::Step::ALL {
        let scopes = shipdeck_engine::deploy::scopes(deployment, step);
        for scope in scopes {
            let prefix = match scope.app {
                Some(name) => format!("{name}/"),
                None => String::new(),
            };
            for (side, list) in [("server", scope.server), ("local", scope.local)] {
                for command in list.iter() {
                    let name = if step.name() == last { "" } else { step.name() };
                    last = step.name();
                    let color = match side {
                        "local" => OK,
                        _ => VIOLET,
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {name:<10}"), theme::text()),
                        Span::styled(format!("{:<7}", step.key()), theme::faint()),
                        Span::styled(format!("{prefix}{side}: "), Style::new().fg(color)),
                        Span::styled(command_text(command), theme::text()),
                    ]));
                    rows += 1;
                }
            }
        }
    }
    if rows == 0 {
        field(
            lines,
            "steps",
            vec![Span::styled(
                "no commands; shipdeck does the work",
                theme::faint(),
            )],
        );
    }
}

/// Policy rows: what is kept, linked and used to start the processes.
fn policy_section(lines: &mut Vec<Line<'static>>, deployment: &Deployment) {
    let keep = deployment.keep();
    section(lines, "Policy");
    field(
        lines,
        "keeps",
        vec![Span::styled(
            format!("{} unpacked + {} compressed", keep.open, keep.compressed),
            theme::text(),
        )],
    );
    let shared = &deployment.deploy.release.shared;
    if !shared.is_empty() {
        field(
            lines,
            "shared",
            vec![Span::styled(shared.join(", "), theme::text())],
        );
    }
    let mode = match deployment.rollback_mode() {
        RollbackMode::Ask => format!("asks, then rolls back after {}s", deployment.ask_secs()),
        RollbackMode::Auto => "rolls back at once".to_string(),
        RollbackMode::Off => "never rolls back on its own".to_string(),
    };
    field(lines, "rollback", vec![Span::styled(mode, theme::text())]);
    field(
        lines,
        "releases",
        vec![Span::styled(
            format!("{} apps", deployment.deploy.apps.len()),
            theme::text(),
        )],
    );
}

fn command_text(command: &Command) -> String {
    match command {
        Command::Sh(text) => text.trim().to_string(),
        Command::Runner { runner, code } => format!("{}: {}", runner.key(), code.trim()),
    }
}

fn section(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(title.to_uppercase(), theme::bold(MUTED)),
        Span::styled(" ─────────────────────────", theme::faint()),
    ]));
}

fn field(lines: &mut Vec<Line<'static>>, name: &str, value: Vec<Span<'static>>) {
    let mut spans = vec![Span::styled(format!("  {name:<9}"), theme::muted())];
    spans.extend(value);
    lines.push(Line::from(spans));
}

fn empty(frame: &mut Frame, area: Rect, app: &App) {
    let root = app.root.display().to_string();
    let lines = vec![
        Line::from(Span::styled("No deploys yet", theme::bold(ACCENT))),
        Line::default(),
        Line::from(Span::styled(
            format!("shipdeck reads {root}/shipdeck.yaml (or .shipdeck)"),
            theme::text(),
        )),
        Line::default(),
        Line::from(Span::styled("deploys:", Style::new().fg(VIOLET))),
        Line::from(Span::styled(
            "  production: deploy/production.yaml",
            Style::new().fg(VIOLET),
        )),
        Line::default(),
        Line::from(Span::styled(
            "The README and examples/ show every field.",
            theme::muted(),
        )),
    ];
    let height = u16::try_from(lines.len() + 4)
        .unwrap_or(u16::MAX)
        .min(area.height);
    let box_area = area.centered(Constraint::Percentage(70), Constraint::Length(height));
    frame.render_widget(
        Paragraph::new(lines)
            .centered()
            .wrap(Wrap { trim: false })
            .block(theme::panel("shipdeck", true)),
        box_area,
    );
}
