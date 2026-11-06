use super::*;
use crate::config::monorepo::parse;

#[test]
fn collects_both_processes_and_their_release_settings() {
    let folder =
        std::env::temp_dir().join(format!("shipdeck-assemble-apps-{}", std::process::id()));
    for app in ["api", "web"] {
        std::fs::create_dir_all(folder.join(app)).unwrap();
        std::fs::write(folder.join(app).join("server.js"), "console.log('ready')").unwrap();
    }
    let config = parse("name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {pack: {files: {include: {project: [server.js]}}}}\napps:\n  api: {project: api, start: 'node server.js'}\n  web: {project: web, start: 'node server.js', process_manager: {name: pm2}}\n").unwrap();
    let apps = config.resolve_apps(&folder).unwrap();
    let facts = Facts {
        arch: "x86_64".into(),
        glibc: None,
        dir_exists: false,
        dir_writable: true,
        free_bytes: None,
        current: None,
        content: None,
        active: String::new(),
        systemd: String::new(),
        pm2_version: None,
        sudo_missing: Vec::new(),
        tools: vec![("node".into(), Some("/usr/bin/node".into()))],
    };
    let release = ReleaseName::new("1.0.0", "test", false, chrono::Utc::now());
    let contents = gather_monorepo(
        &config,
        &folder,
        &apps,
        "deploy",
        &release,
        &facts,
        |_, _| {},
    )
    .unwrap();
    let names: Vec<_> = contents
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    for name in [
        "apps/api/server.js",
        "apps/web/server.js",
        "product-api.service",
        "product-web.pm2.json",
        "shipdeck-monorepo.json",
    ] {
        assert!(names.contains(&name), "missing {name}");
    }
    let fingerprint =
        fingerprint_monorepo(&contents, &config, &folder, &apps, "deploy", &facts).unwrap();
    let later = ReleaseName::new("2.0.0", "later", true, chrono::Utc::now());
    let changed_id =
        gather_monorepo(&config, &folder, &apps, "deploy", &later, &facts, |_, _| {}).unwrap();
    assert_eq!(
        fingerprint,
        fingerprint_monorepo(&changed_id, &config, &folder, &apps, "deploy", &facts).unwrap()
    );
    std::fs::write(
        folder.join(UNIT_TEMPLATE),
        "[Service]\nExecStart={{exec}}\nEnvironment=CHANGED=yes\n",
    )
    .unwrap();
    assert_ne!(
        fingerprint,
        fingerprint_monorepo(&contents, &config, &folder, &apps, "deploy", &facts).unwrap()
    );
    std::fs::remove_dir_all(folder).unwrap();
}
