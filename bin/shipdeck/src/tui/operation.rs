//! A running or finished deploy or rollback: banner, step checklist, live
//! progress bars and the log.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Gauge, LineGauge, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Wrap,
};

use shipdeck_engine::deploy::report::{Outcome, StepState, Unit};
use shipdeck_engine::human;
use shipdeck_engine::release::ReleaseName;

use super::app::{App, OpKind, Operation, Progress, StepView};
use super::theme::{self, ACCENT, BORDER, ERR, OK, SURFACE, TEXT, VIOLET, WARN};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let Some(op) = &app.op else {
        return;
    };
    let [banner_area, body] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);
    banner(frame, banner_area, app, op);

    let left_width = (body.width * 40 / 100).clamp(46, 64);
    let [left, right] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(0)]).areas(body);
    let [steps_area, progress_area, timeline_area] = Layout::vertical([
        Constraint::Length(op.steps.len() as u16 + 2),
        Constraint::Length(10),
        Constraint::Min(0),
    ])
    .areas(left);
    steps(frame, steps_area, op, app.tick);
    progress(frame, progress_area, op, app.tick);
    timeline(frame, timeline_area, op);
    log(frame, right, op);
}

fn banner(frame: &mut Frame, area: Rect, app: &App, op: &Operation) {
    let deployable = &app.deployables[op.deployable];
    let config = &deployable.config;
    let (title, color) = match (&op.kind, &op.finished) {
        (_, None) => (op_title(op), ACCENT),
        (_, Some((Outcome::Succeeded { .. }, _))) => (op_title(op), OK),
        (_, Some((Outcome::RolledBack { .. }, _))) => (op_title(op), WARN),
        (_, Some(_)) => (op_title(op), ERR),
    };
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(Style::new().fg(color))
        .title(Span::styled(format!(" {title} "), theme::bold(color)))
        .title(
            Line::from(Span::styled(
                format!(" {} ", human::duration(op.elapsed())),
                theme::bold(color),
            ))
            .right_aligned(),
        );

    let release = op
        .release
        .as_deref()
        .map(|id| {
            let tag = ReleaseName::parse(id).map(|n| n.tag()).unwrap_or_default();
            vec![
                Span::styled("  release ", theme::faint()),
                Span::styled(id.to_string(), Style::new().fg(VIOLET)),
                Span::styled(format!("  {tag}"), theme::muted()),
            ]
        })
        .unwrap_or_default();
    let first = Line::from(
        [
            vec![
                Span::styled(config.name.clone(), theme::bold(TEXT)),
                Span::styled("  →  ", theme::faint()),
                Span::styled(
                    format!("{}:{}", deployable.endpoint.label(), config.remote.dir),
                    theme::muted(),
                ),
            ],
            release,
        ]
        .concat(),
    );

    let second = match &op.finished {
        None => {
            let running = op.steps.iter().find(|s| s.state == StepState::Running);
            let what = running.map(|s| s.name).unwrap_or("Starting");
            Line::from(vec![
                Span::styled(
                    format!("{} ", theme::spinner(app.tick)),
                    theme::bold(ACCENT),
                ),
                Span::styled(format!("{what}…"), theme::bold(ACCENT)),
                Span::styled(
                    format!("   step {} of {}", op.completed_steps() + 1, op.steps.len()),
                    theme::muted(),
                ),
            ])
        }
        Some((outcome, _)) => outcome_line(op, outcome),
    };
    frame.render_widget(
        Paragraph::new(vec![first, second])
            .wrap(Wrap { trim: true })
            .block(block.padding(ratatui::widgets::Padding::horizontal(1))),
        area,
    );
}

fn op_title(op: &Operation) -> String {
    match &op.kind {
        OpKind::Deploy => "Deploy".into(),
        OpKind::Rollback { .. } => "Rollback".into(),
    }
}

