use std::path::{Path, PathBuf};

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "shipdeck-env-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn real(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

#[test]
fn the_default_file_is_dot_env_in_the_project_folder() {
    let dir = scratch("default");
    std::fs::write(
        dir.join(".env"),
        "# a comment\nA=1\nB=\"two words\"\nC='x y'\n",
    )
    .unwrap();
    let env = Env::load_over(&dir, None, real(&[])).unwrap();
    assert_eq!(env.get("A"), Some("1"));
    assert_eq!(env.get("B"), Some("two words"));
    assert_eq!(env.get("C"), Some("x y"));
    assert_eq!(env.get("D"), None);
}

#[test]
fn no_dot_env_is_no_error() {
    let dir = scratch("none");
    let env = Env::load_over(&dir, None, real(&[("X", "1")])).unwrap();
    assert_eq!(env.get("X"), Some("1"));
}

#[test]
fn a_named_file_replaces_dot_env() {
    let dir = scratch("named");
    std::fs::write(dir.join(".env"), "A=default\nB=default\n").unwrap();
    std::fs::create_dir_all(dir.join("ci")).unwrap();
    std::fs::write(dir.join("ci/prod.env"), "A=named\n").unwrap();
    let env = Env::load_over(&dir, Some(Path::new("ci/prod.env")), real(&[])).unwrap();
    assert_eq!(env.get("A"), Some("named"));
    assert_eq!(env.get("B"), None, "only one file is read");
}

#[test]
fn a_named_file_may_be_absolute() {
    let dir = scratch("absolute");
    let file = dir.join("elsewhere.env");
    std::fs::write(&file, "A=1\n").unwrap();
    let other = scratch("absolute-folder");
    let env = Env::load_over(&other, Some(&file), real(&[])).unwrap();
    assert_eq!(env.get("A"), Some("1"));
}

#[test]
fn a_named_file_that_is_missing_is_an_error() {
    let dir = scratch("missing");
    let err = Env::load_over(&dir, Some(Path::new("nope.env")), real(&[])).unwrap_err();
    let text = format!("{err:#}");
    assert!(
        text.contains("nope.env") && text.contains("does not exist"),
        "{text}"
    );
}

#[test]
fn the_real_environment_wins_over_the_file() {
    let dir = scratch("wins");
    std::fs::write(dir.join(".env"), "A=file\nB=file\n").unwrap();
    let env = Env::load_over(&dir, None, real(&[("A", "real")])).unwrap();
    assert_eq!(env.get("A"), Some("real"));
    assert_eq!(env.get("B"), Some("file"));
}

#[test]
fn an_empty_real_value_counts_as_not_set() {
    let dir = scratch("empty");
    std::fs::write(dir.join(".env"), "A=file\n").unwrap();
    let env = Env::load_over(&dir, None, real(&[("A", ""), ("B", "")])).unwrap();
    assert_eq!(env.get("A"), Some("file"));
    assert_eq!(env.get("B"), None);
}

#[test]
fn the_first_of_a_repeated_name_stays() {
    let dir = scratch("repeat");
    std::fs::write(dir.join(".env"), "A=first\nA=second\n").unwrap();
    let env = Env::load_over(&dir, None, real(&[])).unwrap();
    assert_eq!(env.get("A"), Some("first"));
}

#[test]
fn a_bad_line_is_named_without_its_text() {
    let dir = scratch("bad");
    std::fs::write(dir.join(".env"), "A=1\nhunter2 is not a line\n").unwrap();
    let err = Env::load_over(&dir, None, real(&[])).unwrap_err();
    let text = format!("{err:#}");
    assert!(
        text.contains(".env") && text.contains("NAME=value"),
        "{text}"
    );
    assert!(
        !text.contains("hunter2"),
        "a line may hold a secret: {text}"
    );
}

#[test]
fn loading_never_changes_the_process_environment() {
    let dir = scratch("process");
    std::fs::write(dir.join(".env"), "SHIPDECK_ENV_TEST_ONLY_NAME=1\n").unwrap();
    Env::load_over(&dir, None, real(&[])).unwrap();
    assert!(std::env::var_os("SHIPDECK_ENV_TEST_ONLY_NAME").is_none());
}
