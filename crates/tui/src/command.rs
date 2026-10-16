//! The `:` command line on the Deployables screen, as in vim: `:logs`,
//! `:restart`, `:stop` and `:start` for the selected deployable's service.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use shipdeck_engine::remote::Control;

use super::theme::{self, ACCENT};

/// What the command line runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Logs,
    Restart,
    Stop,
    Start,
}

impl Command {
    pub const ALL: [Command; 4] = [Self::Logs, Self::Restart, Self::Stop, Self::Start];

    pub fn name(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Restart => "restart",
            Self::Stop => "stop",
            Self::Start => "start",
        }
    }

    /// What it does to the service; `None` for `:logs`, which only reads.
    pub fn control(self) -> Option<Control> {
        match self {
            Self::Logs => None,
            Self::Restart => Some(Control::Restart),
            Self::Stop => Some(Control::Stop),
            Self::Start => Some(Control::Start),
        }
    }
}

/// Reads what was typed after the `:`.
pub fn parse(input: &str) -> Result<Command, String> {
    let name = input.trim();
    Command::ALL
        .into_iter()
        .find(|command| command.name() == name)
        .ok_or_else(|| format!("Not a command: :{name}. Try {}", known()))
}

/// `:logs, :restart, :stop or :start`
fn known() -> String {
    let names: Vec<String> = Command::ALL
        .iter()
        .map(|command| format!(":{}", command.name()))
        .collect();
    let (last, rest) = names.split_last().expect("there are commands");
    format!("{} or {last}", rest.join(", "))
}

/// The command line while it is open.
#[derive(Debug, Default)]
pub struct CommandLine {
    pub input: String,
    /// What was typed before Tab started going through the matches.
    typed: Option<String>,
}

impl CommandLine {
    pub fn push(&mut self, c: char) {
        self.input.push(c);
        self.typed = None;
    }

    /// Removes the last character; `false` when there was none, which
    /// closes the line, as in vim.
    pub fn pop(&mut self) -> bool {
        self.typed = None;
        self.input.pop().is_some()
    }

    /// Completes to the next command starting with what was typed; Tab
    /// again goes on to the one after.
    pub fn complete(&mut self) {
        let typed = self
            .typed
            .get_or_insert_with(|| self.input.trim().to_string());
        let matches: Vec<&str> = Command::ALL
            .iter()
            .map(|command| command.name())
            .filter(|name| name.starts_with(typed.as_str()))
            .collect();
        if matches.is_empty() {
            return;
        }
        let next = match matches.iter().position(|name| *name == self.input) {
            Some(at) => (at + 1) % matches.len(),
            None => 0,
        };
        self.input = matches[next].to_string();
    }
}

/// The line at the bottom: `:` and what is typed, with the cursor.
pub fn draw(frame: &mut Frame, area: Rect, line: &CommandLine) {
    frame.render_widget(
        Line::from(vec![
            Span::styled(":", theme::bold(ACCENT)),
            Span::styled(line.input.clone(), theme::text()),
            Span::styled("▏", Style::new().fg(ACCENT)),
        ]),
        area,
    );
}

#[cfg(test)]
#[path = "command_test.rs"]
mod tests;
