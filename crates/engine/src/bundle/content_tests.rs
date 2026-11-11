use super::*;
use crate::bundle::plan::{Entry, Origin, Plan, Source, release_plan};
use crate::config::Deployment;
use crate::config::load::parse_deploy;
use crate::release::ReleaseName;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

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
        neutral: BTreeMap::new(),
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

/// A plan with one generated unit or PM2 file, made for the named release.
fn app_plan(process: &str, release: &str) -> Plan {
    let yaml = format!(
        "name: shop\nserver: {{ ssh: {{ host: example.com, user: deploy }}, dir: /opt/shop }}\n\
         apps:\n  web:\n    process:\n      start: node main.js\n      {process}\n"
    );
    let deploy = parse_deploy(&yaml).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    let root = std::path::PathBuf::from("/nonexistent-shipdeck-test");
    let deployment = Deployment {
        slug: "prod".into(),
        file: root.join("deploy.yaml"),
        project: root.clone(),
        config_dir: root,
        yaml,
        endpoint,
        deploy,
    };
    let name = ReleaseName::parse(release).unwrap();
    let mut plan = release_plan(&deployment, &name, b"{}", b"{}", &BTreeMap::new()).unwrap();
    hash(&mut plan, |_, _| {}).unwrap();
    plan
}

#[test]
fn a_new_release_name_alone_keeps_the_fingerprint() {
    for process in [
        "systemd: {}",
        "pm2: { options: { namespace: '{{RELEASE_ID}}' } }",
    ] {
        let one = app_plan(process, "v1.0.0-abc1234-20260101.120000");
        let two = app_plan(process, "v1.0.1-def5678-20260202.130000");
        let unit = one
            .entries
            .iter()
            .find(|entry| entry.path.starts_with(".shipdeck/shop-web"))
            .unwrap();
        let other = two
            .entries
            .iter()
            .find(|entry| entry.path == unit.path)
            .unwrap();
        assert_ne!(
            unit.sha256, other.sha256,
            "{process}: the files should differ"
        );
        assert_eq!(fingerprint(&one), fingerprint(&two), "{process}");
    }
}

#[test]
fn a_changed_systemd_option_changes_the_fingerprint() {
    let release = "v1.0.0-abc1234-20260101.120000";
    let one = app_plan("systemd: { options: { Service: { Nice: 5 } } }", release);
    let two = app_plan("systemd: { options: { Service: { Nice: 6 } } }", release);
    assert_ne!(fingerprint(&one), fingerprint(&two));
}
