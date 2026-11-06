//! The release details popup over the Releases screen: what a release is,
//! every file it holds, and the text of one.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use shipdeck_engine::human;
use shipdeck_engine::remote::{FileInfo, FileKind, Kind, READ_CAP, ReleaseContents, Source};

use super::app::{App, DetailsTab, Fetch, OpenFile, ReleaseDetails};
use super::theme::{self, ACCENT, ERR, OK, SURFACE, TEXT, VIOLET};
use super::wrap;

pub fn draw(frame: &mut Frame, app: &App) {
    let Some(view) = &app.releases else {
        return;
    };
    let Some(details) = &view.details else {
        return;
    };
    let dir = &app.deployables[view.deployable].config.service.dir;
    let area = frame
        .area()
        .centered(Constraint::Percentage(90), Constraint::Percentage(90));
    let tab = |label: &str, on: bool| {
        if on {
            theme::badge(label, ACCENT)
        } else {
            Span::styled(format!(" {label} "), theme::muted())
        }
    };
    let block = theme::panel(format!("Release · {}", details.entry.id), true).title(
        Line::from(vec![
            tab("Details", details.tab == DetailsTab::Details),
            Span::raw(" "),
            tab("Files", details.tab == DetailsTab::Files),
            Span::raw(" "),
        ])
        .right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    match (&details.file, details.tab) {
        (Some(file), _) => draw_file(frame, inner, file, app.tick),
        (None, DetailsTab::Details) => draw_info(frame, inner, details, dir, app.tick),
        (None, DetailsTab::Files) => draw_files(frame, inner, details, dir, app.tick),
    }
}

fn row(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), theme::muted()),
        Span::styled(value.into(), theme::text()),
    ])
}

/// Where `source` is on the server.
fn place(dir: &str, id: &str, source: Source) -> String {
    match source {
        Source::Folder => format!("{dir}/releases/{id}/"),
        Source::Backup => format!("{dir}/backups/{id}.tar.zst"),
    }
}

fn waiting(text: &str, tick: u64) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", theme::spinner(tick)), theme::bold(ACCENT)),
        Span::styled(text.to_string(), theme::text()),
    ])
}

fn draw_info(frame: &mut Frame, area: Rect, details: &ReleaseDetails, dir: &str, tick: u64) {
    let entry = &details.entry;
    let contents = match &details.contents {
        Fetch::Ready(contents) => Some(contents),
        _ => None,
    };
    let manifest = entry
        .manifest
        .as_ref()
        .or(contents.and_then(|c| c.manifest.as_ref()));
    let mut lines = vec![row("release", entry.id.clone())];
    match manifest {
        Some(m) => {
            lines.push(row("version", m.version.clone()));
            let dirty = if m.dirty {
                " (built with uncommitted changes)"
            } else {
                ""
            };
            lines.push(row("commit", format!("{}{dirty}", m.git)));
            lines.push(row("built", m.built_at.clone()));
            lines.push(row("deployed by", m.deployed_by.clone()));
            lines.push(row(
                "content",
                m.content.clone().unwrap_or_else(|| "–".into()),
            ));
        }
        None if contents.is_none() && details.source == Source::Backup => {
            lines.push(waiting("reading release.json from the backup…", tick));
        }
        None => lines.push(row("release.json", "not found")),
    }
    lines.push(Line::default());

    let state = match (entry.current, entry.kind, entry.name.is_some()) {
        (true, _, _) => Span::styled("live", theme::bold(OK)),
        (_, _, false) => Span::styled("not created by shipdeck", theme::faint()),
        (_, Kind::Open, _) => Span::styled("unpacked, ready to switch to", Style::new().fg(ACCENT)),
        (_, Kind::Compressed, _) => {
            Span::styled("kept as its backup only", Style::new().fg(VIOLET))
        }
    };
    lines.push(Line::from(vec![
        Span::styled(format!("{:<13}", "state"), theme::muted()),
        state,
    ]));
    if entry.kind == Kind::Open {
        lines.push(row("folder", place(dir, &entry.id, Source::Folder)));
    }
    lines.push(row(
        "backup",
        if entry.backup {
            place(dir, &entry.id, Source::Backup)
        } else {
            "none".into()
        },
    ));
    if let Some(size) = entry.size_bytes {
        let of = match entry.kind {
            Kind::Open => "the folder",
            Kind::Compressed => "the backup",
        };
        lines.push(row("size", format!("{} ({of})", human::bytes(size))));
    }
    lines.push(Line::default());
    lines.push(match &details.contents {
        Fetch::Loading => waiting("reading the file list…", tick),
        Fetch::Error(err) => Line::from(Span::styled(err.clone(), Style::new().fg(ERR))),
        Fetch::Ready(contents) => row("holds", summary(contents, details.source)),
    });
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// `4 files · 1 folder · 8.2 MiB in the folder`.
fn summary(contents: &ReleaseContents, source: Source) -> String {
    let count = |kind| contents.files.iter().filter(|f| f.kind == kind).count();
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let bytes: u64 = contents
        .files
        .iter()
        .filter(|f| f.kind == FileKind::File)
        .map(|f| f.size)
        .sum();
    let mut parts = vec![
        plural(count(FileKind::File), "file", "files"),
        plural(count(FileKind::Dir), "folder", "folders"),
    ];
    let links = count(FileKind::Link);
    if links > 0 {
        parts.push(plural(links, "link", "links"));
    }
    let from = match source {
        Source::Folder => "in the folder",
        Source::Backup => "in the backup",
    };
    format!("{} · {} {from}", parts.join(" · "), human::bytes(bytes))
}

fn draw_files(frame: &mut Frame, area: Rect, details: &ReleaseDetails, dir: &str, tick: u64) {
    let [head, _, list] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let mut top = vec![Span::styled(
        place(dir, &details.entry.id, details.source),
        theme::muted(),
    )];
    if details.entry.kind == Kind::Open && details.entry.backup {
        top.push(Span::styled("   b shows the ", theme::faint()));
        top.push(Span::styled(
            match details.source {
                Source::Folder => "backup",
                Source::Backup => "folder",
            },
            theme::faint(),
        ));
    }
    frame.render_widget(Line::from(top), head);

    let contents = match &details.contents {
        Fetch::Loading => {
            let text = match details.source {
                Source::Folder => "listing the folder…",
                Source::Backup => "listing the backup (it is read, not unpacked)…",
            };
            frame.render_widget(waiting(text, tick), list);
            return;
        }
        Fetch::Error(err) => {
            frame.render_widget(
                Paragraph::new(Span::styled(err.clone(), Style::new().fg(ERR)))
                    .wrap(Wrap { trim: false }),
                list,
            );
            return;
        }
        Fetch::Ready(contents) => contents,
    };
    if contents.files.is_empty() {
        frame.render_widget(Span::styled("No files.", theme::muted()), list);
        return;
    }
    let height = list.height as usize;
    let offset = (details.selected + 1).saturating_sub(height);
    let lines: Vec<Line> = contents
        .files
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, file)| file_line(file, i == details.selected))
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
}

