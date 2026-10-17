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
fn reads_any_pm2_version() {
    for (version, major) in [
        ("2.10.4", 2),
        ("5.4.3", 5),
        ("6.0.14", 6),
        ("7.0.4", 7),
        ("8.0.0", 8),
    ] {
        assert_eq!(
            parse_version(&format!("{version}\n")),
            Ok((version.to_string(), major))
        );
    }
    for odd in ["", "6", "6.0", "6.0.x", "notice\n6.0.8"] {
        let err = parse_version(odd).unwrap_err();
        assert!(err.contains("did not print a version"), "{odd:?}: {err}");
    }
}
