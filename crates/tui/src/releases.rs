//! Every release on the server for one deployable, with rollback.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, Wrap};

use shipdeck_engine::human;
use shipdeck_engine::remote::{Archive, Kind, Listing, ReleaseEntry};

use super::app::{App, Fetch};
use super::theme::{self, ACCENT, ERR, OK, SURFACE, TEXT, VIOLET};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let App {
        releases,
        deployables,
        tick,
        ..
    } = app;
    let Some(view) = releases.as_mut() else {
        return;
    };
    let deployable = &deployables[view.deployable];
    let config = &deployable.config;
    let block = theme::panel(format!("Releases · {}", config.name), true).title(
        Line::from(Span::styled(
            format!(" {}:{} ", deployable.endpoint.label(), config.service.dir),
            theme::muted(),
        ))
        .right_aligned(),
    );

    let listing = match &view.listing {
        Fetch::Loading => {
            let text = Line::from(vec![
                Span::styled(format!("{} ", theme::spinner(*tick)), theme::bold(ACCENT)),
                Span::styled("Reading releases from the server…", theme::text()),
            ]);
            frame.render_widget(Paragraph::new(text).centered().block(block), area);
            return;
        }
        Fetch::Error(err) => {
            let text = vec![
                Line::from(Span::styled(
                    "Could not read the releases",
                    theme::bold(ERR),
                )),
                Line::default(),
                Line::from(Span::styled(err.clone(), Style::new().fg(ERR))),
                Line::default(),
                Line::from(Span::styled("Press r to try again.", theme::muted())),
            ];
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
                area,
            );
            return;
        }
        Fetch::Ready(listing) => listing,
    };

    let [table_area, detail_area] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(8)]).areas(area);

    if listing.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No releases on the server yet. Deploy once and they will show up here.",
                theme::muted(),
            )))
            .centered()
            .block(block),
            table_area,
        );
    } else {
        let rows: Vec<Row> = listing.entries.iter().map(row).collect();
        let header = Row::new([
            "",
            "Release",
            "Version",
            "Commit",
            "Built (UTC)",
            "Size",
            "Where",
        ])
        .style(theme::bold(theme::MUTED))
        .bottom_margin(1);
        let table = Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(36),
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(16),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Min(8),
            ],
        )
        .header(header)
        .column_spacing(2)
        .row_highlight_style(Style::new().bg(SURFACE).add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled("▌", theme::bold(ACCENT)))
        .block(block);
        frame.render_stateful_widget(table, table_area, &mut view.table);
    }

    let selected = view.table.selected().and_then(|i| listing.entries.get(i));
    detail(
        frame,
        detail_area,
        listing,
        selected,
        config.pipeline.backup.keep,
    );
}

fn row(entry: &ReleaseEntry) -> Row<'static> {
    let (version, commit, built) = match &entry.name {
        Some(name) => (
            name.version.clone(),
            if name.dirty {
                format!("{}*", name.git)
            } else {
                name.git.clone()
            },
            name.stamp.format("%Y-%m-%d %H:%M").to_string(),
        ),
        None => ("–".into(), "–".into(), "–".into()),
    };
    let size = entry.size_bytes.map(human::bytes).unwrap_or_default();
    let place = match (entry.current, entry.kind, entry.name.is_some()) {
        (true, _, _) => theme::badge("LIVE", OK),
        (_, _, false) => Span::styled("not ours", theme::faint()),
        (_, Kind::Open, _) => Span::styled("unpacked", Style::new().fg(ACCENT)),
        (_, Kind::Compressed, _) => Span::styled(
            match entry.backup {
                Some(Archive::Whole) => "tar.gz",
                _ => "backup",
            },
            Style::new().fg(VIOLET),
        ),
    };
    let marker = if entry.current {
        Span::styled("●", theme::bold(OK))
    } else {
        Span::raw(" ")
    };
    let id_style = if entry.current {
        theme::bold(OK)
    } else if entry.kind == Kind::Compressed {
        theme::muted()
    } else {
        Style::new().fg(TEXT)
    };
    Row::new(vec![
        Cell::from(marker),
        Cell::from(Span::styled(entry.id.clone(), id_style)),
        Cell::from(version),
        Cell::from(commit),
        Cell::from(built),
        Cell::from(Line::from(size).right_aligned()),
        Cell::from(place),
        Cell::from(Span::styled(
            entry
                .manifest
                .as_ref()
                .map(|m| m.deployed_by.clone())
                .unwrap_or_default(),
            theme::muted(),
        )),
    ])
}

fn detail(
    frame: &mut Frame,
    area: Rect,
    listing: &Listing,
    selected: Option<&ReleaseEntry>,
    keep: shipdeck_engine::config::Keep,
) {
    let open = listing
        .entries
        .iter()
        .filter(|e| e.kind == Kind::Open)
        .count();
    let compressed = listing.entries.len() - open;
    let mut lines = vec![Line::from(vec![
        Span::styled("kept  ", theme::muted()),
        Span::styled(format!("{open}/{} unpacked", keep.open), theme::text()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(
            format!("{compressed}/{} compressed", keep.compressed),
            theme::text(),
        ),
    ])];

    if let Some(entry) = selected {
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("release  ", theme::muted()),
            Span::styled(entry.id.clone(), theme::bold(VIOLET)),
        ]));
        if let Some(manifest) = &entry.manifest {
            lines.push(Line::from(vec![
                Span::styled("deployed by  ", theme::muted()),
                Span::styled(manifest.deployed_by.clone(), theme::text()),
                Span::styled("   built  ", theme::muted()),
                Span::styled(manifest.built_at.clone(), theme::text()),
            ]));
        }
        if entry.name.as_ref().is_some_and(|n| n.dirty) {
            lines.push(Line::from(Span::styled(
                "built from a working copy with uncommitted changes",
                Style::new().fg(theme::WARN),
            )));
        }
        let action = match (entry.current, entry.kind, entry.name.is_some()) {
            (true, _, _) => Span::styled("This release is live.", theme::bold(OK)),
            (_, _, false) => Span::styled(
                "shipdeck did not create this folder and will not switch to it.",
                theme::faint(),
            ),
            (_, Kind::Open, _) => Span::styled(
                "⏎  roll back to this release (switch + restart)",
                theme::bold(ACCENT),
            ),
            (_, Kind::Compressed, _) => Span::styled(
                match entry.backup {
                    Some(Archive::Whole) => {
                        "⏎  roll back to this release (unpack, switch + restart)"
                    }
                    _ => "⏎  roll back to this release (unpack, setup, switch + restart)",
                },
                theme::bold(ACCENT),
            ),
        };
        lines.push(Line::from(action));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(theme::panel("Details", false)),
        area,
    );
}