fn file_line(file: &FileInfo, selected: bool) -> Line<'static> {
    let size = match file.kind {
        FileKind::File => human::bytes(file.size),
        _ => String::new(),
    };
    let mut spans = vec![
        Span::styled(if selected { "› " } else { "  " }, theme::bold(ACCENT)),
        Span::styled(format!("{} ", file.mode), theme::faint()),
        Span::styled(format!("{size:>10}  "), theme::muted()),
    ];
    match file.kind {
        FileKind::Dir => spans.push(Span::styled(
            format!("{}/", file.path),
            Style::new().fg(ACCENT),
        )),
        FileKind::Link => {
            spans.push(Span::styled(file.path.clone(), Style::new().fg(VIOLET)));
            spans.push(Span::styled(
                format!(" → {}", file.target.as_deref().unwrap_or("")),
                theme::muted(),
            ));
        }
        FileKind::File => spans.push(Span::styled(file.path.clone(), Style::new().fg(TEXT))),
    }
    let line = Line::from(spans);
    if selected {
        line.style(Style::new().bg(SURFACE).add_modifier(Modifier::BOLD))
    } else {
        line
    }
}

fn draw_file(frame: &mut Frame, area: Rect, file: &OpenFile, tick: u64) {
    let [head, _, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let mut top = vec![
        Span::styled(file.info.path.clone(), theme::bold(TEXT)),
        Span::styled(
            format!("  {}", human::bytes(file.info.size)),
            theme::muted(),
        ),
    ];
    if file.info.size > READ_CAP {
        top.push(Span::styled(
            format!("  showing the first {}", human::bytes(READ_CAP)),
            theme::faint(),
        ));
    }
    frame.render_widget(Line::from(top), head);

    let text = match &file.text {
        Fetch::Loading => return frame.render_widget(waiting("reading…", tick), body),
        Fetch::Error(err) => {
            return frame.render_widget(
                Paragraph::new(Span::styled(err.clone(), Style::new().fg(ERR)))
                    .wrap(Wrap { trim: false }),
                body,
            );
        }
        Fetch::Ready(text) => text,
    };
    if text.binary {
        frame.render_widget(
            Span::styled("A binary file; it is not shown as text.", theme::muted()),
            body,
        );
        return;
    }
    let numbers = text.text.lines().count().max(1).to_string().len();
    let mut rows = Vec::new();
    for (n, line) in text.text.lines().enumerate().skip(file.scroll) {
        if rows.len() >= body.height as usize {
            break;
        }
        let prefix = vec![Span::styled(
            format!("{:>numbers$} │ ", n + 1),
            theme::faint(),
        )];
        let line = line.trim_end_matches('\r').replace('\t', "    ");
        rows.extend(wrap::rows(
            prefix,
            &line,
            theme::text(),
            body.width as usize,
        ));
    }
    rows.truncate(body.height as usize);
    frame.render_widget(Paragraph::new(rows), body);
}

#[cfg(test)]
#[path = "release_details_test.rs"]
mod tests;
