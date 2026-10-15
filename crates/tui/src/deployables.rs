//! The deployables found under the root, with live status and the selected
//! one's settings.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{HighlightSpacing, List, ListItem, Paragraph, Wrap};

use shipdeck_engine::bundle::unit;
use shipdeck_engine::config::{Commands, Deployable, ROLLBACK_ANSWER_SECS, Rollback};
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::ssh::Login;

use super::app::{App, Live};
use super::theme::{self, ACCENT, ERR, MUTED, OK, SURFACE, TEXT, VIOLET};

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
        .map(|(deployable, live)| item(deployable, live, app.tick))
        .collect();
    let list = List::new(items)
        .block(theme::panel(
            format!("Deployables · {}", app.deployables.len()),
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
            app.pool.login(),
            app.tick,
        );
    }
}

fn item<'a>(deployable: &'a Deployable, live: &Live, tick: u64) -> ListItem<'a> {
    let config = &deployable.config;
    let (dot, dot_color) = match live {
        Live::Ready(status) if status.current.is_some() => {
            ("●", theme::service_color(&status.active))
        }
        Live::Ready(_) => ("○", MUTED),
        Live::Loading => (theme::spinner(tick), ACCENT),
        Live::Error(_) => ("●", ERR),
        Live::Unknown => ("○", theme::FAINT),
    };
    let summary = live_summary(live, tick);
    ListItem::new(Text::from(vec![
        Line::from(vec![
            Span::styled(format!("{dot} "), theme::bold(dot_color)),
            Span::styled(config.name.clone(), theme::bold(TEXT)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(deployable.endpoint.label(), theme::muted()),
            Span::styled(format!("  {}", deployable.slug), theme::faint()),
        ]),
        Line::from([vec![Span::raw("  ")], summary].concat()),
        Line::default(),
    ]))
}

/// One line about what is live: version and service state, or why not.
fn live_summary(live: &Live, tick: u64) -> Vec<Span<'static>> {
    match live {
        Live::Unknown => vec![Span::styled("not checked yet", theme::faint())],
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
                let label = ReleaseName::parse(id)
                    .map(|n| n.tag())
                    .unwrap_or_else(|| id.clone());
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

fn details(
    frame: &mut Frame,
    area: Rect,
    deployable: &Deployable,
    live: &Live,
    login: &Login,
    tick: u64,
) {
    let config = &deployable.config;
    let service = &config.service;
    let mut lines = vec![
        Line::from(Span::styled(
            config.name.clone(),
            theme::bold(ACCENT).add_modifier(Modifier::UNDERLINED),
        )),
        Line::from(Span::styled(
            deployable.folder.display().to_string(),
            theme::faint(),
        )),
    ];

    section(&mut lines, "Target");
    let endpoint = &deployable.endpoint;
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
        vec![Span::styled(service.dir.clone(), theme::text())],
    );
    let mut unit = vec![Span::styled(service.name.clone(), theme::text())];
    if let Live::Ready(status) = live
        && !status.active.is_empty()
    {
        unit.push(Span::raw("  "));
        unit.push(theme::badge(
            &status.active,
            theme::service_color(&status.active),
        ));
    }
    field(&mut lines, "service", unit);
    field(
        &mut lines,
        "runs as",
        vec![Span::styled(deployable.run_as().to_string(), theme::text())],
    );
    field(
        &mut lines,
        "start",
        vec![Span::styled(
            unit::exec_start(deployable, None),
            theme::text(),
        )],
    );
    field(&mut lines, "live", live_summary(live, tick));

    section(&mut lines, "Pipeline");
    let pipeline = &config.pipeline;
    let mut step = Step::new(&mut lines);
    step.commands("preflight", "server", &pipeline.preflight.run.server);
    step.commands("preflight", "local", &pipeline.preflight.run.local);
    if !pipeline.preflight.requires.is_empty() {
        step.text("preflight", "needs", pipeline.preflight.requires.join(", "));
    }
    if pipeline.build.run.local.is_empty() {
        step.faint("build", "nothing to build");
    } else {
        step.commands("build", "local", &pipeline.build.run.local);
    }
    let pack = &pipeline.pack;
    if let Some(binary) = &pack.binary {
        step.text("pack", "binary", binary.clone());
    }
    for (side, list) in [
        ("project", &pack.files.include.project),
        ("deploy", &pack.files.include.deploy),
    ] {
        if !list.is_empty() {
            step.text("pack", side, list.join(", "));
        }
    }
    let excluded: Vec<&str> = pack
        .files
        .exclude
        .project
        .iter()
        .chain(&pack.files.exclude.deploy)
        .map(String::as_str)
        .collect();
    if !excluded.is_empty() {
        step.text("pack", "leaves", excluded.join(", "));
    }
    step.commands("setup", "server", &pipeline.setup.run.server);
    step.commands("setup", "check", &pipeline.setup.check.server);
    let health = &pipeline.health;
    let waits = match &health.url {
        Some(url) => format!("{url} · {}s", health.timeout_secs),
        None => format!("systemd active · {}s", health.timeout_secs),
    };
    step.text("health", "waits", waits);
    step.commands("health", "server", &health.check.server);
    step.commands("health", "local", &health.check.local);
    let rollback = match health.rollback {
        Rollback::Ask => format!("asks, then rolls back after {ROLLBACK_ANSWER_SECS}s"),
        Rollback::Auto => "rolls back at once".to_string(),
    };
    step.text("health", "failing", rollback);
    step.commands("done", "server", &pipeline.done.run.server);
    step.commands("done", "local", &pipeline.done.run.local);
    step.commands("failed", "local", &pipeline.failed.run.local);

    section(&mut lines, "Policy");
    field(
        &mut lines,
        "keeps",
        vec![Span::styled(
            format!(
                "{} unpacked + {} compressed",
                service.keep.open, service.keep.compressed
            ),
            theme::text(),
        )],
    );
    if !service.shared.is_empty() {
        field(
            &mut lines,
            "shared",
            vec![Span::styled(service.shared.join(", "), theme::text())],
        );
    }
    if let Some(file) = &service.env_file {
        field(
            &mut lines,
            "env file",
            vec![Span::styled(file.clone(), theme::text())],
        );
    }
    let unit = if deployable.unit_template_path().is_file() {
        "service.template"
    } else {
        "built-in template"
    };
    field(&mut lines, "unit", vec![Span::styled(unit, theme::text())]);

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(theme::panel(deployable.slug.clone(), false)),
        area,
    );
}

