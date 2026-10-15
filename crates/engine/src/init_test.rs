use super::*;

fn new(preset: Preset, manager: Manager) -> NewDeployable {
    NewDeployable {
        name: "my-api".into(),
        preset,
        manager,
        host: "203.0.113.10".into(),
        user: Some("deploy".into()),
        project: "../..".into(),
    }
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("shipdeck-init-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn every_preset_passes_the_launch_checks_with_either_manager() {
    let root = Path::new("/srv/deploy");
    for preset in Preset::ALL {
        for manager in [Manager::Systemd, Manager::Pm2] {
            let new = new(preset, manager);
            let yaml = check(root, &new)
                .unwrap_or_else(|problems| panic!("{preset} {manager:?}: {problems:?}"));
            assert!(yaml.starts_with(&schema::modeline()), "{preset}");
            assert_eq!(yaml.contains("manager: pm2"), manager == Manager::Pm2);
        }
    }
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
fn values_are_quoted_and_a_missing_user_is_left_out() {
    let mut new = new(Preset::Rust, Manager::Systemd);
    new.host = "prod # main".into();
    new.user = None;
    let yaml = new.yaml();
    assert!(yaml.contains("  host: \"prod # main\"\n"), "{yaml}");
    assert!(!yaml.contains("user:"), "{yaml}");
    let config = crate::config::parse(&{
        let mut with_user = new.clone();
        with_user.user = Some("deploy".into());
        with_user.yaml()
    })
    .unwrap();
    assert_eq!(config.ssh.host, "prod # main");
    assert_eq!(config.service.dir, "/opt/my-api");
    assert_eq!(
        config.pipeline.pack.binary.as_deref(),
        Some("target/release/my-api")
    );
}

#[test]
fn a_bad_value_is_a_config_problem() {
    let mut new = new(Preset::Go, Manager::Systemd);
    new.user = Some("de ploy".into());
    let problems = check(Path::new("/srv/deploy"), &new).unwrap_err();
    assert!(problems.path.ends_with("my-api/deploy.yaml"));
    assert!(
        problems
            .problems
            .iter()
            .any(|p| p.message.contains("ssh.user")),
        "{problems:?}"
    );
}

#[test]
fn names_must_be_unit_names_and_new() {
    let root = temp_root("names");
    assert!(name_problem(&root, "my-api").is_none());
    for bad in ["", "my api", "../x", ".hidden", "a/b"] {
        assert!(name_problem(&root, bad).is_some(), "{bad:?}");
    }
    std::fs::create_dir_all(root.join("my-api")).unwrap();
    std::fs::write(root.join("my-api/deploy.yaml"), "").unwrap();
    assert!(
        name_problem(&root, "my-api")
            .unwrap()
            .contains("already exists")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn writes_the_folder_and_keeps_what_is_there() {
    let root = temp_root("write");
    std::fs::create_dir_all(root.join("my-api")).unwrap();
    std::fs::write(root.join("my-api/.env"), "PORT=3000\n").unwrap();

    let written = write(&root, "my-api", "name: x\n").unwrap();
    assert_eq!(
        written,
        [
            root.join("my-api/deploy.yaml"),
            root.join("my-api/.gitignore")
        ]
    );
    assert_eq!(
        std::fs::read_to_string(root.join("my-api/.env")).unwrap(),
        "PORT=3000\n"
    );
    assert!(write(&root, "my-api", "name: y\n").is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("my-api/deploy.yaml")).unwrap(),
        "name: x\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_project_is_relative_to_the_new_folder() {
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

    let root = temp_root("project");
    std::fs::create_dir_all(root.join("deploy")).unwrap();
    assert_eq!(
        project_from(&root.join("deploy"), "my-api", &root).unwrap(),
        "../.."
    );
    assert!(project_from(&root.join("deploy"), "my-api", &root.join("missing")).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
