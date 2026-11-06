use shipdeck_protocol::message::common as wire;
use shipdeck_protocol::message::event::Event;

use shipdeck_engine::deploy::report::OpEvent;

use super::{listing, status, update};

#[test]
fn a_status_keeps_its_three_parts() {
    let state = status(shipdeck_protocol::message::StatusGetResult {
        dir_exists: true,
        current: Some("v1".into()),
        state: "active".into(),
    });
    assert!(state.dir_exists);
    assert_eq!(state.current.as_deref(), Some("v1"));
    assert_eq!(state.active, "active");
}

#[test]
fn a_listing_reads_its_ids_back_into_names() {
    let id = "v0.3.1-ab12cd3-20261014.153012";
    let found = listing(shipdeck_protocol::message::ReleasesListResult {
        current: Some(id.into()),
        entries: vec![wire::ReleaseInfo {
            id: id.into(),
            name: None,
            kind: wire::ReleaseKind::Open,
            current: true,
            size_bytes: Some(1024),
            backup: false,
            manifest: None,
        }],
    });
    let entry = &found.entries[0];
    assert_eq!(
        entry.name.as_ref().map(|n| n.version.as_str()),
        Some("0.3.1")
    );
    assert_eq!(entry.id, id);
}

#[test]
fn an_event_keeps_its_operation_and_its_step() {
    let event = Event::Step(shipdeck_protocol::message::OpStep {
        op: 4,
        step: 2,
        state: wire::StepState::Failed,
        detail: Some("it broke".into()),
    });
    let update = update(event).unwrap();
    assert_eq!(update.op, 4);
    assert!(matches!(
        update.event,
        OpEvent::Step {
            step: 2,
            state: shipdeck_engine::deploy::report::StepState::Failed,
            ..
        }
    ));
}

#[test]
fn the_log_events_belong_to_the_log_view() {
    let event = Event::LogsEnded(shipdeck_protocol::message::LogsEnded {
        view: 1,
        code: Some(0),
        error: None,
    });
    assert!(update(event).is_none(), "the pump sends these on its own");
}

#[test]
fn a_progress_carries_its_unit() {
    let event = Event::Progress(shipdeck_protocol::message::OpProgress {
        op: 1,
        step: 3,
        done: 1024,
        total: 4096,
        unit: wire::Unit::Bytes,
        label: "uploading".into(),
    });
    let update = update(event).unwrap();
    let OpEvent::Progress { done, unit, .. } = update.event else {
        panic!("expected progress");
    };
    assert_eq!(done, 1024);
    assert_eq!(unit, shipdeck_engine::deploy::report::Unit::Bytes);
}