/// Pipeline rows: the step's name once, then where each line runs.
struct Step<'a> {
    lines: &'a mut Vec<Line<'static>>,
    last: &'static str,
}

impl<'a> Step<'a> {
    fn new(lines: &'a mut Vec<Line<'static>>) -> Self {
        Self { lines, last: "" }
    }

    fn row(&mut self, step: &'static str, side: &str, value: Span<'static>) {
        let name = if step == self.last { "" } else { step };
        self.last = step;
        let side_color = match side {
            "local" => OK,
            "server" | "check" => VIOLET,
            _ => MUTED,
        };
        self.lines.push(Line::from(vec![
            Span::styled(format!("  {name:<10}"), theme::text()),
            Span::styled(format!("{side:<8}"), Style::new().fg(side_color)),
            value,
        ]));
    }

    fn commands(&mut self, step: &'static str, side: &str, commands: &Commands) {
        for (index, command) in commands.iter().enumerate() {
            let side = if index == 0 { side } else { "" };
            self.row(
                step,
                side,
                Span::styled(command.trim().to_string(), theme::text()),
            );
        }
    }

    fn text(&mut self, step: &'static str, side: &str, text: String) {
        self.row(step, side, Span::styled(text, theme::text()));
    }

    fn faint(&mut self, step: &'static str, text: &str) {
        self.row(step, "", Span::styled(text.to_string(), theme::faint()));
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
        Line::from(Span::styled("No deployables yet", theme::bold(ACCENT))),
        Line::default(),
        Line::from(Span::styled(
            format!("shipdeck looks for folders with a deploy.yaml inside {root}"),
            theme::text(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("{root}/server-prod/deploy.yaml"),
            Style::new().fg(VIOLET),
        )),
        Line::from(Span::styled(
            format!("{root}/server-prod/.env"),
            Style::new().fg(VIOLET),
        )),
        Line::default(),
        Line::from(Span::styled(
            "The README and examples/ show every deploy.yaml field.",
            theme::muted(),
        )),
    ];
    let height = (lines.len() as u16 + 4).min(area.height);
    let box_area = area.centered(Constraint::Percentage(70), Constraint::Length(height));
    frame.render_widget(
        Paragraph::new(lines)
            .centered()
            .wrap(Wrap { trim: false })
            .block(theme::panel("shipdeck", true)),
        box_area,
    );
}
