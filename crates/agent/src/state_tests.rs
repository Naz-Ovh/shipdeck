use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{AgentEventAppend, AgentEventsAfter};

use super::Store;

#[test]
fn events_survive_reopen_and_concurrent_writers() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("shipdeck-state-{}-{suffix}", std::process::id()));
    std::fs::create_dir(&dir).expect("test directory");
    let db = dir.join("state.db");
    let event = AgentEventAppend {
        project: "/srv/app".into(),
        operation: "v1".into(),
        kind: "start".into(),
        detail: "ready".into(),
    };
    let first = Store::open(&db)
        .expect("open database")
        .append(&event)
        .expect("first event");
    assert_eq!(first.sequence, 1);
    let writers: Vec<_> = (0..8)
        .map(|_| {
            let path = db.clone();
            let event = event.clone();
            std::thread::spawn(move || {
                Store::open(&path)
                    .expect("open writer")
                    .append(&event)
                    .expect("append")
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("writer");
    }
    let store = Store::open(&db).expect("reopen");
    let page = store
        .after(&AgentEventsAfter {
            project: None,
            sequence: 1,
            limit: 3,
        })
        .expect("page");
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [2, 3, 4]
    );
    let rest = store
        .after(&AgentEventsAfter {
            project: None,
            sequence: 4,
            limit: 100,
        })
        .expect("remaining events");
    assert_eq!(rest.events.len(), 5);
    assert!(
        store
            .after(&AgentEventsAfter {
                project: None,
                sequence: -1,
                limit: 1
            })
            .is_err()
    );
    drop(store);
    std::fs::remove_dir_all(dir).expect("remove test directory");
}
