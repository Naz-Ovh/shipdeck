//! The deployables found under the root, with live status and the selected
//! one's settings.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{HighlightSpacing, List, ListItem, Paragraph, Wrap};

use shipdeck_engine::config::Deployable;
use shipdeck_engine::release::ReleaseName;

use super::app::{App, Live};
use super::theme::{self, ACCENT, ERR, MUTED, OK, SURFACE, TEXT, VIOLET, WARN};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.deployables.is_empty() {
        return empty(frame, area, app);
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);

    let (list_area, problems_area) = if app.broken.is_empty() {
        (left, None)
    } else {
        let height = (app.broken.len() as u16 * 2 + 2).min(left.height / 2);
        let [list, problems] =
            Layout::vertical([Constraint::Min(6), Constraint::Length(height)]).areas(left);
        (list, Some(problems))
    };

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

    if let Some(area) = problems_area {
        problems(frame, area, app);
    }
    if let Some(index) = app.selected() {
        details(
            frame,
            right,
            &app.deployables[index],
            &app.live[index],
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
            Span::styled(config.ssh.label(), theme::muted()),
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
            "▲ remote folder missing: one-time setup needed",
            Style::new().fg(WARN),
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

fn details(frame: &mut Frame, area: Rect, deployable: &Deployable, live: &Live, tick: u64) {
    let config = &deployable.config;
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
    field(
        &mut lines,
        "ssh",
        vec![Span::styled(config.ssh.label(), theme::text())],
    );
    field(
        &mut lines,
        "folder",
        vec![Span::styled(config.remote.dir.clone(), theme::text())],
    );
    let mut service = vec![Span::styled(config.remote.service.clone(), theme::text())];
    if let Live::Ready(status) = live
        && !status.active.is_empty()
    {
        service.push(Span::raw("  "));
        service.push(theme::badge(
            &status.active,
            theme::service_color(&status.active),
        ));
    }
    field(&mut lines, "service", service);
    field(
        &mut lines,
        "runs as",
        vec![Span::styled(deployable.run_as().to_string(), theme::text())],
    );
    field(&mut lines, "live", live_summary(live, tick));

    section(&mut lines, "Build");
    field(
        &mut lines,
        "run",
        vec![Span::styled(
            config.build.command.join(" "),
            Style::new().fg(VIOLET),
        )],
    );
    field(
        &mut lines,
        "in",
        vec![Span::styled(config.build.cwd.clone(), theme::text())],
    );
    field(
        &mut lines,
        "binary",
        vec![Span::styled(config.build.binary.clone(), theme::text())],
    );

    section(&mut lines, "Bundle");
    let bundle = deployable.bundle_status();
    if bundle.is_empty() {
        lines.push(Line::from(Span::styled(
            "  only the binary",
            theme::muted(),
        )));
    }
    for (entry, exists) in bundle {
        lines.push(if exists {
            Line::from(vec![
                Span::styled("  ✔ ", theme::bold(OK)),
                Span::styled(entry, theme::text()),
            ])
        } else {
            Line::from(vec![
                Span::styled("  ✘ ", theme::bold(ERR)),
                Span::styled(entry, Style::new().fg(ERR)),
                Span::styled("  missing, the deploy will stop", theme::muted()),
            ])
        });
    }

    section(&mut lines, "Policy");
    field(
        &mut lines,
        "releases",
        vec![Span::styled(
            format!(
                "{} unpacked + {} compressed",
                config.releases.open, config.releases.compressed
            ),
            theme::text(),
        )],
    );
    let health = match &config.health.url {
        Some(url) => format!("active + {url} within {}s", config.health.timeout_secs),
        None => format!("systemd active within {}s", config.health.timeout_secs),
    };
    field(
        &mut lines,
        "health",
        vec![Span::styled(health, theme::text())],
    );
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

fn problems(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = Vec::new();
    for (folder, err) in &app.broken {
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("✘ {name}"),
            theme::bold(ERR),
        )));
        lines.push(Line::from(Span::styled(format!("  {err}"), theme::muted())));
    }
    let block = theme::panel(format!("Problems · {}", app.broken.len()), false)
        .border_style(Style::new().fg(ERR));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

fn empty(frame: &mut Frame, area: Rect, app: &App) {
    let root = app.root.display().to_string();
    let mut lines = vec![
        Line::from(Span::styled("No deployables yet", theme::bold(ACCENT))),
        Line::default(),
        Line::from(Span::styled(
            format!("shipdeck looks for folders with a deploy.json inside {root}"),
            theme::text(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("{root}/server-prod/deploy.json"),
            Style::new().fg(VIOLET),
        )),
        Line::from(Span::styled(
            format!("{root}/server-prod/.env"),
            Style::new().fg(VIOLET),
        )),
        Line::default(),
        Line::from(Span::styled(
            "See docs/plan.md for every deploy.json field.",
            theme::muted(),
        )),
    ];
    if !app.broken.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "These folders have a deploy.json that could not be used:",
            Style::new().fg(ERR),
        )));
        for (folder, err) in &app.broken {
            lines.push(Line::from(Span::styled(
                format!("✘ {}: {err}", folder.display()),
                Style::new().fg(ERR),
            )));
        }
    }
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