fn outcome_line(op: &Operation, outcome: &Outcome) -> Line<'static> {
    let failed_step = op
        .steps
        .iter()
        .find(|s| s.state == StepState::Failed)
        .map(|s| s.name)
        .unwrap_or("a step");
    match outcome {
        Outcome::Succeeded { release } => Line::from(vec![
            theme::badge("LIVE", OK),
            Span::styled(
                format!("  {release} is live, healthy and serving."),
                theme::bold(OK),
            ),
        ]),
        Outcome::Failed { error } => Line::from(vec![
            theme::badge("STOPPED", ERR),
            Span::styled(format!("  at {failed_step}: "), theme::bold(ERR)),
            Span::styled(first_line(error), Style::new().fg(ERR)),
            Span::styled("  · the live release was not touched", theme::muted()),
        ]),
        Outcome::RolledBack { error, to } => Line::from(vec![
            theme::badge("ROLLED BACK", WARN),
            Span::styled(format!("  {failed_step} failed: "), theme::bold(WARN)),
            Span::styled(first_line(error), Style::new().fg(WARN)),
            Span::styled(format!("  · {to} is live again"), theme::muted()),
        ]),
        Outcome::RollbackFailed {
            error,
            rollback_error,
        } => Line::from(vec![
            theme::badge("ROLLBACK FAILED", ERR),
            Span::styled(
                format!(
                    "  {}  then: {}",
                    first_line(error),
                    first_line(rollback_error)
                ),
                theme::bold(ERR),
            ),
            Span::styled("  · check the server now", theme::bold(WARN)),
        ]),
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

fn steps(frame: &mut Frame, area: Rect, op: &Operation, tick: u64) {
    // When the panel is shorter than the list, scroll so the step in hand
    // and the one after it stay in view.
    let visible = (area.height.saturating_sub(2) as usize).max(1);
    let focus = op
        .steps
        .iter()
        .rposition(|s| s.state != StepState::Pending)
        .unwrap_or(0);
    let skip = (focus + 2)
        .saturating_sub(visible)
        .min(op.steps.len().saturating_sub(visible));
    let rows: Vec<Row> = op
        .steps
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(index, step)| {
            let name_style = match step.state {
                StepState::Running => theme::bold(ACCENT),
                StepState::Done => theme::text(),
                StepState::Warned => Style::new().fg(WARN),
                StepState::Failed => theme::bold(ERR),
                StepState::Pending | StepState::Skipped => theme::faint(),
            };
            let detail = match (&step.state, &step.progress, &step.detail) {
                (StepState::Running, Some(p), _) => percent_text(p),
                (_, _, Some(detail)) => detail.clone(),
                _ => String::new(),
            };
            let detail_style = match step.state {
                StepState::Failed => Style::new().fg(ERR),
                StepState::Warned => Style::new().fg(WARN),
                StepState::Running => Style::new().fg(ACCENT),
                _ => theme::muted(),
            };
            let time = step.elapsed().map(human::duration).unwrap_or_default();
            Row::new(vec![
                Cell::from(Line::from(vec![
                    theme::step_icon(step.state, tick),
                    Span::styled(format!(" {} ", index + 1), theme::faint()),
                    Span::styled(step.name, name_style),
                ])),
                Cell::from(Span::styled(detail, detail_style)),
                Cell::from(Line::from(Span::styled(time, theme::muted())).right_aligned()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Min(10),
            Constraint::Length(7),
        ],
    )
    .column_spacing(1)
    .block(theme::panel("Steps", false));
    frame.render_widget(table, area);
}

fn percent_text(progress: &Progress) -> String {
    if progress.total == 0 {
        return progress.label.clone();
    }
    format!("{}%", (progress.done * 100 / progress.total).min(100))
}

fn progress(frame: &mut Frame, area: Rect, op: &Operation, tick: u64) {
    let block = theme::panel("Progress", op.running());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 6 {
        return;
    }
    let [title, gauge, amounts, label, _, overall, overall_label] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    match op.focus_step() {
        Some(step) => focus(frame, [title, gauge, amounts, label], step, tick),
        None => frame.render_widget(Span::styled("Waiting to start…", theme::muted()), title),
    }

    let total = op.steps.len().max(1);
    let completed = op.completed_steps();
    let color = match &op.finished {
        None => ACCENT,
        Some((Outcome::Succeeded { .. }, _)) => OK,
        Some((Outcome::RolledBack { .. }, _)) => WARN,
        Some(_) => ERR,
    };
    frame.render_widget(
        LineGauge::default()
            .ratio(completed as f64 / total as f64)
            .filled_style(Style::new().fg(color))
            .unfilled_style(Style::new().fg(BORDER))
            .filled_symbol("━")
            .unfilled_symbol("━")
            .label(Span::styled("overall ", theme::muted())),
        overall,
    );
    frame.render_widget(
        Line::from(vec![
            Span::styled(format!("{completed}/{total} steps"), theme::text()),
            Span::styled(
                format!("  ·  {}", human::duration(op.elapsed())),
                theme::muted(),
            ),
        ]),
        overall_label,
    );
}

/// How long each step took, as bars scaled to the slowest one.
fn timeline(frame: &mut Frame, area: Rect, op: &Operation) {
    if area.height < 4 {
        return;
    }
    let block = theme::panel("Timeline", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let timed: Vec<_> = op
        .steps
        .iter()
        .filter_map(|step| step.elapsed().map(|d| (step, d)))
        .collect();
    if timed.is_empty() {
        frame.render_widget(
            Span::styled("Step times appear here.", theme::faint()),
            inner,
        );
        return;
    }
    let slowest = timed
        .iter()
        .map(|(_, d)| d.as_secs_f64())
        .fold(0.0_f64, f64::max)
        .max(0.001);
    let bar_width = inner.width.saturating_sub(20) as f64;
    let lines: Vec<Line> = timed
        .iter()
        .map(|(step, elapsed)| {
            let color = match step.state {
                StepState::Failed => ERR,
                StepState::Warned => WARN,
                StepState::Running => ACCENT,
                _ => OK,
            };
            let cells = ((elapsed.as_secs_f64() / slowest) * bar_width)
                .round()
                .max(1.0) as usize;
            Line::from(vec![
                Span::styled(format!("{:<10}", step.name), theme::muted()),
                Span::styled("▆".repeat(cells), Style::new().fg(color)),
                Span::styled(format!(" {}", human::duration(*elapsed)), theme::text()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The big bar for the step in focus.
fn focus(frame: &mut Frame, [title, gauge, amounts, label]: [Rect; 4], step: &StepView, tick: u64) {
    let running = step.state == StepState::Running;
    let color = match step.state {
        StepState::Failed => ERR,
        StepState::Warned => WARN,
        StepState::Done => OK,
        _ => ACCENT,
    };
    let elapsed = step.elapsed().map(human::duration).unwrap_or_default();
    frame.render_widget(
        Line::from(vec![
            theme::step_icon(step.state, tick),
            Span::styled(format!(" {}", step.name), theme::bold(color)),
            Span::styled(format!("  {elapsed}"), theme::muted()),
        ]),
        title,
    );

    let Some(p) = step.progress.as_ref() else {
        // No numbers for this step: show a moving bar while it runs.
        if running {
            frame.render_widget(indeterminate(gauge.width, tick, color), gauge);
            frame.render_widget(
                Span::styled(
                    step.detail.clone().unwrap_or_else(|| "working…".into()),
                    theme::muted(),
                ),
                amounts,
            );
        } else if let Some(detail) = &step.detail {
            frame.render_widget(Span::styled(detail.clone(), theme::muted()), amounts);
        }
        return;
    };

    if p.total == 0 {
        frame.render_widget(indeterminate(gauge.width, tick, color), gauge);
    } else {
        let ratio = (p.done as f64 / p.total as f64).clamp(0.0, 1.0);
        frame.render_widget(
            Gauge::default()
                .ratio(ratio)
                .use_unicode(true)
                .gauge_style(Style::new().fg(color).bg(SURFACE))
                .label(Span::styled(
                    format!("{:.0}%", ratio * 100.0),
                    Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
                )),
            gauge,
        );
    }
    let amount = match p.unit {
        Unit::Bytes => format!("{} of {}", human::bytes(p.done), human::bytes(p.total)),
        Unit::Items if step.name == "Build" => format!("{} of {} crates", p.done, p.total),
        Unit::Items => format!("{} of {}", p.done, p.total),
        Unit::Seconds => format!("{}s of {}s", p.done, p.total),
    };
    frame.render_widget(Span::styled(amount, theme::text()), amounts);
    frame.render_widget(Span::styled(p.label.clone(), theme::muted()), label);
}

/// A bright segment sliding along the bar for work without a known end.
fn indeterminate(width: u16, tick: u64, color: Color) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::default();
    }
    let segment = (width / 5).max(3);
    let span = width + segment;
    let head = (tick as usize * 2) % span;
    let mut spans = Vec::with_capacity(width);
    for x in 0..width {
        let lit = x + segment > head && x <= head;
        spans.push(Span::styled(
            "━",
            Style::new().fg(if lit { color } else { BORDER }),
        ));
    }
    Line::from(spans)
}

fn log(frame: &mut Frame, area: Rect, op: &Operation) {
    let following = op.scroll == 0;
    let status = if following {
        Span::styled(" following ", theme::bold(OK))
    } else {
        Span::styled(
            format!(" ↑ {} lines · G to follow ", op.scroll),
            theme::bold(WARN),
        )
    };
    let block = theme::panel(format!("Log · {} lines", op.logs.len()), false)
        .title(Line::from(status).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    let total = op.logs.len();
    let end = total.saturating_sub(op.scroll);
    let start = end.saturating_sub(height);
    let lines: Vec<Line> = op
        .logs
        .range(start..end)
        .map(|line| {
            let (marker, marker_style) = theme::log_marker(line.kind);
            let secs = line.at.as_secs();
            Line::from(vec![
                Span::styled(
                    format!("{:02}:{:02} ", secs / 60, secs % 60),
                    theme::faint(),
                ),
                Span::styled(format!("{marker} "), marker_style),
                Span::styled(line.text.clone(), theme::log_style(line.kind)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);

    if total > height {
        let mut state = ScrollbarState::new(total.saturating_sub(height)).position(start);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::new().fg(ACCENT))
                .track_style(Style::new().fg(BORDER)),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut state,
        );
    }
}
