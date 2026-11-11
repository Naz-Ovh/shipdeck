//! How keys change the app: per screen, the command line and the dialogs.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use shipdeck_protocol::message::request::SetupAccess;

use super::{App, Fetch, Modal, Screen, Tone};
use crate::command::{self, CommandLine};
use shipdeck_engine::remote::{Control, Kind};

impl App {
    pub fn on_event(&mut self, event: &Event) {
        let Event::Key(key) = event else {
            return;
        };
        let key = *key;
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
        if self.screen == Screen::Releases
            && self.releases.as_ref().is_some_and(|v| v.details.is_some())
        {
            self.on_details_key(key);
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
        } else if let Some(index) = self.selected().filter(|index| self.usable(*index)) {
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
                if let Some(index) = self.selected().filter(|index| self.usable(*index)) {
                    if self.op_running() {
                        self.notify("An operation is already running", Tone::Warn);
                    } else {
                        self.modals.push_back(Modal::ConfirmDeploy {
                            deployable: index,
                            missing: None,
                        });
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(index) = self.selected().filter(|index| self.usable(*index)) {
                    self.open_releases(index);
                }
            }
            KeyCode::Char('c') => {
                if let Some(index) = self.selected().filter(|index| self.usable(*index)) {
                    self.toggle_connection(index);
                }
            }
            KeyCode::Char('s') => self.refresh_connected(),
            KeyCode::Char('x') => {
                if let Some(index) = self.selected().filter(|index| self.usable(*index)) {
                    self.ask_cleanup(index);
                }
            }
            KeyCode::Char(':') if self.selected().is_some_and(|index| self.usable(index)) => {
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
            return match command {
                command::Command::Logs => self.open_logs(index),
                command::Command::Setup => {
                    if self.setting_up.contains(&index) {
                        self.notify("Guided setup is already running", Tone::Warn);
                    } else {
                        self.modals
                            .push_back(Modal::SetupAccess { deployable: index });
                    }
                }
                _ => {}
            };
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
            KeyCode::Char('D') => self.open_details(),
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
            from_backup: entry.kind == Kind::Compressed,
            current: listing.current.clone(),
        };
        self.modals.push_back(modal);
    }

    pub(super) fn on_modal_key(&mut self, key: KeyEvent) {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match self.modals.front() {
            Some(Modal::Secret { .. }) => self.on_secret_key(key, ctrl_c),
            Some(Modal::TrustHost { .. }) => self.on_trust_host_key(key, ctrl_c),
            Some(Modal::ConfirmDeploy { .. }) => self.on_confirm_deploy_key(key, ctrl_c),
            Some(Modal::ConfirmRollback { .. }) => self.on_confirm_rollback_key(key, ctrl_c),
            Some(Modal::ConfirmControl { .. }) => self.on_confirm_control_key(key, ctrl_c),
            Some(Modal::ConfirmCleanup { .. }) => self.on_confirm_cleanup_key(key, ctrl_c),
            Some(Modal::SetupAccess { .. }) => self.on_setup_access_key(key, ctrl_c),
            Some(Modal::SetupPassword { .. }) => self.on_setup_password_key(key, ctrl_c),
            Some(Modal::AskRollback { .. }) => self.on_ask_rollback_key(key),
            Some(Modal::AskUnchanged { .. }) => self.on_ask_unchanged_key(key, ctrl_c),
            Some(Modal::Notice { .. }) => self.on_notice_key(key, ctrl_c),
            Some(Modal::ConfirmQuit) => self.on_confirm_quit_key(key, ctrl_c),
            None => {}
        }
    }

    /// Reads a password; enter submits it and esc or ctrl+c cancels.
    fn on_secret_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::Secret { input, reply, .. }) = self.modals.front_mut() else {
            return;
        };
        let answer = match key.code {
            KeyCode::Enter => Some(std::mem::take(input)),
            KeyCode::Esc => None,
            _ if ctrl_c => None,
            KeyCode::Backspace => {
                input.pop();
                return;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.clear();
                return;
            }
            KeyCode::Char(c) => {
                input.push(c);
                return;
            }
            _ => return,
        };
        if let Some(reply) = reply.take() {
            let _ = reply.send(answer);
        }
        self.modals.pop_front();
    }

