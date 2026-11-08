use super::{pm2_state, safe_service};

#[test]
fn reads_pm2_states_in_systemd_words() {
    let jlist = r#"[
      {"name":"api","pm2_env":{"status":"online"}},
      {"name":"api","pm2_env":{"status":"online"}},
      {"name":"web","pm2_env":{"status":"errored"}}
    ]"#;
    assert_eq!(pm2_state(jlist, "api").expect("state"), "active");
    assert_eq!(pm2_state(jlist, "web").expect("state"), "failed");
    assert_eq!(pm2_state(jlist, "none").expect("state"), "missing");
    assert!(pm2_state("not json", "api").is_err());
}

#[test]
fn refuses_service_names_that_could_be_options() {
    assert!(safe_service("my-api"));
    assert!(safe_service("api@blue"));
    assert!(!safe_service("--all"));
    assert!(!safe_service("api;rm"));
    assert!(!safe_service(""));
}
