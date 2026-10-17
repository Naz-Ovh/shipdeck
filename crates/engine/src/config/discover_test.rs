use crate::config::fixtures::*;
use crate::config::*;

#[test]
fn discovery_collects_every_problem_and_keeps_the_good_ones() {
    let root = scratch("discover");
    for name in ["api", "twice", "broken"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("api/deploy.yml"), SAMPLE).unwrap();
    std::fs::write(root.join("twice/deploy.yaml"), SAMPLE).unwrap();
    std::fs::write(root.join("twice/deploy.yml"), SAMPLE).unwrap();
    std::fs::write(
        root.join("broken/deploy.yaml"),
        SAMPLE.replace("/opt/demo", "opt/demo"),
    )
    .unwrap();

    let found = discover(&root).unwrap();
    let slugs: Vec<&str> = found.deployables.iter().map(|d| d.slug.as_str()).collect();
    assert_eq!(slugs, ["api"]);
    let problems: Vec<(String, String)> = found
        .problems
        .iter()
        .map(|file| {
            (
                file.path
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                file.problems[0].message.clone(),
            )
        })
        .collect();
    assert_eq!(
        problems,
        [
            (
                "broken".to_string(),
                "`service.dir` must be an absolute path".to_string()
            ),
            (
                "twice".to_string(),
                "this folder has both deploy.yaml and deploy.yml".to_string()
            ),
        ]
    );
    assert!(found.problems[0].source.is_some());
    std::fs::remove_dir_all(root).unwrap();
}
