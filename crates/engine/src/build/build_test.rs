use super::*;

#[test]
fn parses_cargo_progress_with_crate_names() {
    let line = "    Building [=======================> ] 283/286: aws-lc-rs, russh        ";
    assert_eq!(
        cargo_progress(line),
        Some((283, 286, "aws-lc-rs, russh".to_string()))
    );
}

#[test]
fn parses_cargo_progress_without_crate_names() {
    assert_eq!(
        cargo_progress("Building [====>   ] 12/40"),
        Some((12, 40, String::new()))
    );
}

#[test]
fn ignores_ordinary_lines() {
    assert_eq!(cargo_progress("   Compiling russh v0.64.1"), None);
    assert_eq!(cargo_progress("Building the thing"), None);
}

/// The same shell line works on every OS: on Windows it runs in Git's sh.
#[tokio::test]
async fn a_local_command_is_one_sh_line() {
    let dir = std::env::temp_dir().join(format!("shipdeck-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let report = Reporter::new(1, tx);
    let env = [("SHIPDECK_VERSION", "1.4.0".to_string())];

    let line = "echo \"v$SHIPDECK_VERSION\" | tr v V > a.txt && cp a.txt b.txt && mkdir -p out/x";
    run_local(line, &dir, &env, &report, 0).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("b.txt")).unwrap(),
        "V1.4.0\n"
    );
    assert!(dir.join("out/x").is_dir());

    let err = run_local("exit 3", &dir, &env, &report, 0)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "`exit 3` exited with code 3");
    std::fs::remove_dir_all(dir).unwrap();
}
