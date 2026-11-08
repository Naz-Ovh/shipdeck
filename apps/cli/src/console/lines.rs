//! What each event of a run becomes: one line for a person.

use std::time::{Duration, Instant};

use shipdeck_engine::human;
use shipdeck_protocol::message::common::{LogKind, Outcome, StepState, Unit};
use shipdeck_protocol::message::event::{Event, OpProgress, OpRelease, OpStep};

use super::paint::Paint;

/// How often a long step reports how far it got.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// The last line of an operation: what is live now. The error itself is
/// already in the log right above it.
#[must_use]
pub(crate) fn summary(paint: Paint, name: &str, outcome: &Outcome, took: Duration) -> String {
    match outcome {
        Outcome::Succeeded { release } => paint.success(&format!(
            "✔ {name}: {release} is live ({})",
            human::duration(took)
        )),
        Outcome::Failed { .. } => paint.error(&format!(
            "✘ {name}: failed; the live release was not touched"
        )),
        Outcome::RolledBack { to, .. } => paint.warn(&format!(
            "▲ {name}: the new release failed; {to} is live again"
        )),
        Outcome::Kept { release, .. } => {
            paint.warn(&format!("▲ {name}: {release} failed and stays live"))
        }
        Outcome::RollbackFailed { rollback_error, .. } => paint.error(&format!(
            "✘ {name}: rollback failed: {rollback_error}; check the server now"
        )),
        Outcome::Unchanged { live } => paint.warn(&format!(
            "▲ {name}: nothing changed since {live}, so nothing was deployed; \
             --allow-unchanged deploys it anyway"
        )),
    }
}

/// Turns operation events into lines: logs as they come, each step once it
/// ends, and progress every few seconds while a long step runs.
pub(crate) struct Lines {
    steps: Vec<String>,
    paint: Paint,
    /// After a failure the remaining steps are all skipped; no need to list them.
    failed: bool,
    /// When progress was last printed, or the running step started.
    progress_at: Instant,
}

impl Lines {
    #[must_use]
    pub(crate) fn new(steps: &[String], paint: Paint, now: Instant) -> Self {
        Self {
            steps: steps.to_vec(),
            paint,
            failed: false,
            progress_at: now,
        }
    }

    pub(crate) fn line(&mut self, event: Event, now: Instant) -> Option<String> {
        match event {
            Event::Log(log) => Some(self.log(log.kind, &log.text)),
            Event::Step(step) => self.step(&step, now),
            Event::Progress(progress) => self.progress(&progress, now),
            Event::Release(OpRelease { release, .. }) => {
                Some(self.paint.info(&format!("ℹ release {release}")))
            }
            Event::Finished(_) | Event::LogsLine(_) | Event::LogsEnded(_) => None,
        }
    }

    fn log(&self, kind: LogKind, text: &str) -> String {
        let paint = self.paint;
        match kind {
            LogKind::Step => paint.step(&format!("━━ {text}")),
            LogKind::Command => paint.command(&format!("› {text}")),
            LogKind::Out | LogKind::Err => format!("  {text}"),
            LogKind::Info => paint.info(&format!("ℹ {text}")),
            LogKind::Success => paint.success(&format!("✔ {text}")),
            LogKind::Warn => paint.warn(&format!("▲ {text}")),
            LogKind::Error => paint.error(&format!("✘ {text}")),
        }
    }

    fn step(&mut self, step: &OpStep, now: Instant) -> Option<String> {
        let paint = self.paint;
        let place = usize::try_from(step.step).unwrap_or(usize::MAX);
        let name = self.steps.get(place).map_or("step", String::as_str);
        let detail = step
            .detail
            .as_ref()
            .map(|d| format!("  {d}"))
            .unwrap_or_default();
        match step.state {
            StepState::Pending => None,
            StepState::Running => {
                self.progress_at = now;
                None
            }
            StepState::Done => Some(format!(
                "{} {name}{}",
                paint.success("✔"),
                paint.muted(&detail)
            )),
            StepState::Warned => Some(paint.warn(&format!("▲ {name}{detail}"))),
            // The full error follows as its own log line.
            StepState::Failed => {
                self.failed = true;
                Some(paint.error(&format!("✘ {name}")))
            }
            StepState::Skipped if self.failed => None,
            StepState::Skipped => Some(paint.muted(&format!("– {name}{detail}"))),
        }
    }

    fn progress(&mut self, progress: &OpProgress, now: Instant) -> Option<String> {
        if now.duration_since(self.progress_at) < PROGRESS_EVERY {
            return None;
        }
        self.progress_at = now;
        let amount = amount(progress.done, progress.total, progress.unit);
        Some(self.paint.muted(&format!("… {} {amount}", progress.label)))
    }
}

fn amount(done: u64, total: u64, unit: Unit) -> String {
    let show = |n| match unit {
        Unit::Bytes => human::bytes(n),
        Unit::Items => n.to_string(),
        Unit::Seconds => format!("{n}s"),
    };
    if total == 0 {
        show(done)
    } else {
        format!("{} / {} ({}%)", show(done), show(total), done * 100 / total)
    }
}

#[cfg(test)]
#[path = "lines_test.rs"]
mod tests;
