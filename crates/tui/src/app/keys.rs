//! How keys change the app: per screen, the command line and the dialogs.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Fetch, Modal, Screen, Tone};
use crate::command::{self, CommandLine};
use shipdeck_engine::bundle::files::{self, Source};
use shipdeck_engine::remote::{Control, Kind};

impl App {
    pub fn on_event(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        if !self.modals.is_empty() {
            self.on_modal_key(key);
            return;
        }
        if self.command.is_some() {
            self.on_command_key(key);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return;
        }
        if self.logs.is_some() {
            self.on_logs_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => return self.request_quit(),
            KeyCode::Tab => return self.cycle_screen(1),
            KeyCode::BackTab => return self.cycle_screen(Screen::ALL.len() - 1),
            KeyCode::Char('1') => return self.screen = Screen::Deployables,
            KeyCode::Char('2') => return self.show_operation(),
            KeyCode::Char('3') => return self.show_releases(),
            _ => {}
        }
        match self.screen {
            Screen::Deployables => self.on_deployables_key(key),
            Screen::Operation => self.on_operation_key(key),
            Screen::Releases => self.on_releases_key(key),
        }
    }

    pub(super) fn request_quit(&mut self) {
        if self.op_running() {
            self.modals.push_back(Modal::ConfirmQuit);
        } else {
            self.quit = true;
        }
    }

    pub(super) fn cycle_screen(&mut self, by: usize) {
        let index = Screen::ALL
            .iter()
            .position(|s| *s == self.screen)
            .unwrap_or(0);
        match Screen::ALL[(index + by) % Screen::ALL.len()] {
            Screen::Deployables => self.screen = Screen::Deployables,
            Screen::Operation => self.show_operation(),
            Screen::Releases => self.show_releases(),
        }
    }

    pub(super) fn show_operation(&mut self) {
        if self.op.is_some() {
            self.screen = Screen::Operation;
        } else {
            self.notify(
                "Nothing has run yet. Press ⏎ on a deployable to deploy it",
                Tone::Info,
            );
        }
    }

    pub(super) fn show_releases(&mut self) {
        if self.releases.is_some() {
            self.screen = Screen::Releases;
        } else if let Some(index) = self.selected() {
            self.open_releases(index);
        }
    }

