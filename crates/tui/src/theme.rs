//! Colours, symbols and the small styled pieces every screen shares.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Padding};

use shipdeck_engine::deploy::report::{LogKind, StepState};

pub const ACCENT: Color = Color::Rgb(94, 234, 212);
pub const VIOLET: Color = Color::Rgb(167, 139, 250);
pub const OK: Color = Color::Rgb(74, 222, 128);
pub const WARN: Color = Color::Rgb(251, 191, 36);
pub const ERR: Color = Color::Rgb(248, 113, 113);
pub const INFO: Color = Color::Rgb(96, 165, 250);
pub const TEXT: Color = Color::Rgb(226, 232, 240);
pub const MUTED: Color = Color::Rgb(124, 133, 150);
pub const FAINT: Color = Color::Rgb(71, 85, 105);
pub const SURFACE: Color = Color::Rgb(30, 41, 59);
pub const BORDER: Color = Color::Rgb(51, 65, 85);
pub const INK: Color = Color::Rgb(15, 23, 42);

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(tick: u64) -> &'static str {
    SPINNER[(tick as usize) % SPINNER.len()]
}

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn faint() -> Style {
    Style::new().fg(FAINT)
}

pub fn bold(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

/// A rounded panel; `focused` panels get the accent border.
pub fn panel<'a>(title: impl Into<String>, focused: bool) -> Block<'a> {
    let border = if focused { ACCENT } else { BORDER };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Span::styled(
            format!(" {} ", title.into()),
            bold(if focused { ACCENT } else { TEXT }),
        ))
        .padding(Padding::horizontal(1))
}

/// A key cap for the hint bar and dialogs: ` q `.
pub fn key(label: &str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::new().fg(INK).bg(MUTED).add_modifier(Modifier::BOLD),
    )
}

/// A coloured badge: ` LIVE `.
pub fn badge(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::new().fg(INK).bg(color).add_modifier(Modifier::BOLD),
    )
}

pub fn step_icon(state: StepState, tick: u64) -> Span<'static> {
    match state {
        StepState::Pending => Span::styled("○", faint()),
        StepState::Running => Span::styled(spinner(tick), bold(ACCENT)),
        StepState::Done => Span::styled("✔", bold(OK)),
        StepState::Warned => Span::styled("▲", bold(WARN)),
        StepState::Failed => Span::styled("✘", bold(ERR)),
        StepState::Skipped => Span::styled("–", faint()),
    }
}

/// Colour for `systemctl is-active` output.
pub fn service_color(state: &str) -> Color {
    match state {
        "active" => OK,
        "activating" | "reloading" | "deactivating" => WARN,
        "failed" => ERR,
        _ => MUTED,
    }
}

pub fn log_marker(kind: LogKind) -> (&'static str, Style) {
    match kind {
        LogKind::Step => ("━━", bold(ACCENT)),
        LogKind::Command => ("›", bold(VIOLET)),
        LogKind::Out => (" ", text()),
        LogKind::Err => (" ", Style::new().fg(Color::Rgb(203, 196, 170))),
        LogKind::Info => ("ℹ", Style::new().fg(INFO)),
        LogKind::Success => ("✔", bold(OK)),
        LogKind::Warn => ("▲", bold(WARN)),
        LogKind::Error => ("✘", bold(ERR)),
    }
}

pub fn log_style(kind: LogKind) -> Style {
    match kind {
        LogKind::Step => bold(ACCENT),
        LogKind::Command => Style::new().fg(VIOLET),
        LogKind::Out => text(),
        LogKind::Err => Style::new().fg(Color::Rgb(203, 196, 170)),
        LogKind::Info => Style::new().fg(INFO),
        LogKind::Success => bold(OK),
        LogKind::Warn => Style::new().fg(WARN),
        LogKind::Error => bold(ERR),
    }
}
