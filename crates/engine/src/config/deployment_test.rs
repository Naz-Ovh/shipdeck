use super::*;

#[test]
fn loads_monorepo_without_making_a_single_service_config() {
    let folder = std::env::temp_dir().join(format!("shipdeck-deployment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("deploy.yaml"), "name: product\nssh: {host: example.com, user: deploy}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {start: 'node server.js'}}\n").unwrap();
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
    std::fs::write(folder.join("deploy.yaml"), "name: api\nssh: {host: example.com, user: deploy}\nservice: {name: api, dir: /opt/api, start: 'node server.js'}\npipeline: {pack: {files: {include: {project: [server.js]}}}}\n").unwrap();
    assert!(matches!(
        Deployment::load(&folder).unwrap(),
        Deployment::Single(_)
    ));
    std::fs::remove_dir_all(folder).unwrap();
}
