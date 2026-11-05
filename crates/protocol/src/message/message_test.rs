use serde_json::json;

use super::{Ask, AskRollback, Done, Event, Login, OpLog, Outcome, method};

#[test]
fn a_request_with_nothing_to_report_answers_with_an_empty_object() {
    assert_eq!(serde_json::to_string(&Done {}).unwrap(), "{}");
}

#[test]
fn a_login_is_one_flat_object_with_a_kind() {
    assert_eq!(
        serde_json::to_value(Login::Auto).unwrap(),
        json!({"kind": "auto"})
    );
    assert_eq!(
        serde_json::to_value(Login::Key {
            path: "~/.ssh/id_ed25519".into()
        })
        .unwrap(),
        json!({"kind": "key", "path": "~/.ssh/id_ed25519"})
    );
}

#[test]
fn an_outcome_is_one_flat_object_with_a_kind() {
    let outcome = Outcome::RolledBack {
        error: "health failed".into(),
        to: "v0.3.0".into(),
    };
    assert_eq!(
        serde_json::to_value(outcome).unwrap(),
        json!({"kind": "rolled_back", "error": "health failed", "to": "v0.3.0"})
    );
}

#[test]
fn an_event_is_read_by_its_method() {
    let event = Event::from_wire(
        method::OP_LOG,
        json!({"op": 2, "kind": "warn", "text": "slow"}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        event,
        Event::Log(OpLog {
            op: 2,
            kind: super::LogKind::Warn,
            text: "slow".into(),
        })
    );
    assert!(Event::from_wire("op/tables", json!({})).unwrap().is_none());
}

#[test]
fn a_question_is_read_by_its_method() {
    let ask = Ask::from_wire(
        method::ASK_ROLLBACK,
        json!({
            "op": 1,
            "release": "v0.4.0",
            "previous": "v0.3.0",
            "step": "health",
            "error": "no answer",
            "seconds": 30
        }),
    )
    .unwrap()
    .unwrap();
    assert!(matches!(ask, Ask::Rollback(AskRollback { op: 1, .. })));
    assert!(Ask::from_wire("ask/coffee", json!({})).unwrap().is_none());
}

#[test]
fn params_that_do_not_fit_are_an_error_not_a_silent_default() {
    assert!(Event::from_wire(method::OP_STEP, json!({"op": 1})).is_err());
    assert!(Ask::from_wire(method::ASK_SECRET, json!({"title": "x"})).is_err());
}
