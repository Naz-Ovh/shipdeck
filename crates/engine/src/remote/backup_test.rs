use super::*;

#[test]
fn a_listing_is_read_from_its_nul_ended_records() {
    let text = "d./\0d./apps\0d./apps/web\0f./apps/web/server.js\0f./release.json\0";
    let files = parse_files(text);
    assert_eq!(
        files,
        vec![
            ReleaseFile {
                path: String::new(),
                is_dir: true
            },
            ReleaseFile {
                path: "apps".into(),
                is_dir: true
            },
            ReleaseFile {
                path: "apps/web".into(),
                is_dir: true
            },
            ReleaseFile {
                path: "apps/web/server.js".into(),
                is_dir: false
            },
            ReleaseFile {
                path: "release.json".into(),
                is_dir: false
            },
        ]
    );
}

#[test]
fn unknown_records_are_skipped() {
    let text = "x./a\0frelative\0f./ok\0";
    assert_eq!(
        parse_files(text),
        vec![ReleaseFile {
            path: "ok".into(),
            is_dir: false
        }]
    );
}
