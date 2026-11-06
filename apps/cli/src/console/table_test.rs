use shipdeck_protocol::message::common::{ReleaseInfo, ReleaseKind, ReleaseName};
use shipdeck_protocol::message::request::ReleasesListResult;

use super::table;

#[test]
fn the_release_table_lines_up() {
    let ids = [
        "v0.3.1-ab12cd3-20261014.153012",
        "v0.3.0-9f8e7d6-20261012.091000",
    ];
    let mut entries: Vec<ReleaseInfo> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| ReleaseInfo {
            id: id.to_string(),
            name: Some(ReleaseName {
                version: if i == 0 { "0.3.1" } else { "0.3.0" }.into(),
                git: if i == 0 { "ab12cd3" } else { "9f8e7d6" }.into(),
                dirty: false,
                built_at: if i == 0 {
                    "2026-10-14T15:30:12+00:00"
                } else {
                    "2026-10-12T09:10:00+00:00"
                }
                .into(),
            }),
            kind: if i == 0 {
                ReleaseKind::Open
            } else {
                ReleaseKind::Compressed
            },
            current: i == 0,
            size_bytes: Some(31 * 1024 * 1024),
            backup: true,
            manifest: None,
        })
        .collect();
    entries.push(ReleaseInfo {
        id: "old".into(),
        name: None,
        kind: ReleaseKind::Open,
        current: false,
        size_bytes: None,
        backup: false,
        manifest: None,
    });
    let text = table(&ReleasesListResult {
        current: Some(ids[0].into()),
        entries,
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("RELEASE  "));
    assert!(lines[1].contains("live"), "{text}");
    assert!(lines[1].contains("2026-10-14 15:30"), "{text}");
    assert!(lines[2].contains("compressed"), "{text}");
    assert!(
        lines[3].starts_with("old ") && lines[3].contains("not-ours"),
        "{text}"
    );
    // Every column starts at the same place on every line.
    let col = lines[0].find("VERSION").unwrap();
    assert!(lines[1][col..].starts_with("0.3.1"), "{text}");
}
