use std::time::{Duration, Instant};

use shipdeck_protocol::message::common::{LogKind, Outcome, StepState, Unit};
use shipdeck_protocol::message::event::{Event, OpLog, OpProgress, OpStep};

use super::{Lines, Paint, summary};

#[test]
fn steps_print_when_they_end() {
    let start = Instant::now();
    let steps = ["Build".to_string(), "Upload".to_string()];
    let mut lines = Lines::new(&steps, Paint::plain(), start);
    let step = |step, state, detail: Option<&str>| {
        Event::Step(OpStep {
            op: 1,
            step,
            state,
            detail: detail.map(str::to_string),
        })
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
    let log = |kind, text: &str| {
        Event::Log(OpLog {
            op: 1,
            kind,
            text: text.into(),
        })
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
    let steps = ["Upload".to_string()];
    let mut lines = Lines::new(&steps, Paint::plain(), start);
    let progress = |done| {
        Event::Progress(OpProgress {
            op: 1,
            step: 0,
            done,
            total: 4 * 1024 * 1024,
            unit: Unit::Bytes,
            label: "uploading".into(),
        })
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
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::Unchanged {
                live: "v0.3.1".into()
            },
            took
        ),
        "▲ my-api: nothing changed since v0.3.1, so nothing was deployed; \
         --allow-unchanged deploys it anyway"
    );
}
