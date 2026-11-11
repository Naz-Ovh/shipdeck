use super::*;

fn new(preset: Preset) -> NewDeployable {
    NewDeployable {
        name: "shop".into(),
        preset,
        manager: ManagerKind::Systemd,
        host: "prod".into(),
        user: Some("deploy".into()),
        project: ".".into(),
    }
}

#[test]
fn every_preset_writes_a_valid_deploy_file() {
    for preset in Preset::ALL {
        let yaml = new(preset).yaml();
        crate::config::parse_deploy(&yaml)
            .unwrap_or_else(|problems| panic!("{preset}: {problems:#?}"));
        assert!(
            yaml.starts_with(&crate::config::schema::modeline()),
            "{preset}"
        );
    }
}

#[test]
fn a_pm2_preset_writes_the_pm2_block() {
    let mut deploy = new(Preset::Node);
    deploy.manager = ManagerKind::Pm2;
    let yaml = deploy.yaml();
    assert!(yaml.contains("      pm2: {}"), "{yaml}");
    assert!(!yaml.contains("systemd"), "{yaml}");
}

#[test]
fn init_writes_a_root_file_and_a_deploy_file() {
    let dir = std::env::temp_dir().join(format!("shipdeck-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let deploy = new(Preset::Rust);
    let yaml = check(&dir, &deploy).unwrap();
    let written = write(&dir, &deploy.name, &yaml).unwrap();
    assert_eq!(written.len(), 4, "{written:?}");
    let root = crate::config::Root::read(&dir, None).unwrap();
    assert!(matches!(
        root.deploys.get("shop"),
        Some(crate::config::Deploy::File(file)) if file == &dir.join(DEPLOY_FILE)
    ));
    let loaded = crate::config::load_deploy(&dir, &dir.join(DEPLOY_FILE), Some("shop")).unwrap();
    assert_eq!(loaded.name(), "shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn project_from_is_relative_to_the_folder() {
    let root = std::env::temp_dir().join(format!("shipdeck-init-rel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("web")).unwrap();
    assert_eq!(project_from(&root, &root.join("web")).unwrap(), "web");
    assert_eq!(project_from(&root, &root).unwrap(), ".");
    std::fs::remove_dir_all(&root).unwrap();
}
