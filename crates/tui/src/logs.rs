//! A service's log, followed live over the whole screen (`:logs`).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

use super::app::{App, LogsState};
use super::theme::{self, ACCENT, BORDER, ERR, OK, WARN};
use super::wrap;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let Some(logs) = &app.logs else {
        return;
    };
    let deployable = &app.deployables[logs.deployable];
    let status = match &logs.state {
        LogsState::Connecting => Span::styled(
            format!(" {} connecting ", theme::spinner(app.tick)),
            theme::bold(ACCENT),
        ),
        LogsState::Ended(_) => Span::styled(" ended ", theme::bold(ERR)),
        LogsState::Following if logs.scroll == 0 => Span::styled(" following ", theme::bold(OK)),
        LogsState::Following => Span::styled(
            format!(" ↑ {} lines · G to follow ", logs.scroll),
            theme::bold(WARN),
        ),
    };
    let block = theme::panel(
        format!(
            "Logs · {} · {}",
            deployable.config.service.name,
            deployable.endpoint.label()
        ),
        true,
    )
    .title(Line::from(status).right_aligned())
    .title_bottom(Line::from(Span::styled(
        format!(" {} ", logs.command),
        theme::faint(),
    )));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Why it ended goes under the lines, wrapped, so a long hint shows whole.
    let ended = match &logs.state {
        LogsState::Ended(why) => Some(why.as_str()),
        _ => None,
    };
    let width = inner.width.max(1) as usize;
    let note_height = ended.map_or(0, |why| {
        why.lines()
            .map(|line| line.chars().count().max(1).div_ceil(width))
            .sum::<usize>() as u16
            + 1
    });
    let [lines_area, note_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(note_height)]).areas(inner);

    let height = lines_area.height as usize;
    let total = logs.lines.len();
    let end = total.saturating_sub(logs.scroll);
    let (lines, shown) = wrap::bottom(
        logs.lines
            .range(..end)
            .rev()
            .map(|line| wrap::rows(Vec::new(), line, theme::text(), width)),
        height,
    );
    let start = end - shown;
    if lines.is_empty() && logs.state == LogsState::Connecting {
        frame.render_widget(
            Span::styled("Connecting to the server…", theme::muted()),
            lines_area,
        );
    } else {
        frame.render_widget(Paragraph::new(lines), lines_area);
    }
    if let Some(why) = ended {
        let mut note = vec![Line::default()];
        note.extend(
            why.lines()
                .map(|line| Line::from(Span::styled(line.to_string(), Style::new().fg(ERR)))),
        );
        frame.render_widget(Paragraph::new(note).wrap(Wrap { trim: false }), note_area);
    }

    if start > 0 || end < total {
        let mut state = ScrollbarState::new(total.saturating_sub(shown)).position(start);
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
