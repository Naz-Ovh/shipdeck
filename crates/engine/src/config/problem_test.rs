use std::path::{Path, PathBuf};

use super::*;

fn files() -> Vec<FileProblems> {
    vec![FileProblems {
        path: PathBuf::from("/work/deploy/shop.yaml"),
        source: Some("name: shop\nserver:\n  dir: /opt/shop\n".into()),
        problems: vec![
            Problem::new("`server.dir` is wrong")
                .at(Some(Span::point(3, 7)))
                .label("here")
                .help("use an absolute path"),
        ],
    }]
}

#[test]
fn a_report_names_the_file_line_and_help() {
    let text = render(
        &files(),
        Path::new("/work"),
        false,
        "shipdeck did not start",
    );
    assert!(text.contains("shipdeck did not start"), "{text}");
    assert!(text.contains("shop.yaml"), "{text}");
    assert!(text.contains("deploy/shop.yaml:3:8"), "{text}");
    assert!(text.contains("help"), "{text}");
    assert!(!text.contains("\u{1b}["), "no colours without a terminal");
}

#[test]
fn colour_adds_escape_codes() {
    let text = render(&files(), Path::new("/work"), true, "shipdeck did not start");
    assert!(text.contains("\u{1b}["), "{text}");
}

#[test]
fn a_file_without_source_still_prints_its_problem() {
    let files = vec![FileProblems::without_source(
        PathBuf::from("/work/deploy/shop.yaml"),
        Problem::new("cannot read it: no such file"),
    )];
    let text = render(&files, Path::new("/work"), false, "shipdeck did not start");
    assert!(text.contains("cannot read it"), "{text}");
}
