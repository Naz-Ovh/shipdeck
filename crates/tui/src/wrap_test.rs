use super::*;

fn text(line: &Line) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn a_short_line_stays_one_row() {
    let rows = rows(vec![Span::raw("00:01 › ")], "connect", Style::new(), 40);
    assert_eq!(
        rows.iter().map(text).collect::<Vec<_>>(),
        ["00:01 › connect"]
    );
}

#[test]
fn a_long_line_wraps_at_spaces_under_its_text() {
    let rows = rows(
        vec![Span::raw("00:01 › ")],
        "upload 4.5 MiB to /opt/my-api",
        Style::new(),
        24,
    );
    assert_eq!(
        rows.iter().map(text).collect::<Vec<_>>(),
        ["00:01 › upload 4.5 MiB ", "        to /opt/my-api"]
    );
}

#[test]
fn a_word_longer_than_the_room_is_cut() {
    assert_eq!(
        pieces("releases/v0.6.1-ad1d5f6", 10),
        ["releases/v", "0.6.1-ad1d", "5f6"]
    );
    assert_eq!(pieces("", 10), [""]);
    // A wide character in a one-column room still moves on.
    assert_eq!(pieces("日本", 1), ["日", "本"]);
}

#[test]
fn the_bottom_fills_the_view_from_the_newest_line() {
    let line = |rows: usize| vec![Line::raw("x"); rows];
    let newest_first = || [line(2), line(3), line(1)].into_iter();
    // 4 rows: the newest line whole and the bottom 2 rows of the one before.
    let (rows, shown) = bottom(newest_first(), 4);
    assert_eq!((rows.len(), shown), (4, 2));
    let (rows, shown) = bottom(newest_first(), 10);
    assert_eq!((rows.len(), shown), (6, 3));
    assert_eq!(bottom(std::iter::empty(), 4), (vec![], 0));
}
