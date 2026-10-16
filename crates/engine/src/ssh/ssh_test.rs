use super::*;

#[test]
fn quotes_single_quotes_for_sh() {
    assert_eq!(sh_quote("plain"), "'plain'");
    assert_eq!(sh_quote("it's"), r"'it'\''s'");
    assert_eq!(sh_quote("$(rm -rf /)"), "'$(rm -rf /)'");
}

#[test]
fn splits_chunks_into_lines() {
    let mut lines = Lines::default();
    let mut seen = Vec::new();
    lines.feed(b"one\ntw", |l| seen.push(l.to_string()));
    lines.feed(b"o\r\nthree", |l| seen.push(l.to_string()));
    lines.flush(|l| seen.push(l.to_string()));
    assert_eq!(seen, ["one", "two", "three"]);
    assert_eq!(lines.text(), "one\ntwo\r\nthree");
}

#[test]
fn a_character_split_between_packets_survives() {
    let mut lines = Lines::default();
    let mut seen = Vec::new();
    let text = "café\n".as_bytes();
    lines.feed(&text[..4], |l| seen.push(l.to_string()));
    lines.feed(&text[4..], |l| seen.push(l.to_string()));
    assert_eq!(seen, ["café"]);
    assert_eq!(lines.text(), "café\n");
}

#[test]
fn check_reports_stderr_first() {
    let out = Output {
        code: 2,
        stdout: "partial".into(),
        stderr: "tar: broken\n".into(),
    };
    let err = out.check("unpacking").unwrap_err().to_string();
    assert_eq!(err, "unpacking failed (exit 2): tar: broken");
}
