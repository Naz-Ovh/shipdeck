use std::path::{Path, PathBuf};

use super::Target;

#[test]
fn a_plain_name_is_a_deploy_name() {
    assert_eq!(
        "production".parse::<Target>(),
        Ok(Target::Name("production".into()))
    );
    assert_eq!(
        "prod-2".parse::<Target>(),
        Ok(Target::Name("prod-2".into()))
    );
    assert_eq!(
        "prod_2".parse::<Target>(),
        Ok(Target::Name("prod_2".into()))
    );
}

#[test]
fn a_yaml_or_yml_ending_is_a_deploy_file() {
    assert_eq!(
        "deploy/prod.yaml".parse::<Target>(),
        Ok(Target::File(PathBuf::from("deploy/prod.yaml")))
    );
    assert_eq!(
        "prod.yml".parse::<Target>(),
        Ok(Target::File(PathBuf::from("prod.yml")))
    );
    assert_eq!(
        "DEPLOY/PROD.YAML".parse::<Target>(),
        Ok(Target::File(PathBuf::from("DEPLOY/PROD.YAML")))
    );
}

#[test]
fn anything_else_is_refused() {
    let err = "./production".parse::<Target>().unwrap_err();
    assert!(err.contains(".yaml or .yml"), "{err}");
    assert!("prod.2".parse::<Target>().is_err());
    assert!("".parse::<Target>().is_err());
}

#[test]
fn a_relative_file_starts_from_the_project_folder() {
    let target = Target::File(PathBuf::from("deploy/prod.yaml"));
    assert_eq!(
        target.resolved(Path::new("/work")),
        Target::File(PathBuf::from("/work/deploy/prod.yaml"))
    );
    let named = Target::Name("prod".into());
    assert_eq!(
        named.resolved(Path::new("/work")),
        Target::Name("prod".into())
    );
}
