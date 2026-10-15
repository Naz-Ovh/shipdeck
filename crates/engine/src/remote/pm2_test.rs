use super::*;

fn process(name: &str, status: &str, restarts: u64) -> String {
    format!(
        r#"{{"pid":1,"name":"{name}","pm_id":0,"pm2_env":{{"status":"{status}","restart_time":{restarts}}}}}"#
    )
}

fn jlist(processes: &[String]) -> String {
    format!("[{}]", processes.join(","))
}

/// `app_state` for output that is valid JSON.
fn state(jlist: &str, app: &str) -> (String, Option<u64>) {
    app_state(jlist, app).unwrap()
}

#[test]
fn reads_the_app_out_of_jlist() {
    let list = jlist(&[process("web", "online", 2), process("worker", "errored", 9)]);
    assert_eq!(state(&list, "web"), ("active".into(), Some(2)));
    assert_eq!(state(&list, "worker"), ("failed".into(), Some(9)));
    assert_eq!(state(&list, "api"), ("missing".into(), None));
    assert_eq!(state("[]\n", "web"), ("missing".into(), None));
}

#[test]
fn a_cluster_is_up_only_when_every_instance_is() {
    let up = jlist(&[process("web", "online", 1), process("web", "online", 2)]);
    assert_eq!(state(&up, "web"), ("active".into(), Some(3)));
    let starting = jlist(&[process("web", "online", 0), process("web", "launching", 0)]);
    assert_eq!(state(&starting, "web").0, "activating");
    let crashed = jlist(&[process("web", "online", 0), process("web", "errored", 4)]);
    assert_eq!(state(&crashed, "web").0, "failed");
    let stopped = jlist(&[process("web", "stopped", 0)]);
    assert_eq!(state(&stopped, "web").0, "inactive");
}

#[test]
fn output_that_is_not_json_is_an_error_that_shows_it() {
    let noisy = format!(
        ">>>> In-memory PM2 is out-of-date, do:\n{}",
        jlist(&[process("web", "online", 0)])
    );
    let err = app_state(&noisy, "web").unwrap_err();
    assert!(err.contains("did not print JSON"), "{err}");
    assert!(err.contains("In-memory PM2 is out-of-date"), "{err}");
}

#[test]
fn refuses_a_pm2_newer_than_it_knows() {
    assert_eq!(check_version("5.4.3\n").as_deref(), Ok("5.4.3"));
    assert_eq!(
        check_version(&format!("{PM2_NEWEST_MAJOR}.0.1")),
        Ok(format!("{PM2_NEWEST_MAJOR}.0.1"))
    );
    let newer = check_version(&format!("{}.0.0", PM2_NEWEST_MAJOR + 1)).unwrap_err();
    assert!(
        newer.contains("newer than this shipdeck supports"),
        "{newer}"
    );
    for odd in ["", "6", "6.0", "6.0.x", "notice\n6.0.8"] {
        let err = check_version(odd).unwrap_err();
        assert!(err.contains("did not print a version"), "{odd:?}: {err}");
    }
}
