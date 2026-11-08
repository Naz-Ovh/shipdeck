//! What each event of a run becomes: one line for a person.

use std::time::{Duration, Instant};

use shipdeck_engine::human;
use shipdeck_protocol::message::common::{LogKind, Outcome, StepState, Unit};
use shipdeck_protocol::message::event::{Event, OpProgress, OpRelease, OpStep};

use super::paint::Paint;

/// How often a long step reports how far it got.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// How long a silent step waits before it says it is still working.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);

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
    /// The running step, while one is running.
    current: Option<usize>,
    /// When the running step started.
    step_started: Instant,
    /// When progress was last printed, or the running step started.
    progress_at: Instant,
    /// Whether the running step printed any progress yet.
    announced: bool,
    /// When any line was last printed.
    last_line: Instant,
    /// The last progress text, for a silent-step heartbeat.
    last_progress: Option<String>,
    /// The first progress point of the running step: when, and how far.
    speed_from: Option<(Instant, u64)>,
}

impl Lines {
    #[must_use]
    pub(crate) fn new(steps: &[String], paint: Paint, now: Instant) -> Self {
        Self {
            steps: steps.to_vec(),
            paint,
            failed: false,
            current: None,
            step_started: now,
            progress_at: now,
            announced: false,
            last_line: now,
            last_progress: None,
            speed_from: None,
        }
    }

    pub(crate) fn line(&mut self, event: Event, now: Instant) -> Option<String> {
        let line = match event {
            Event::Log(log) => Some(self.log(log.kind, &log.text)),
            Event::Step(step) => self.step(&step, now),
            Event::Progress(progress) => self.progress(&progress, now),
            Event::Release(OpRelease { release, .. }) => {
                Some(self.paint.info(&format!("ℹ release {release}")))
            }
            Event::Finished(_) | Event::LogsLine(_) | Event::LogsEnded(_) => None,
        };
        if line.is_some() {
            self.last_line = now;
        }
        line
    }

    /// A line for a step that has printed nothing for a while.
    pub(crate) fn heartbeat(&mut self, now: Instant) -> Option<String> {
        let step = self.current?;
        if now.duration_since(self.last_line) < HEARTBEAT_EVERY {
            return None;
        }
        let name = self.steps.get(step).map_or("step", String::as_str);
        let working = self.last_progress.as_deref().unwrap_or("working");
        self.last_line = now;
        Some(self.paint.muted(&format!(
            "… {name}: {working} · {} so far",
            human::duration(now.duration_since(self.step_started))
        )))
    }

    fn log(&self, kind: LogKind, text: &str) -> String {
        let paint = self.paint;
        match kind {
            LogKind::Step => {
                let count = self.steps.len();
                match self.current {
                    Some(step) => paint.step(&format!("━━ [{}/{count}] {text}", step + 1)),
                    None => paint.step(&format!("━━ {text}")),
                }
            }
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
                self.current = Some(place);
                self.step_started = now;
                self.progress_at = now;
                self.announced = false;
                self.last_progress = None;
                self.speed_from = None;
                None
            }
            StepState::Done => {
                self.current = None;
                Some(format!(
                    "{} {name}{}",
                    paint.success("✔"),
                    paint.muted(&detail)
                ))
            }
            StepState::Warned => {
                self.current = None;
                Some(paint.warn(&format!("▲ {name}{detail}")))
            }
            // The full error follows as its own log line.
            StepState::Failed => {
                self.failed = true;
                self.current = None;
                Some(paint.error(&format!("✘ {name}")))
            }
            StepState::Skipped if self.failed => None,
            StepState::Skipped => {
                self.current = None;
                Some(paint.muted(&format!("– {name}{detail}")))
            }
        }
    }

    fn progress(&mut self, progress: &OpProgress, now: Instant) -> Option<String> {
        let name = usize::try_from(progress.step)
            .ok()
            .and_then(|step| self.steps.get(step).cloned())
            .unwrap_or_else(|| "step".into());
        let text = self.progress_text(progress, now);
        self.last_progress = Some(text.clone());
        let finished = progress.total > 0 && progress.done >= progress.total;
        if !self.announced || finished || now.duration_since(self.progress_at) >= PROGRESS_EVERY {
            self.announced = true;
            self.progress_at = now;
            return Some(self.paint.muted(&format!("… {name}: {text}")));
        }
        None
    }

    /// What a progress event says, with a rate and time left for bytes.
    fn progress_text(&mut self, progress: &OpProgress, now: Instant) -> String {
        let amount = amount(progress.done, progress.total, progress.unit);
        let mut text = format!("{} {amount}", progress.label);
        if progress.unit != Unit::Bytes {
            return text;
        }
        let Some((started, start_done)) = self.speed_from else {
            self.speed_from = Some((now, progress.done));
            return text;
        };
        let seconds = now.duration_since(started).as_secs();
        let done = progress.done.saturating_sub(start_done);
        if seconds > 0 && done > 0 {
            let rate = done / seconds;
            text.push_str(" · ");
            text.push_str(&human::bytes(rate));
            text.push_str("/s");
            if rate > 0 && progress.total > progress.done {
                let left = progress.total.saturating_sub(progress.done) / rate;
                text.push_str(" · ");
                text.push_str(&human::duration(Duration::from_secs(left)));
                text.push_str(" left");
            }
        }
        text
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
        let percent = u128::from(done) * 100 / u128::from(total);
        format!("{} / {} ({percent}%)", show(done), show(total))
    }
}

#[cfg(test)]
#[path = "lines_test.rs"]
mod tests;
