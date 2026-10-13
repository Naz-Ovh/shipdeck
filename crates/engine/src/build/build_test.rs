use super::*;

#[test]
fn parses_cargo_progress_with_crate_names() {
    let line = "    Building [=======================> ] 283/286: aws-lc-rs, russh        ";
    assert_eq!(
        cargo_progress(line),
        Some((283, 286, "aws-lc-rs, russh".to_string()))
    );
}

#[test]
fn parses_cargo_progress_without_crate_names() {
    assert_eq!(
        cargo_progress("Building [====>   ] 12/40"),
        Some((12, 40, String::new()))
    );
}

#[test]
fn ignores_ordinary_lines() {
    assert_eq!(cargo_progress("   Compiling russh v0.64.1"), None);
    assert_eq!(cargo_progress("Building the thing"), None);
}
