use super::*;
use crate::config::MonorepoDeployment;
use crate::deploy::report::Reporter;

#[tokio::test]
async fn failure_hooks_run_without_ssh_and_keep_scope_and_failed_step() {
    let folder = std::env::temp_dir().join(format!("shipdeck-hook-scopes-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("api")).unwrap();
    let config = crate::config::monorepo::parse(
        "name: product\nssh: {host: 127.0.0.1, user: deploy}\ndir: /opt/product\npipeline:\n  pack: {files: {include: {project: [server.js]}}}\n  failed:\n    run:\n      local: 'echo global:$SHIPDECK_STEP:$SHIPDECK_ERROR >> order'\napps:\n  api:\n    project: api\n    start: node server.js\n    pipeline:\n      failed:\n        run:\n          local: 'echo $SHIPDECK_APP:$SHIPDECK_SERVICE:$SHIPDECK_STEP:$SHIPDECK_ERROR >> ../order'\n  web: {start: node server.js}\n",
    ).unwrap();
    let endpoint = crate::ssh::Endpoint::resolve(&config.ssh).unwrap();
    let deployment = MonorepoDeployment {
        folder: folder.clone(),
        slug: "product".into(),
        config,
        endpoint,
    };
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let report = Reporter::new(1, sender);
    let mut run = Multiple::new(&deployment, &report, false).unwrap();
    run.step = Step::Setup;
    run.failed_hooks("failure").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("order")).unwrap(),
        "global:setup:failure\napi:product-api:setup:failure\n"
    );
    std::fs::remove_dir_all(folder).unwrap();
}
