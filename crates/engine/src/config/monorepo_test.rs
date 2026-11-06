use super::*;

#[test]
fn reads_inline_apps_and_app_file_paths() {
    let config = parse(
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps:\n  api: apps/api.yaml\n  web:\n    start: node server.js\n",
    )
    .unwrap();
    assert!(
        matches!(config.apps.get("api"), Some(AppDefinition::File(path)) if path == "apps/api.yaml")
    );
    assert!(
        matches!(config.apps.get("web"), Some(AppDefinition::Inline(app)) if app.start.as_deref() == Some("node server.js"))
    );
}

#[test]
fn reads_app_files_and_rejects_paths_outside_the_deploy_folder() {
    let folder =
        std::env::temp_dir().join(format!("shipdeck-monorepo-apps-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("apps")).unwrap();
    std::fs::write(folder.join("apps/api.yaml"), "start: ./api\n").unwrap();
    let mut config =
        parse("name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: apps/api.yaml}\n")
            .unwrap();
    config.load_app_files(&folder).unwrap();
    assert!(
        matches!(config.apps.get("api"), Some(AppDefinition::Inline(app)) if app.start.as_deref() == Some("./api"))
    );
    let mut unsafe_config =
        parse("name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: ../api.yaml}\n")
            .unwrap();
    assert!(unsafe_config.load_app_files(&folder).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn rejects_unsafe_names_paths_and_commands() {
    for text in [
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps: {}\n",
        "name: product\nssh: {host: prod}\ndir: ../product\napps: {api: {start: ./api}}\n",
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: {project: ../other, start: ./api}}\n",
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: {start: ''}}\n",
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: {start: ./api, env_file: ../.env}}\n",
        "name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: {start: ./api, env: {'BAD KEY': x}}}\n",
    ] {
        assert!(parse(text).is_err(), "accepted {text}");
    }
}

#[test]
fn app_manager_replaces_the_global_manager() {
    let global: ProcessManager =
        serde_json::from_str(r#"{"name":"pm2","options":{"instances":4}}"#).unwrap();
    let inherited: AppConfig = serde_json::from_str(r#"{"start":"node app.js"}"#).unwrap();
    let overridden: AppConfig =
        serde_json::from_str(r#"{"start":"node app.js","process_manager":{"name":"systemd"}}"#)
            .unwrap();
    assert_eq!(inherited.process_manager(&global).name, Manager::Pm2);
    let selected = overridden.process_manager(&global);
    assert_eq!(selected.name, Manager::Systemd);
    assert!(selected.options.is_empty());
}

#[test]
fn app_pipeline_overrides_only_the_fields_it_names() {
    let config = parse(
        "name: product\nssh: {host: prod}\ndir: /opt/product\npipeline:\n  health: {timeout_secs: 60, url: 'http://localhost:3000/health'}\n  build: {run: {local: 'npm run build'}}\napps:\n  web:\n    start: node server.js\n    pipeline:\n      health: {url: 'http://localhost:3001/health'}\n",
    )
    .unwrap();
    let Some(AppDefinition::Inline(app)) = config.apps.get("web") else {
        panic!("web is not an inline app");
    };
    let pipeline = app.pipeline(&config.pipeline).unwrap();
    assert_eq!(pipeline.health.timeout_secs, 60);
    assert_eq!(
        pipeline.health.url.as_deref(),
        Some("http://localhost:3001/health")
    );
    assert_eq!(pipeline.build.run.local.0, ["npm run build"]);
}

#[test]
fn release_metadata_preserves_app_settings_and_rejects_unresolved_files() {
    let config = parse("name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [api]}}}}\napps: {api: {start: ./api, process_manager: {name: pm2}}}\n").unwrap();
    let bytes = config.release_metadata().unwrap();
    let saved = MonorepoConfig::from_release_metadata(&bytes).unwrap();
    assert!(
        matches!(saved.apps.get("api"), Some(AppDefinition::Inline(app)) if app.process_manager.as_ref().is_some_and(|manager| manager.name == Manager::Pm2))
    );
    let unresolved =
        parse("name: product\nssh: {host: prod}\ndir: /opt/product\napps: {api: api.yaml}\n")
            .unwrap();
    assert!(unresolved.release_metadata().is_err());
    assert!(MonorepoConfig::from_release_metadata(b"{} ").is_err());
}

#[test]
fn app_service_uses_the_deploy_name_and_the_selected_manager() {
    let config = parse("name: product\nssh: {host: prod}\ndir: /opt/product\nprocess_manager: {name: pm2, options: {instances: 4}}\napps: {api: {start: 'node api.js', process_manager: {name: systemd}}}\n").unwrap();
    let Some(AppDefinition::Inline(app)) = config.apps.get("api") else {
        panic!("api is not an inline app");
    };
    let service = app.service(&config, "api");
    assert_eq!(service.name, "product-api");
    assert_eq!(service.dir, "/opt/product");
    assert_eq!(service.manager, Manager::Systemd);
    assert!(service.pm2.options.is_empty());
    assert_eq!(service.start.as_deref(), Some("node api.js"));
}

#[test]
fn checks_every_app_with_the_existing_service_rules() {
    let valid = parse("name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {start: 'node api.js'}, web: {start: 'node web.js'}}\n").unwrap();
    assert!(valid.check_apps().is_ok());
    let invalid = parse("name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {start: 'node api.js'}, web: {start: 'node web.js', pipeline: {health: {url: 'invalid'}}}}\n").unwrap();
    let error = invalid.check_apps().unwrap_err().to_string();
    assert!(error.contains("app web"));
    assert!(error.contains("health.url"));
    let duplicate = parse("name: product\nssh: {host: prod}\ndir: /opt/product\nprocess_manager: {name: pm2, app: shared}\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {start: 'node api.js'}, web: {start: 'node web.js'}}\n").unwrap();
    assert!(
        duplicate
            .check_apps()
            .unwrap_err()
            .to_string()
            .contains("PM2 app name")
    );
}

#[test]
fn resolves_each_app_inside_the_repository() {
    let folder = std::env::temp_dir().join(format!("shipdeck-resolve-apps-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("repo/api")).unwrap();
    let config = parse("name: product\nproject: repo\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {project: api, start: 'node api.js'}}\n").unwrap();
    let apps = config.resolve_apps(&folder).unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(
        apps[0].project,
        folder.join("repo/api").canonicalize().unwrap()
    );
    assert_eq!(apps[0].release_dir, "apps/api");
    assert_eq!(apps[0].service.name, "product-api");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&folder, folder.join("repo/outside")).unwrap();
        let escaped = parse("name: product\nproject: repo\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: {project: outside, start: 'node api.js'}}\n").unwrap();
        assert!(escaped.resolve_apps(&folder).is_err());
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn loads_and_checks_external_app_files() {
    let folder = std::env::temp_dir().join(format!("shipdeck-load-apps-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("apps")).unwrap();
    let path = folder.join("deploy.yaml");
    std::fs::write(&path, "name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [dist]}}}}\napps: {api: apps/api.yaml}\n").unwrap();
    std::fs::write(folder.join("apps/api.yaml"), "start: node app.js\n").unwrap();
    let loaded = MonorepoConfig::load(&folder, &path).unwrap();
    assert!(matches!(
        loaded.apps.get("api"),
        Some(AppDefinition::Inline(_))
    ));
    std::fs::write(
        folder.join("apps/api.yaml"),
        "start: node app.js\npipeline: {health: {url: bad}}\n",
    )
    .unwrap();
    assert!(MonorepoConfig::load(&folder, &path).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}
