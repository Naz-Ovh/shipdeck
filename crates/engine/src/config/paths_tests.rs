use std::path::{Path, PathBuf};

use super::*;

#[test]
fn the_three_forms_resolve_from_their_folder() {
    let project = Path::new("/home/me/shop");
    let config = Path::new("/home/me/shop/deploy");
    assert_eq!(local(project, config, "/opt/x"), PathBuf::from("/opt/x"));
    assert_eq!(
        local(project, config, "apps/admin/dist"),
        PathBuf::from("/home/me/shop/apps/admin/dist")
    );
    assert_eq!(
        local(project, config, "./prod/.env"),
        PathBuf::from("/home/me/shop/deploy/prod/.env")
    );
    assert_eq!(
        local(project, config, "../shared"),
        PathBuf::from("/home/me/shop/deploy/../shared")
    );
    assert_eq!(form("/opt/x"), Form::Absolute);
    assert_eq!(form("./x"), Form::Config);
    assert_eq!(form("../x"), Form::Config);
    assert_eq!(form("x/y"), Form::Project);
}

#[test]
fn paths_are_normalized_without_touching_the_disk() {
    assert_eq!(lexical(Path::new("a/./b/../c")), PathBuf::from("a/c"));
    assert_eq!(slash(Path::new("a//b/./c")), "a/b/c");
    assert_eq!(
        relative(
            Path::new("/home/me/shop"),
            Path::new("/home/me/shop/apps/x")
        ),
        Some("apps/x".into())
    );
    assert_eq!(
        relative(Path::new("/home/me/shop"), Path::new("/home/me/other")),
        None
    );
}

#[test]
fn a_release_path_is_relative_and_safe() {
    assert_eq!(release_path("a/./b").unwrap(), "a/b");
    assert_eq!(release_path("./dist").unwrap(), "dist");
    assert!(release_path("/dist").is_err());
    assert!(release_path("../dist").is_err());
    assert!(release_path("a/../b").is_err());
    assert!(release_path(".").is_err());
    assert!(release_path("a\\b").is_err());
    assert_eq!(join("apps/web", "dist"), "apps/web/dist");
    assert!(overlaps("apps/web/dist", "apps/web"));
    assert!(!overlaps("apps/web", "apps/webb"));
}