    fn on_trust_host_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let answer = match key.code {
            KeyCode::Char('y' | 'Y') => true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => false,
            _ if ctrl_c => false,
            _ => return,
        };
        let Some(Modal::TrustHost { reply, .. }) = self.modals.front_mut() else {
            return;
        };
        if let Some(reply) = reply.take() {
            let _ = reply.send(answer);
        }
        self.modals.pop_front();
    }

    fn on_confirm_deploy_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::ConfirmDeploy { deployable, .. }) = self.modals.front() else {
            return;
        };
        let index = *deployable;
        match confirmation(key.code, ctrl_c) {
            Some(true) => {
                self.modals.pop_front();
                self.start_deploy(index);
            }
            Some(false) => {
                self.modals.pop_front();
            }
            None => {}
        }
    }

    fn on_confirm_rollback_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::ConfirmRollback {
            deployable, target, ..
        }) = self.modals.front()
        else {
            return;
        };
        let (index, target) = (*deployable, target.clone());
        match confirmation(key.code, ctrl_c) {
            Some(true) => {
                self.modals.pop_front();
                self.start_rollback(index, target);
            }
            Some(false) => {
                self.modals.pop_front();
            }
            None => {}
        }
    }

    fn on_confirm_control_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::ConfirmControl { deployable, action }) = self.modals.front() else {
            return;
        };
        let (index, action) = (*deployable, *action);
        match confirmation(key.code, ctrl_c) {
            Some(true) => {
                self.modals.pop_front();
                if self.op_running_on(index) {
                    self.notify("A deploy started meanwhile; not now", Tone::Warn);
                } else {
                    self.control(index, action);
                }
            }
            Some(false) => {
                self.modals.pop_front();
            }
            None => {}
        }
    }

    fn on_confirm_cleanup_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::ConfirmCleanup {
            deployable,
            artifacts,
        }) = self.modals.front()
        else {
            return;
        };
        let (index, artifacts) = (*deployable, artifacts.clone());
        match confirmation(key.code, ctrl_c) {
            Some(true) => {
                self.modals.pop_front();
                self.cleanup_work(index, artifacts);
            }
            Some(false) => {
                self.modals.pop_front();
            }
            None => {}
        }
    }

    fn on_setup_access_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::SetupAccess { deployable }) = self.modals.front() else {
            return;
        };
        let index = *deployable;
        let access = match key.code {
            KeyCode::Char('f' | 'F') => SetupAccess::Full,
            KeyCode::Char('c' | 'C') => SetupAccess::Commands,
            KeyCode::Esc => {
                self.modals.pop_front();
                return;
            }
            _ if ctrl_c => {
                self.modals.pop_front();
                return;
            }
            _ => return,
        };
        self.modals.pop_front();
        self.modals.push_back(Modal::SetupPassword {
            deployable: index,
            access,
            input: String::new(),
        });
    }

    fn on_setup_password_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let Some(Modal::SetupPassword {
            deployable,
            access,
            input,
        }) = self.modals.front_mut()
        else {
            return;
        };
        let index = *deployable;
        let access = *access;
        let password = match key.code {
            KeyCode::Enter => Some(std::mem::take(input)),
            KeyCode::Esc => {
                self.modals.pop_front();
                return;
            }
            _ if ctrl_c => {
                self.modals.pop_front();
                return;
            }
            KeyCode::Backspace => {
                input.pop();
                return;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.clear();
                return;
            }
            KeyCode::Char(c) => {
                input.push(c);
                return;
            }
            _ => return,
        };
        self.modals.pop_front();
        self.start_setup(index, access, password.filter(|value| !value.is_empty()));
    }

    /// Opens the cleanup question with exactly what the server reported.
    pub(super) fn ask_cleanup(&mut self, index: usize) {
        let Some(report) = self.work.get(index).and_then(Option::as_ref) else {
            self.notify("No inspection of this server yet; press c or s", Tone::Warn);
            return;
        };
        if report.unfinished.is_empty() {
            self.notify("Nothing unfinished to clean up", Tone::Info);
            return;
        }
        if self.op_running() {
            self.notify(
                "An operation is running; clean up after it ends",
                Tone::Warn,
            );
            return;
        }
        self.modals.push_back(Modal::ConfirmCleanup {
            deployable: index,
            artifacts: report.unfinished.clone(),
        });
    }

    /// The deploy question takes y or n and nothing else.
    fn on_ask_rollback_key(&mut self, key: KeyEvent) {
        let answer = match key.code {
            KeyCode::Char('y' | 'Y') => true,
            KeyCode::Char('n' | 'N') => false,
            _ => return,
        };
        if let Some(Modal::AskRollback { question, .. }) = self.modals.pop_front() {
            // The deploy may have stopped waiting a moment ago.
            let _ = question.reply.send(answer);
        }
    }

    fn on_ask_unchanged_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        let answer = match key.code {
            KeyCode::Char('y' | 'Y') => true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc | KeyCode::Enter => false,
            _ if ctrl_c => false,
            _ => return,
        };
        if let Some(Modal::AskUnchanged { question, .. }) = self.modals.pop_front() {
            let _ = question.reply.send(answer);
        }
    }

    fn on_notice_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                self.modals.pop_front();
            }
            _ if ctrl_c => {
                self.modals.pop_front();
            }
            _ => {}
        }
    }

    fn on_confirm_quit_key(&mut self, key: KeyEvent, ctrl_c: bool) {
        match key.code {
            KeyCode::Char('y' | 'Y') => self.quit = true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.modals.pop_front();
            }
            _ if ctrl_c => self.quit = true,
            _ => {}
        }
    }
}

/// Reads a confirmation key: yes for y/enter, no for n/esc or ctrl+c.
fn confirmation(code: KeyCode, ctrl_c: bool) -> Option<bool> {
    match code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(true),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(false),
        _ if ctrl_c => Some(false),
        _ => None,
    }
}
