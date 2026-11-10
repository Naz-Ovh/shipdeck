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
fn progress_is_immediate_then_updates_with_rate_and_time_left() {
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
        Some("… Upload: uploading [----------] 1.0 KiB of 4.0 MiB (0%)".into())
    );
    let second = lines
        .line(progress(1024 * 1024), start + Duration::from_secs(6))
        .unwrap();
    assert!(
        second.starts_with("… Upload: uploading [##--------] 1.0 MiB of 4.0 MiB (25%)"),
        "{second}"
    );
    assert!(second.contains("/s"), "{second}");
    assert!(second.contains("remaining"), "{second}");
    assert!(second.contains("left"), "{second}");
    assert_eq!(
        lines.line(
            progress(1024 * 1024 + 100 * 1024),
            start + Duration::from_millis(6500)
        ),
        None
    );
    assert!(
        lines
            .line(
                progress(2 * 1024 * 1024),
                start + Duration::from_millis(7100)
            )
            .is_some()
    );
}

#[test]
fn a_step_prints_its_number_and_survives_a_silent_spell() {
    let start = Instant::now();
    let steps = ["Upload".to_string(), "Health".to_string()];
    let mut lines = Lines::new(&steps, Paint::plain(), start);
    let step = |step, state| {
        Event::Step(OpStep {
            op: 1,
            step,
            state,
            detail: None,
        })
    };
    assert_eq!(lines.line(step(0, StepState::Running), start), None);
    assert_eq!(
        lines.line(
            Event::Log(OpLog {
                op: 1,
                kind: LogKind::Step,
                text: "Upload".into(),
            }),
            start
        ),
        Some("━━ [1/2] Upload".into())
    );
    assert_eq!(
        lines.line(
            Event::Progress(OpProgress {
                op: 1,
                step: 0,
                done: 1024,
                total: 4096,
                unit: Unit::Bytes,
                label: "uploading".into(),
            }),
            start + Duration::from_secs(1)
        ),
        Some("… Upload: uploading [##--------] 1.0 KiB of 4.0 KiB (25%)".into())
    );
    assert_eq!(lines.heartbeat(start + Duration::from_secs(2)), None);
    let beat = lines.heartbeat(start + Duration::from_secs(7)).unwrap();
    assert!(beat.contains("… Upload: uploading"), "{beat}");
    assert!(beat.contains("so far"), "{beat}");
    assert!(
        lines
            .line(step(0, StepState::Done), start + Duration::from_secs(8))
            .is_some()
    );
    assert_eq!(lines.heartbeat(start + Duration::from_secs(20)), None);
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
