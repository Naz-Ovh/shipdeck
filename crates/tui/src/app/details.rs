//! The release details popup (`D` in Releases): opening it, reading what
//! the release holds from the server, and its keys.

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, DetailsTab, Fetch, Fetched, OpenFile, ReleaseDetails};
use shipdeck_engine::remote::{FileKind, FileText, Kind, ReleaseContents, Server, Source};

impl App {
    /// Opens the popup on the selected release.
    pub(super) fn open_details(&mut self) {
        let Some(view) = self.releases.as_mut() else {
            return;
        };
        let Fetch::Ready(listing) = &view.listing else {
            return;
        };
        let Some(entry) = view.table.selected().and_then(|i| listing.entries.get(i)) else {
            return;
        };
        let source = match entry.kind {
            Kind::Open => Source::Folder,
            Kind::Compressed => Source::Backup,
        };
        let (index, id) = (view.deployable, entry.id.clone());
        view.details = Some(ReleaseDetails {
            entry: entry.clone(),
            tab: DetailsTab::Details,
            source,
            contents: Fetch::Loading,
            selected: 0,
            file: None,
        });
        self.fetch_contents(index, id, source);
    }

    fn fetch_contents(&mut self, index: usize, id: String, source: Source) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .contents(&id, source)
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Contents {
                index,
                id,
                source,
                result,
            });
        });
    }

    /// Reads the selected file, if it is one.
    fn open_file(&mut self) {
        let Some(view) = self.releases.as_mut() else {
            return;
        };
        let index = view.deployable;
        let Some(details) = view.details.as_mut() else {
            return;
        };
        let Fetch::Ready(contents) = &details.contents else {
            return;
        };
        let Some(info) = contents.files.get(details.selected).cloned() else {
            return;
        };
        if info.kind != FileKind::File {
            return;
        }
        let (id, source, path) = (details.entry.id.clone(), details.source, info.path.clone());
        details.file = Some(OpenFile {
            info,
            text: Fetch::Loading,
            scroll: 0,
        });
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .read(&id, source, &path)
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::FileText {
                index,
                id,
                path,
                result,
            });
        });
    }

    pub(super) fn on_contents(
        &mut self,
        index: usize,
        id: &str,
        source: Source,
        result: Result<ReleaseContents, String>,
    ) {
        if let Some(details) = self.details_of(index, id)
            && details.source == source
        {
            details.selected = 0;
            details.contents = match result {
                Ok(contents) => Fetch::Ready(contents),
                Err(err) => Fetch::Error(err),
            };
        }
    }

    pub(super) fn on_file_text(
        &mut self,
        index: usize,
        id: &str,
        path: &str,
        result: Result<FileText, String>,
    ) {
        if let Some(file) = self
            .details_of(index, id)
            .and_then(|details| details.file.as_mut())
            .filter(|file| file.info.path == path)
        {
            file.text = match result {
                Ok(text) => Fetch::Ready(text),
                Err(err) => Fetch::Error(err),
            };
        }
    }

    fn details_of(&mut self, index: usize, id: &str) -> Option<&mut ReleaseDetails> {
        self.releases
            .as_mut()
            .filter(|view| view.deployable == index)?
            .details
            .as_mut()
            .filter(|details| details.entry.id == id)
    }

    pub(super) fn on_details_key(&mut self, key: KeyEvent) {
        let Some(view) = self.releases.as_mut() else {
            return;
        };
        let index = view.deployable;
        let Some(details) = view.details.as_mut() else {
            return;
        };

        if let Some(file) = details.file.as_mut() {
            let last = match &file.text {
                Fetch::Ready(text) => text.text.lines().count().saturating_sub(1),
                _ => 0,
            };
            file.scroll = match key.code {
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => {
                    details.file = None;
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => file.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => file.scroll + 1,
                KeyCode::PageUp => file.scroll.saturating_sub(20),
                KeyCode::PageDown => file.scroll + 20,
                KeyCode::Home | KeyCode::Char('g') => 0,
                KeyCode::End | KeyCode::Char('G') => last,
                _ => file.scroll,
            }
            .min(last);
            return;
        }

        let files = match &details.contents {
            Fetch::Ready(contents) => contents.files.len(),
            _ => 0,
        };
        let last = files.saturating_sub(1);
        let on_files = details.tab == DetailsTab::Files;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => view.details = None,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                details.tab = match details.tab {
                    DetailsTab::Details => DetailsTab::Files,
                    DetailsTab::Files => DetailsTab::Details,
                };
            }
            KeyCode::Char('b') if details.entry.kind == Kind::Open && details.entry.backup => {
                details.source = match details.source {
                    Source::Folder => Source::Backup,
                    Source::Backup => Source::Folder,
                };
                details.contents = Fetch::Loading;
                let (id, source) = (details.entry.id.clone(), details.source);
                self.fetch_contents(index, id, source);
            }
            KeyCode::Char('r') => {
                details.contents = Fetch::Loading;
                let (id, source) = (details.entry.id.clone(), details.source);
                self.fetch_contents(index, id, source);
            }
            KeyCode::Up | KeyCode::Char('k') if on_files => {
                details.selected = details.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if on_files => {
                details.selected = (details.selected + 1).min(last);
            }
            KeyCode::PageUp if on_files => details.selected = details.selected.saturating_sub(20),
            KeyCode::PageDown if on_files => details.selected = (details.selected + 20).min(last),
            KeyCode::Home | KeyCode::Char('g') if on_files => details.selected = 0,
            KeyCode::End | KeyCode::Char('G') if on_files => details.selected = last,
            KeyCode::Enter if on_files => self.open_file(),
            _ => {}
        }
    }
}
