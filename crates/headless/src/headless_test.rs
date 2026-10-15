use std::path::PathBuf;
use std::time::{Duration, Instant};

use shipdeck_engine::config::{Deployable, Rollback, parse};
use shipdeck_engine::deploy::report::{LogKind, OpEvent, Outcome, StepState, Unit};
use shipdeck_engine::release::ReleaseName;
use shipdeck_engine::remote::{Archive, Kind, Listing, ReleaseEntry};
use shipdeck_engine::ssh::Endpoint;
use shipdeck_engine::ssh::ssh_config::HostConfig;

use super::{Lines, OnFailure, Paint, exit_code, find, summary, table};

fn deployable(slug: &str, name: &str) -> Deployable {
    let config = parse(&format!(
        "
name: {name}
project: ../../app
ssh: {{ host: 203.0.113.10, user: deploy }}
service: {{ name: {slug}, dir: /opt/{slug}, start: ./server }}
pipeline:
  pack:
    binary: target/release/server
"
    ))
    .unwrap();
    let endpoint = Endpoint::from_config(&config.ssh, HostConfig::default()).unwrap();
    Deployable {
        folder: PathBuf::from(format!("deploy/{slug}")),
        slug: slug.into(),
        config,
        endpoint,
    }
}

#[test]
fn every_outcome_has_its_own_exit_code() {
    let error = String::from("http 502");
    let codes = [
        Outcome::Succeeded {
            release: "r".into(),
        },
        Outcome::Failed {
            error: error.clone(),
        },
        Outcome::RolledBack {
            error: error.clone(),
            to: "r".into(),
        },
        Outcome::Kept {
            release: "r".into(),
            error: error.clone(),
        },
        Outcome::RollbackFailed {
            error: error.clone(),
            rollback_error: error,
        },
    ]
    .map(|outcome| exit_code(&outcome));
    assert_eq!(codes, [0, 1, 3, 4, 5]);
}

#[test]
fn on_failure_overrides_the_config() {
    let mut config = deployable("my-api", "my-api").config;
    OnFailure::Rollback.apply(&mut config);
    assert_eq!(config.pipeline.failed.rollback, Rollback::Auto);
    OnFailure::Keep.apply(&mut config);
    assert_eq!(config.pipeline.failed.rollback, Rollback::Ask);
}

#[test]
fn finds_by_folder_then_by_name() {
    let all = [
        deployable("api-prod", "my-api"),
        deployable("web", "my-web"),
    ];
    assert_eq!(find(&all, "api-prod").unwrap().slug, "api-prod");
    assert_eq!(find(&all, "my-web").unwrap().slug, "web");
    let err = find(&all, "nope").unwrap_err();
    assert!(err.contains("api-prod, web"), "{err}");
}

#[test]
fn a_name_shared_by_two_needs_the_folder() {
    let all = [
        deployable("api-prod", "my-api"),
        deployable("api-staging", "my-api"),
    ];
    let err = find(&all, "my-api").unwrap_err();
    assert!(err.contains("api-prod, api-staging"), "{err}");
    assert_eq!(find(&all, "api-staging").unwrap().slug, "api-staging");
}

#[test]
fn steps_print_when_they_end() {
    let start = Instant::now();
    let mut lines = Lines::new(&["Build", "Upload"], Paint::plain(), start);
    let step = |step, state, detail: Option<&str>| OpEvent::Step {
        step,
        state,
        detail: detail.map(str::to_string),
    };
    assert_eq!(lines.line(step(0, StepState::Running, None), start), None);
    assert_eq!(
        lines.line(step(0, StepState::Done, Some("31.0 MiB")), start),
        Some("✔ Build  31.0 MiB".into())
    );
    assert_eq!(
        lines.line(step(1, StepState::Failed, Some("timed out")), start),
        Some("✘ Upload".into())
    );
    // After a failure the rest are skipped; that is already clear.
    assert_eq!(
        lines.line(step(1, StepState::Skipped, Some("stopped")), start),
        None
    );
}

#[test]
fn logs_keep_their_marker() {
    let now = Instant::now();
    let mut lines = Lines::new(&[], Paint::plain(), now);
    let log = |kind, text: &str| OpEvent::Log {
        kind,
        text: text.into(),
    };
    assert_eq!(
        lines.line(log(LogKind::Command, "cargo build --release"), now),
        Some("› cargo build --release".into())
    );
    assert_eq!(
        lines.line(log(LogKind::Out, "Compiling my-api"), now),
        Some("  Compiling my-api".into())
    );
    assert_eq!(
        lines.line(log(LogKind::Error, "http 502"), now),
        Some("✘ http 502".into())
    );
}

#[test]
fn progress_prints_every_few_seconds() {
    let start = Instant::now();
    let mut lines = Lines::new(&["Upload"], Paint::plain(), start);
    let progress = |done| OpEvent::Progress {
        step: 0,
        done,
        total: 4 * 1024 * 1024,
        unit: Unit::Bytes,
        label: "uploading".into(),
    };
    assert_eq!(
        lines.line(progress(1024), start + Duration::from_secs(1)),
        None
    );
    assert_eq!(
        lines.line(progress(1024 * 1024), start + Duration::from_secs(6)),
        Some("… uploading 1.0 MiB / 4.0 MiB (25%)".into())
    );
    assert_eq!(
        lines.line(progress(2 * 1024 * 1024), start + Duration::from_secs(8)),
        None
    );
}

#[test]
fn the_summary_says_what_is_live() {
    let took = Duration::from_secs(72);
    let paint = Paint::plain();
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::Succeeded {
                release: "v0.3.1".into()
            },
            took
        ),
        "✔ my-api: v0.3.1 is live (1m12s)"
    );
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::RolledBack {
                error: "http 502".into(),
                to: "v0.3.0".into()
            },
            took
        ),
        "▲ my-api: the new release failed; v0.3.0 is live again"
    );
}

#[test]
fn the_release_table_lines_up() {
    let ids = [
        "v0.3.1-ab12cd3-20261014.153012",
        "v0.3.0-9f8e7d6-20261012.091000",
    ];
    let mut entries: Vec<ReleaseEntry> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| ReleaseEntry {
            id: id.to_string(),
            name: ReleaseName::parse(id),
            kind: if i == 0 { Kind::Open } else { Kind::Compressed },
            current: i == 0,
            size_bytes: Some(31 * 1024 * 1024),
            backup: Some(Archive::Backup),
            manifest: None,
        })
        .collect();
    entries.push(ReleaseEntry {
        id: "old".into(),
        name: None,
        kind: Kind::Open,
        current: false,
        size_bytes: None,
        backup: None,
        manifest: None,
    });
    let text = table(&Listing {
        current: Some(ids[0].into()),
        entries,
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("RELEASE  "));
    assert!(lines[1].contains("live"), "{text}");
    assert!(lines[2].contains("compressed"), "{text}");
    assert!(
        lines[3].starts_with("old ") && lines[3].contains("not-ours"),
        "{text}"
    );
    // Every column starts at the same place on every line.
    let col = lines[0].find("VERSION").unwrap();
    let version = ReleaseName::parse(ids[0]).unwrap().version;
    assert!(lines[1][col..].starts_with(&version), "{text}");
}
