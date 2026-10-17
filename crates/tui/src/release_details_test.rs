use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;

use shipdeck_engine::remote::{FileInfo, FileKind, FileText, ReleaseContents, Source};

use crate::app::{DetailsTab, Fetch, Fetched, ReleasesView, Screen};
use crate::tests::{app, listing, press, render};

const OPEN: &str = "v0.3.0-9f8e7d6-20261012.091000";

fn file(path: &str, kind: FileKind, size: u64, target: Option<&str>) -> FileInfo {
    let mode = match kind {
        FileKind::Dir => "drwxr-xr-x",
        FileKind::Link => "lrwxrwxrwx",
        FileKind::File => "-rw-r--r--",
    };
    FileInfo {
        path: path.into(),
        kind,
        mode: mode.into(),
        size,
        target: target.map(str::to_string),
    }
}

#[tokio::test]
async fn shift_d_shows_a_release_its_files_and_one_file() {
    let mut app = app();
    app.releases = Some(ReleasesView {
        deployable: 0,
        listing: Fetch::Ready(listing()),
        table: TableState::default().with_selected(Some(1)),
        details: None,
    });
    app.screen = Screen::Releases;
    app.on_event(Event::Key(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::SHIFT,
    )));
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert_eq!(details.entry.id, OPEN);
    assert_eq!(details.source, Source::Folder);

    app.on_fetched(Fetched::Contents {
        index: 0,
        id: OPEN.into(),
        source: Source::Folder,
        result: Ok(ReleaseContents {
            files: vec![
                file("dist", FileKind::Dir, 4096, None),
                file("dist/main.js", FileKind::File, 48_213, None),
                file(
                    "node_modules/.bin/tool",
                    FileKind::Link,
                    0,
                    Some("../tool/cli.js"),
                ),
                file("release.json", FileKind::File, 268, None),
            ],
            manifest: None,
        }),
    });
    for (width, height) in [(100, 30), (60, 18)] {
        render(&mut app, width, height, "release-details");
    }

    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Down);
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert_eq!((details.tab, details.selected), (DetailsTab::Files, 1));
    render(&mut app, 100, 30, "release-files");

    press(&mut app, KeyCode::Enter);
    app.on_fetched(Fetched::FileText {
        index: 0,
        id: OPEN.into(),
        path: "dist/main.js".into(),
        result: Ok(FileText {
            text: "const port = 3000;\n\tconsole.log(`listening on ${port}`);\n".into(),
            binary: false,
        }),
    });
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert!(matches!(
        details.file.as_ref().map(|f| &f.text),
        Some(Fetch::Ready(_))
    ));
    render(&mut app, 100, 30, "release-file");

    // Esc steps back: the file, then the popup.
    press(&mut app, KeyCode::Esc);
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert!(details.file.is_none());
    press(&mut app, KeyCode::Esc);
    assert!(app.releases.as_ref().unwrap().details.is_none());
    assert_eq!(app.screen, Screen::Releases);
}

#[tokio::test]
async fn b_switches_an_open_release_between_folder_and_backup() {
    let mut app = app();
    app.releases = Some(ReleasesView {
        deployable: 0,
        listing: Fetch::Ready(listing()),
        table: TableState::default().with_selected(Some(1)),
        details: None,
    });
    app.screen = Screen::Releases;
    app.on_event(Event::Key(KeyEvent::new(
        KeyCode::Char('D'),
        KeyModifiers::SHIFT,
    )));
    press(&mut app, KeyCode::Char('b'));
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert_eq!(details.source, Source::Backup);
    assert!(matches!(details.contents, Fetch::Loading));

    // A late answer for the folder is dropped.
    app.on_fetched(Fetched::Contents {
        index: 0,
        id: OPEN.into(),
        source: Source::Folder,
        result: Ok(ReleaseContents::default()),
    });
    let details = app.releases.as_ref().unwrap().details.as_ref().unwrap();
    assert!(matches!(details.contents, Fetch::Loading));
}
