use super::*;

#[test]
fn reads_the_live_content_fingerprint_from_release_json() {
    let json = r#"{  "id": "v1",  "version": "1.0.0",  "git": "abc1234",  "dirty": false,  "built_at": "t",  "deployed_by": "me",  "content": "sha256:ab"}"#;
    assert_eq!(content(json).as_deref(), Some("sha256:ab"));
    // A release from before fingerprints, none at all, or a damaged file.
    assert_eq!(
        content(&json.replace(r#",  "content": "sha256:ab""#, "")),
        None
    );
    assert_eq!(content(""), None);
    assert_eq!(content("{\"content\": "), None);
}
