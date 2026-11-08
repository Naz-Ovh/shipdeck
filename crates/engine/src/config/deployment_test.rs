use super::*;

#[test]
fn loads_monorepo_without_making_a_single_service_config() {
    let folder = std::env::temp_dir().join(format!("shipdeck-deployment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("shipdeck.yaml"), "name: product\nssh: {host: example.com, user: deploy}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {start: 'node server.js'}}\n").unwrap();
    let loaded = Deployment::load(&folder).unwrap();
    assert!(
        matches!(loaded, Deployment::Monorepo(deployment) if deployment.config.name == "product")
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn keeps_the_existing_single_service_config() {
    let folder =
        std::env::temp_dir().join(format!("shipdeck-single-deployment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("shipdeck.yaml"), "name: api\ndir: /opt/api\nssh: {host: example.com, user: deploy}\nservice: {name: api, start: 'node server.js'}\npipeline: {pack: {files: {include: {project: [server.js]}}}}\n").unwrap();
    assert!(matches!(
        Deployment::load(&folder).unwrap(),
        Deployment::Single(_)
    ));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn loads_a_single_deployable_from_a_yml_file() {
    let folder =
        std::env::temp_dir().join(format!("shipdeck-yml-deployment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("shipdeck.yml"),
        "name: api\ndir: /opt/api\nssh: {host: example.com, user: deploy}\nservice: {name: api, start: 'node server.js'}\npipeline: {pack: {files: {include: {project: [server.js]}}}}\n",
    )
    .unwrap();
    assert!(matches!(
        Deployment::load(&folder).unwrap(),
        Deployment::Single(_)
    ));
    std::fs::remove_dir_all(folder).unwrap();
}
