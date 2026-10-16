use chrono::TimeZone;

use super::*;

fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
}

#[test]
fn id_round_trips() {
    let name = ReleaseName::new("0.3.1", "ab12cd3", false, at(2026, 10, 14, 15, 30, 12));
    assert_eq!(name.id(), "v0.3.1-ab12cd3-20261014.153012");
    assert_eq!(ReleaseName::parse(&name.id()), Some(name));
}

#[test]
fn dirty_and_prerelease_versions_round_trip() {
    let name = ReleaseName::new("1.0.0-beta.2", "9f8e7d6", true, at(2026, 1, 2, 3, 4, 5));
    assert_eq!(name.id(), "v1.0.0-beta.2-9f8e7d6-dirty-20260102.030405");
    assert_eq!(ReleaseName::parse(&name.id()), Some(name));
}

#[test]
fn foreign_names_are_not_parsed() {
    for id in [
        "current",
        "v0.3.1",
        "v0.3.1-ab12cd3-yesterday",
        ".v0.3.1-ab12cd3-20261014.153012",
        "v0.3.1-ab12cd3-20261014.153012;rm",
    ] {
        assert_eq!(ReleaseName::parse(id), None, "{id}");
    }
}

#[test]
fn tag_mentions_uncommitted_changes() {
    let clean = ReleaseName::new("0.3.1", "ab12cd3", false, at(2026, 10, 14, 0, 0, 0));
    assert_eq!(clean.tag(), "v0.3.1 (ab12cd3)");
    let dirty = ReleaseName {
        dirty: true,
        ..clean
    };
    assert_eq!(dirty.tag(), "v0.3.1 (ab12cd3, uncommitted changes)");
}

#[test]
fn manifest_records_the_build_time_in_utc() {
    let name = ReleaseName::new("0.3.1", "ab12cd3", false, at(2026, 10, 14, 15, 30, 12));
    let manifest = Manifest::new(&name, "naz");
    assert_eq!(manifest.id, "v0.3.1-ab12cd3-20261014.153012");
    assert_eq!(manifest.built_at, "2026-10-14T15:30:12+00:00");
}

#[test]
fn a_manifest_from_before_fingerprints_still_reads() {
    let old = r#"{"id":"x","version":"0.3.1","git":"ab12cd3","dirty":false,"built_at":"t","deployed_by":"me"}"#;
    let manifest: Manifest = serde_json::from_str(old).unwrap();
    assert_eq!(manifest.content, None);
    let name = ReleaseName::new("0.3.1", "ab12cd3", false, at(2026, 10, 14, 15, 30, 12));
    let json = serde_json::to_string(&Manifest {
        content: Some("sha256:00".into()),
        ..Manifest::new(&name, "me")
    })
    .unwrap();
    assert!(json.contains(r#""content":"sha256:00""#), "{json}");
}
