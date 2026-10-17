//! Log lines wrapped to a panel's width, like an editor's word wrap: a long
//! line goes on below, lined up under its own text.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// The rows one log line takes in `width` columns: `prefix` (the time and
/// marker) on the first, then `text`; rows after the first start under the
/// text, not under the prefix.
pub fn rows(
    prefix: Vec<Span<'static>>,
    text: &str,
    style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let indent: usize = prefix.iter().map(Span::width).sum();
    let mut first = Some(prefix);
    pieces(text, width.saturating_sub(indent).max(1))
        .into_iter()
        .map(|piece| {
            let mut spans = first
                .take()
                .unwrap_or_else(|| vec![Span::raw(" ".repeat(indent))]);
            spans.push(Span::styled(piece, style));
            Line::from(spans)
        })
        .collect()
}

/// The rows at the bottom of a view `height` rows tall, from the rows of
/// each log line, newest line first. Also how many lines show, even in part.
pub fn bottom(
    newest_first: impl Iterator<Item = Vec<Line<'static>>>,
    height: usize,
) -> (Vec<Line<'static>>, usize) {
    let mut lines = Vec::new();
    let mut taken = 0;
    for rows in newest_first {
        if taken >= height {
            break;
        }
        taken += rows.len();
        lines.push(rows);
    }
    let shown = lines.len();
    let rows: Vec<Line<'static>> = lines.into_iter().rev().flatten().collect();
    let skip = rows.len().saturating_sub(height);
    (rows.into_iter().skip(skip).collect(), shown)
}

/// `text` cut into pieces at most `room` columns wide, after a space when
/// the piece has one, else mid-word.
fn pieces(text: &str, room: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    loop {
        let mut used = 0;
        let mut cut = rest.len();
        for (at, c) in rest.char_indices() {
            used += c.width().unwrap_or(0);
            if used > room {
                cut = at;
                break;
            }
        }
        if cut == rest.len() {
            pieces.push(rest.to_string());
            return pieces;
        }
        // A character wider than the room still goes, alone.
        let cut = if cut == 0 {
            rest.chars().next().map_or(rest.len(), char::len_utf8)
        } else {
            rest[..cut].rfind(' ').map_or(cut, |space| space + 1)
        };
        pieces.push(rest[..cut].to_string());
        rest = &rest[cut..];
        if rest.is_empty() {
            return pieces;
        }
    }
}

#[cfg(test)]
#[path = "wrap_test.rs"]
mod tests;
