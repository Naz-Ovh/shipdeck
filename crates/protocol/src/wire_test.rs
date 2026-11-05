use serde_json::json;

use super::{ErrorObject, Id, Incoming, Notification, Request, Response, Version, code, to_line};

#[test]
fn a_request_is_one_line_with_its_id_and_params() {
    let request = Request::new(
        Id::Number(1),
        "deploy/start",
        json!({"deployable": "my-api"}),
    );
    let line = to_line(&request).unwrap();
    assert_eq!(
        line,
        r#"{"jsonrpc":"2.0","id":1,"method":"deploy/start","params":{"deployable":"my-api"}}"#
    );
    assert!(!line.contains('\n'));
}

#[test]
fn a_notification_carries_no_id() {
    let line = to_line(&Notification::new("op/finished", json!({"op": 1}))).unwrap();
    assert!(!line.contains("\"id\""));
}

#[test]
fn a_line_is_read_back_as_what_it_is() {
    let request: Incoming =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":3,"method":"status/get","params":{}}"#)
            .unwrap();
    assert!(matches!(request, Incoming::Request(found) if found.id == Id::Number(3)));

    let note: Incoming =
        serde_json::from_str(r#"{"jsonrpc":"2.0","method":"op/log","params":{}}"#).unwrap();
    assert!(matches!(note, Incoming::Notification(_)));

    let answer: Incoming =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":4,"result":{"ok":true}}"#).unwrap();
    assert!(matches!(answer, Incoming::Response(found) if found.id == Id::Number(4)));

    let failure: Incoming = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":5,"error":{"code":-32001,"message":"no such project"}}"#,
    )
    .unwrap();
    assert!(
        matches!(failure, Incoming::Error(found) if found.error.code == code::UNKNOWN_DEPLOYABLE)
    );
}

#[test]
fn the_json_rpc_version_is_checked_both_ways() {
    assert_eq!(serde_json::to_string(&Version).unwrap(), r#""2.0""#);
    let wrong: Result<Version, _> = serde_json::from_str(r#""1.0""#);
    assert!(wrong.is_err());
}

#[test]
fn an_id_may_be_a_number_or_a_text() {
    assert_eq!(Id::Number(7).to_string(), "7");
    assert_eq!(Id::Text("start-1".into()).to_string(), "start-1");
    let text: Id = serde_json::from_str(r#""start-1""#).unwrap();
    assert_eq!(text, Id::Text("start-1".into()));
}

#[test]
fn an_error_shows_its_code() {
    let error = ErrorObject::new(code::BUSY, "a deploy is already running");
    assert_eq!(
        error.to_string(),
        "a deploy is already running (code -32006)"
    );
}

#[test]
fn an_answer_round_trips() {
    let answer = Response::new(Id::Number(1), json!({"deployables": []}));
    let text = to_line(&answer).unwrap();
    let back: Response = serde_json::from_str(&text).unwrap();
    assert_eq!(back, answer);
}
