use super::{LineBuffer, MAX_LINE};

#[test]
fn splits_lines_and_keeps_the_end_of_a_stream_without_a_newline() {
    let mut buffer = LineBuffer::default();
    let mut lines = Vec::new();
    for byte in b"one\ntwo" {
        if let Some(line) = buffer.push(*byte) {
            lines.push(line);
        }
    }
    if let Some(line) = buffer.finish() {
        lines.push(line);
    }
    assert_eq!(lines, ["one", "two"]);
}

#[test]
fn a_line_longer_than_the_limit_keeps_only_its_start() {
    let mut buffer = LineBuffer::default();
    let mut lines = Vec::new();
    for byte in std::iter::repeat_n(b'x', MAX_LINE + 100).chain(std::iter::once(b'\n')) {
        if let Some(line) = buffer.push(byte) {
            lines.push(line);
        }
    }
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].len(), MAX_LINE);
    for byte in b"next\n" {
        if let Some(line) = buffer.push(*byte) {
            lines.push(line);
        }
    }
    assert_eq!(lines.last().map(String::as_str), Some("next"));
}

#[test]
fn a_huge_line_without_a_newline_is_cut_at_the_limit() {
    let mut buffer = LineBuffer::default();
    for byte in std::iter::repeat_n(b'y', MAX_LINE * 3) {
        let _ = buffer.push(byte);
    }
    assert_eq!(buffer.finish().map(|line| line.len()), Some(MAX_LINE));
}
