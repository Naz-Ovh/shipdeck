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
    let loaded = crate::config::load_deploy(
        &dir,
        &dir.join(DEPLOY_FILE),
        Some("shop"),
        crate::config::ServerInputs::default().details(false),
    )
    .unwrap();
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

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("shipdeck-init-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn presets_go_by_their_flag_names() {
    let names: Vec<&str> = Preset::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(
        names,
        [
            "nodejs", "golang", "java", "kotlin", "rust", "dart", "python", "bun"
        ]
    );
    for preset in Preset::ALL {
        assert_eq!(Preset::from_name(preset.name()), Some(preset));
    }
    assert_eq!(Preset::from_name("go"), None);
}

#[test]
fn values_are_quoted_and_a_missing_user_is_only_a_hint() {
    let mut deploy = new(Preset::Rust);
    deploy.host = "prod # main".into();
    deploy.user = None;
    let yaml = deploy.yaml();
    assert!(yaml.contains("    host: \"prod # main\"  "), "{yaml}");
    assert!(yaml.contains("    # user: deploy  "), "{yaml}");
    assert!(!yaml.contains("\n    user:"), "{yaml}");
    let parsed = crate::config::parse_deploy(&yaml).unwrap();
    assert_eq!(parsed.server.ssh.host, "prod # main");
    assert_eq!(parsed.server.ssh.user, None);
    assert_eq!(parsed.server.dir, "/opt/shop");
}

#[test]
fn a_bad_value_is_a_config_problem() {
    let mut deploy = new(Preset::Go);
    deploy.user = Some("de ploy".into());
    let problems = check(Path::new("/srv/deploy"), &deploy).unwrap_err();
    assert!(problems.path.ends_with(DEPLOY_FILE), "{problems:?}");
    assert!(
        problems
            .problems
            .iter()
            .any(|p| p.message == "`server.ssh.user` is not a user name"),
        "{problems:?}"
    );
}

#[test]
fn names_must_be_plain_and_new() {
    let root = temp_root("names");
    assert!(name_problem(&root, "my-api").is_none());
    // Regression: ISSUE-001 — worker@1 passed the name check, then its folder failed.
    for bad in ["", "my api", "../x", ".hidden", "a/b", "worker@1"] {
        assert!(name_problem(&root, bad).is_some(), "{bad:?}");
    }
    std::fs::create_dir_all(root.join("my-api")).unwrap();
    assert!(
        name_problem(&root, "my-api")
            .unwrap()
            .contains("already exists")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn write_keeps_what_is_there_and_never_writes_twice() {
    let root = temp_root("write");
    std::fs::write(root.join(".env"), "PORT=3000\n").unwrap();

    let written = write(&root, "shop", "name: x\n").unwrap();
    assert_eq!(
        written,
        [
            root.join(crate::config::ROOT_NAMES[0]),
            root.join(DEPLOY_FILE),
            root.join(".gitignore")
        ]
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".env")).unwrap(),
        "PORT=3000\n"
    );
    assert!(write(&root, "shop", "name: y\n").is_err());
    assert_eq!(
        std::fs::read_to_string(root.join(DEPLOY_FILE)).unwrap(),
        "name: x\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_walks_up_then_down() {
    assert_eq!(
        relative(
            Path::new("/home/me/web/deploy/my-api"),
            Path::new("/home/me/web")
        ),
        PathBuf::from("../..")
    );
    assert_eq!(
        relative(Path::new("/srv/deploy/my-api"), Path::new("/home/me/web")),
        PathBuf::from("../../../home/me/web")
    );
    assert_eq!(
        relative(Path::new("/srv/x"), Path::new("/srv/x")),
        PathBuf::from(".")
    );
}

#[test]
fn the_fields_most_likely_to_change_say_what_they_do() {
    for preset in Preset::ALL {
        let yaml = new(preset).yaml();
        for key in ["name:", "host:", "user:", "dir:", "source:"] {
            let line = yaml
                .lines()
                .find(|l| l.trim_start().starts_with(key))
                .unwrap_or_else(|| panic!("{preset}: no {key}"));
            assert!(line.contains("  # "), "{preset}: {line}");
        }
    }
}