    pub(super) fn on_deployables_key(&mut self, key: KeyEvent) {
        let count = self.deployables.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if count > 0 => {
                let i = self.selected().unwrap_or(0);
                self.list
                    .select(Some(if i == 0 { count - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if count > 0 => {
                let i = self.selected().unwrap_or(0);
                self.list.select(Some((i + 1) % count));
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if let Some(index) = self.selected() {
                    if self.op_running() {
                        self.notify("An operation is already running", Tone::Warn);
                    } else {
                        let missing = files::walk_source(&self.deployables[index], Source::Deploy)
                            .err()
                            .map(|err| format!("{err:#}"));
                        self.modals.push_back(Modal::ConfirmDeploy {
                            deployable: index,
                            missing,
                        });
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(index) = self.selected() {
                    self.open_releases(index);
                }
            }
            KeyCode::Char('c') => {
                if let Some(index) = self.selected() {
                    self.toggle_connection(index);
                }
            }
            KeyCode::Char('s') => self.refresh_connected(),
            KeyCode::Char(':') if self.selected().is_some() => {
                self.command = Some(CommandLine::default());
            }
            _ => {}
        }
    }

    pub(super) fn on_command_key(&mut self, key: KeyEvent) {
        let Some(line) = self.command.as_mut() else {
            return;
        };
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match key.code {
            _ if ctrl_c => self.command = None,
            KeyCode::Esc => self.command = None,
            KeyCode::Enter => {
                let input = std::mem::take(&mut line.input);
                self.command = None;
                self.run_command(&input);
            }
            KeyCode::Tab => line.complete(),
            KeyCode::Backspace => {
                if !line.pop() {
                    self.command = None;
                }
            }
            KeyCode::Char(c) => line.push(c),
            _ => {}
        }
    }

    /// Runs what was typed after `:` for the selected deployable.
    pub(super) fn run_command(&mut self, input: &str) {
        let Some(index) = self.selected() else {
            return;
        };
        if input.trim().is_empty() {
            return;
        }
        let command = match command::parse(input) {
            Ok(command) => command,
            Err(why) => return self.notify(why, Tone::Warn),
        };
        let Some(action) = command.control() else {
            return self.open_logs(index);
        };
        if self.op_running_on(index) {
            let slug = &self.deployables[index].slug;
            self.notify(
                format!("{slug} is being deployed; only :logs works until it ends"),
                Tone::Warn,
            );
            return;
        }
        if matches!(action, Control::Stop | Control::Restart) {
            self.modals.push_back(Modal::ConfirmControl {
                deployable: index,
                action,
            });
        } else {
            self.control(index, action);
        }
    }

    pub(super) fn on_logs_key(&mut self, key: KeyEvent) {
        let Some(logs) = self.logs.as_mut() else {
            return;
        };
        let max = logs.lines.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.logs = None,
            KeyCode::Up | KeyCode::Char('k') => logs.scroll = (logs.scroll + 1).min(max),
            KeyCode::Down | KeyCode::Char('j') => logs.scroll = logs.scroll.saturating_sub(1),
            KeyCode::PageUp => logs.scroll = (logs.scroll + 20).min(max),
            KeyCode::PageDown => logs.scroll = logs.scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => logs.scroll = max,
            KeyCode::End | KeyCode::Char('G') => logs.scroll = 0,
            _ => {}
        }
    }

    pub(super) fn on_operation_key(&mut self, key: KeyEvent) {
        let Some(op) = self.op.as_mut() else {
            return;
        };
        let max = op.logs.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => op.scroll = (op.scroll + 1).min(max),
            KeyCode::Down | KeyCode::Char('j') => op.scroll = op.scroll.saturating_sub(1),
            KeyCode::PageUp => op.scroll = (op.scroll + 20).min(max),
            KeyCode::PageDown => op.scroll = op.scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => op.scroll = max,
            KeyCode::End | KeyCode::Char('G') => op.scroll = 0,
            KeyCode::Char('r') => {
                let index = op.deployable;
                self.open_releases(index);
            }
            KeyCode::Esc | KeyCode::Backspace => self.screen = Screen::Deployables,
            _ => {}
        }
    }

    pub(super) fn on_releases_key(&mut self, key: KeyEvent) {
        let Some(view) = self.releases.as_mut() else {
            return;
        };
        let rows = match &view.listing {
            Fetch::Ready(listing) => listing.entries.len(),
            _ => 0,
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if rows > 0 => {
                let i = view.table.selected().unwrap_or(0);
                view.table
                    .select(Some(if i == 0 { rows - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if rows > 0 => {
                let i = view.table.selected().unwrap_or(0);
                view.table.select(Some((i + 1) % rows));
            }
            KeyCode::Char('r') => {
                let index = view.deployable;
                self.fetch_releases(index);
            }
            KeyCode::Enter => self.ask_rollback(),
            KeyCode::Esc | KeyCode::Backspace => self.screen = Screen::Deployables,
            _ => {}
        }
    }

    pub(super) fn ask_rollback(&mut self) {
        let Some(view) = self.releases.as_ref() else {
            return;
        };
        let Fetch::Ready(listing) = &view.listing else {
            return;
        };
        let Some(entry) = view.table.selected().and_then(|i| listing.entries.get(i)) else {
            return;
        };
        if entry.current {
            self.notify("That release is already live", Tone::Info);
            return;
        }
        if entry.name.is_none() {
            self.notify(
                "shipdeck did not create that folder, so it will not switch to it",
                Tone::Warn,
            );
            return;
        }
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let modal = Modal::ConfirmRollback {
            deployable: view.deployable,
            target: entry.id.clone(),
            from: entry.backup.filter(|_| entry.kind == Kind::Compressed),
            current: listing.current.clone(),
        };
        self.modals.push_back(modal);
    }

    pub(super) fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modals.front_mut() else {
            return;
        };
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match modal {
            Modal::Secret { input, reply, .. } => match key.code {
                KeyCode::Enter => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(Some(std::mem::take(input)));
                    }
                    self.modals.pop_front();
                }
                KeyCode::Esc => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(None);
                    }
                    self.modals.pop_front();
                }
                _ if ctrl_c => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(None);
                    }
                    self.modals.pop_front();
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear()
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Modal::TrustHost { reply, .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
                    _ if ctrl_c => Some(false),
                    _ => None,
                };
                if let Some(answer) = answer {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(answer);
                    }
                    self.modals.pop_front();
                }
            }
            Modal::ConfirmDeploy { deployable, .. } => {
                let index = *deployable;
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        self.start_deploy(index);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::ConfirmRollback {
                deployable, target, ..
            } => {
                let (index, target) = (*deployable, target.clone());
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        self.start_rollback(index, target);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::AskRollback { .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => true,
                    KeyCode::Char('n') | KeyCode::Char('N') => false,
                    _ => return,
                };
                if let Some(Modal::AskRollback { question, .. }) = self.modals.pop_front() {
                    // The deploy may have stopped waiting a moment ago.
                    let _ = question.reply.send(answer);
                }
            }
            Modal::AskUnchanged { .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => true,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                        false
                    }
                    _ if ctrl_c => false,
                    _ => return,
                };
                if let Some(Modal::AskUnchanged { question, .. }) = self.modals.pop_front() {
                    let _ = question.reply.send(answer);
                }
            }
            Modal::ConfirmControl { deployable, action } => {
                let (index, action) = (*deployable, *action);
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        if self.op_running_on(index) {
                            self.notify("A deploy started meanwhile; not now", Tone::Warn);
                        } else {
                            self.control(index, action);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::Notice { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    self.modals.pop_front();
                }
                _ if ctrl_c => {
                    self.modals.pop_front();
                }
                _ => {}
            },
            Modal::ConfirmQuit => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.quit = true,
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.modals.pop_front();
                }
                _ if ctrl_c => self.quit = true,
                _ => {}
            },
        }
    }
}
