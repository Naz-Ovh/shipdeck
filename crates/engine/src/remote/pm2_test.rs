use super::*;

fn process(name: &str, status: &str, restarts: u64) -> String {
    format!(
        r#"{{"pid":1,"name":"{name}","pm_id":0,"pm2_env":{{"status":"{status}","restart_time":{restarts}}}}}"#
    )
}

fn jlist(processes: &[String]) -> String {
    format!("[{}]", processes.join(","))
}

#[test]
fn reads_the_app_out_of_jlist() {
    let list = jlist(&[process("web", "online", 2), process("worker", "errored", 9)]);
    assert_eq!(app_state(&list, "web"), ("active".into(), Some(2)));
    assert_eq!(app_state(&list, "worker"), ("failed".into(), Some(9)));
    assert_eq!(app_state(&list, "api"), ("missing".into(), None));
}

#[test]
fn a_cluster_is_up_only_when_every_instance_is() {
    let up = jlist(&[process("web", "online", 1), process("web", "online", 2)]);
    assert_eq!(app_state(&up, "web"), ("active".into(), Some(3)));
    let starting = jlist(&[process("web", "online", 0), process("web", "launching", 0)]);
    assert_eq!(app_state(&starting, "web").0, "activating");
    let crashed = jlist(&[process("web", "online", 0), process("web", "errored", 4)]);
    assert_eq!(app_state(&crashed, "web").0, "failed");
    let stopped = jlist(&[process("web", "stopped", 0)]);
    assert_eq!(app_state(&stopped, "web").0, "inactive");
}

#[test]
fn a_notice_before_the_json_is_skipped_and_garbage_says_nothing() {
    let noisy = format!(
        ">>>> In-memory PM2 is out-of-date, do:\n>>>> $ pm2 update\n{}\n",
        jlist(&[process("web", "online", 0)])
    );
    assert_eq!(app_state(&noisy, "web").0, "active");
    assert_eq!(app_state("not json", "web"), (String::new(), None));
}

#[test]
fn refuses_a_pm2_newer_than_it_knows() {
    assert_eq!(check_version("5.4.3\n").as_deref(), Ok("5.4.3"));
    assert_eq!(
        check_version(&format!("{PM2_NEWEST_MAJOR}.0.1")),
        Ok(format!("{PM2_NEWEST_MAJOR}.0.1"))
    );
    assert_eq!(
        check_version("some notice\n6.0.8\n").as_deref(),
        Ok("6.0.8")
    );
    let newer = check_version(&format!("{}.0.0", PM2_NEWEST_MAJOR + 1)).unwrap_err();
    assert!(
        newer.contains("newer than this shipdeck supports"),
        "{newer}"
    );
    assert!(check_version("").unwrap_err().contains("could not read"));
}
