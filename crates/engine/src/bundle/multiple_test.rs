use std::collections::HashMap;

use super::*;

#[test]
fn packs_all_process_files_and_fingerprints_them() {
    let folder =
        std::env::temp_dir().join(format!("shipdeck-many-services-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("server"), "server").unwrap();
    let yaml = "name: demo\nssh: {host: example.com, user: deploy}\nservice: {name: demo, dir: /opt/demo, start: ./server}\npipeline: {pack: {files: {include: {project: [server]}}}}\n";
    let deployable =
        Deployable::from_text(&folder, &folder.join("deploy.yaml"), yaml.into()).unwrap();
    let api = ServiceFile {
        name: "api.service".into(),
        text: "api".into(),
    };
    let web = ServiceFile {
        name: "web.service".into(),
        text: "web".into(),
    };
    let contents = gather_services(&deployable, &[&api, &web], |_, _| {}).unwrap();
    let release = ReleaseName::new("1.0.0", "test", false, chrono::Utc::now());
    let bundle = pack(
        &contents,
        &Manifest::new(&release, "tester"),
        &HashMap::new(),
        |_, _| {},
    )
    .unwrap();
    assert!(bundle.paths.iter().any(|path| path == "api.service"));
    assert!(bundle.paths.iter().any(|path| path == "web.service"));
    let fingerprint = contents.fingerprint_with(&[&api, &web], yaml);
    assert_eq!(fingerprint, contents.fingerprint_with(&[&web, &api], yaml));
    let changed = ServiceFile {
        name: "web.service".into(),
        text: "changed".into(),
    };
    assert_ne!(
        fingerprint,
        contents.fingerprint_with(&[&api, &changed], yaml)
    );
    assert!(gather_services(&deployable, &[&api, &api], |_, _| {}).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn gathers_files_from_multiple_app_folders_without_collisions() {
    let folder = std::env::temp_dir().join(format!("shipdeck-many-apps-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("api")).unwrap();
    std::fs::create_dir_all(folder.join("web")).unwrap();
    std::fs::write(folder.join("api/server"), "api").unwrap();
    std::fs::write(folder.join("web/server"), "web").unwrap();
    std::fs::write(folder.join("api/release.json"), "app data").unwrap();
    std::fs::write(folder.join("web/release.json"), "app data").unwrap();
    let config = |name: &str| {
        format!(
            "name: {name}\nproject: {name}\nssh: {{host: example.com, user: deploy}}\nservice: {{name: {name}, dir: /opt/demo, start: ./server}}\npipeline: {{pack: {{files: {{include: {{project: [server, '*.json']}}}}}}}}\n"
        )
    };
    let api = Deployable::from_text(&folder, &folder.join("deploy.yaml"), config("api")).unwrap();
    let web = Deployable::from_text(&folder, &folder.join("deploy.yaml"), config("web")).unwrap();
    let api_unit = ServiceFile {
        name: "api.service".into(),
        text: "api".into(),
    };
    let web_unit = ServiceFile {
        name: "web.service".into(),
        text: "web".into(),
    };
    let release = ReleaseName::new("1.0.0", "test", false, chrono::Utc::now());
    let api_project = api.project_dir();
    let web_project = web.project_dir();
    let apps = [
        AppSource {
            project: &api_project,
            deploy_folder: &api.folder,
            pack: &api.config.pipeline.pack,
            service: &api.config.service,
            service_file: Some(&api_unit),
            release_dir: "apps/api",
            excluded_configs: &[],
        },
        AppSource {
            project: &web_project,
            deploy_folder: &web.folder,
            pack: &web.config.pipeline.pack,
            service: &web.config.service,
            service_file: Some(&web_unit),
            release_dir: "apps/web",
            excluded_configs: &[],
        },
    ];
    let contents = gather_apps(&apps, b"{}", |_, _| {}).unwrap();
    let metadata = contents
        .entries
        .iter()
        .find(|entry| entry.path == crate::config::monorepo::RELEASE_METADATA)
        .unwrap();
    assert_eq!(metadata.mode, 0o600);
    assert!(matches!(&metadata.source, Source::Bytes(bytes) if bytes == b"{}"));
    let bundle = pack(
        &contents,
        &Manifest::new(&release, "tester"),
        &HashMap::new(),
        |_, _| {},
    )
    .unwrap();
    for expected in [
        "apps/api/server",
        "apps/web/server",
        "apps/api/release.json",
        "apps/web/release.json",
        "api.service",
        "web.service",
    ] {
        assert!(
            bundle.paths.iter().any(|path| path == expected),
            "missing {expected}"
        );
    }
    let same_folder = [
        AppSource {
            project: &api_project,
            deploy_folder: &api.folder,
            pack: &api.config.pipeline.pack,
            service: &api.config.service,
            service_file: Some(&api_unit),
            release_dir: "apps/shared",
            excluded_configs: &[],
        },
        AppSource {
            project: &web_project,
            deploy_folder: &web.folder,
            pack: &web.config.pipeline.pack,
            service: &web.config.service,
            service_file: Some(&web_unit),
            release_dir: "apps/shared",
            excluded_configs: &[],
        },
    ];
    assert!(gather_apps(&same_folder, b"{}", |_, _| {}).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn renders_process_files_for_app_folders() {
    let folder = std::env::temp_dir().join(format!("shipdeck-app-process-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let config = "name: api\nssh: {host: example.com, user: deploy}\nservice: {name: api, dir: /opt/demo, start: ./server}\npipeline: {pack: {files: {include: {project: [server]}}}}\n";
    let api = Deployable::from_text(&folder, &folder.join("deploy.yaml"), config.into()).unwrap();
    let release = ReleaseName::new("1.0.0", "test", false, chrono::Utc::now());
    let rendered = service_file_in(&api, &release, None, "apps/api")
        .unwrap()
        .unwrap();
    assert!(
        rendered
            .text
            .contains("WorkingDirectory=/opt/demo/current/apps/api")
    );
    assert!(
        rendered
            .text
            .contains("ExecStart=/opt/demo/current/apps/api/server")
    );
    assert!(rendered.text.contains("ReadWritePaths=/opt/demo/shared"));
    assert!(service_file_in(&api, &release, None, "../bad").is_err());
    let pm2_config = config.replace("start: ./server", "manager: pm2, start: node server");
    let pm2_app = Deployable::from_text(&folder, &folder.join("deploy.yaml"), pm2_config).unwrap();
    let pm2_file = service_file_in(&pm2_app, &release, Some("/usr/bin/node"), "apps/web")
        .unwrap()
        .unwrap();
    let pm2: serde_json::Value = serde_json::from_str(&pm2_file.text).unwrap();
    assert_eq!(pm2["apps"][0]["cwd"], "/opt/demo/current/apps/web");
    assert_eq!(pm2["apps"][0]["script"], "server");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn app_config_files_are_not_shipped_from_the_deploy_folder() {
    let root =
        std::env::temp_dir().join(format!("shipdeck-app-config-files-{}", std::process::id()));
    let folder = root.join("deploy");
    std::fs::create_dir_all(folder.join("apps")).unwrap();
    std::fs::create_dir_all(root.join("project")).unwrap();
    std::fs::write(root.join("project/server"), "server").unwrap();
    let config_file = folder.join("apps/api.yaml");
    std::fs::write(&config_file, "private app settings").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&config_file, folder.join("config-link")).unwrap();
    let yaml = "name: api\nproject: ../project\nssh: {host: example.com, user: deploy}\nservice: {name: api, dir: /opt/demo, start: ./server}\npipeline: {pack: {files: {include: {project: [server], deploy: ['.']}}}}\n";
    let deployable =
        Deployable::from_text(&folder, &folder.join("deploy.yaml"), yaml.into()).unwrap();
    let excluded = [config_file.canonicalize().unwrap()];
    let project = deployable.project_dir();
    let source = AppSource {
        project: &project,
        deploy_folder: &deployable.folder,
        pack: &deployable.config.pipeline.pack,
        service: &deployable.config.service,
        service_file: None,
        release_dir: "apps/api",
        excluded_configs: &excluded,
    };
    let contents = gather_apps(&[source], b"{}", |_, _| {}).unwrap();
    let release = ReleaseName::new("1.0.0", "test", false, chrono::Utc::now());
    let bundle = pack(
        &contents,
        &Manifest::new(&release, "tester"),
        &HashMap::new(),
        |_, _| {},
    )
    .unwrap();
    assert!(
        !bundle
            .paths
            .iter()
            .any(|path| path.ends_with("api.yaml") || path.ends_with("config-link"))
    );
    std::fs::remove_dir_all(root).unwrap();
}
