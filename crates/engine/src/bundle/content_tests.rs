use super::*;
use crate::bundle::plan::{Entry, Origin, Plan, Source};

fn plan() -> Plan {
    Plan {
        entries: vec![
            Entry {
                path: ".shipdeck/release.json".into(),
                source: Source::Bytes(b"{}".to_vec()),
                mode: 0o644,
                size: 2,
                sha256: Some(sha256(b"{}")),
                origin: Origin::Generated,
            },
            Entry {
                path: "index.html".into(),
                source: Source::Bytes(b"<html></html>".to_vec()),
                mode: 0o644,
                size: 13,
                sha256: Some(sha256(b"<html></html>")),
                origin: Origin::Release,
            },
            Entry {
                path: "run".into(),
                source: Source::Link(std::path::PathBuf::from("/opt/shop/shared/run")),
                mode: 0o777,
                size: 0,
                sha256: None,
                origin: Origin::Release,
            },
        ],
        links: Vec::new(),
        missing: Vec::new(),
        roots: std::collections::BTreeSet::default(),
    }
}

#[test]
fn the_fingerprint_ignores_the_manifest_and_changes_with_content() {
    let one = fingerprint(&plan());
    assert_eq!(one, fingerprint(&plan()));
    let mut two = plan();
    two.entries[1].sha256 = Some(sha256(b"changed"));
    assert_ne!(one, fingerprint(&two));
    let mut three = plan();
    three.entries[1].mode = 0o600;
    assert_ne!(one, fingerprint(&three));
}

#[test]
fn sha256sum_listing_refuses_awkward_names() {
    assert!(listable("apps/a/b.js"));
    assert!(!listable("apps/a\\b.js"));
    assert!(!listable("apps/a\nb.js"));
}

#[test]
fn hashes_are_the_lowercase_hex_sha256() {
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
