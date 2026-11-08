use super::{apply, apply_tree, operator};
use crate::identity;

#[test]
fn the_operator_is_a_real_account() {
    let account = operator().expect("the current user");
    assert!(identity::safe_user(&account.name));
    assert!(account.home.starts_with('/'));
}

#[test]
fn ownership_does_not_follow_a_symbolic_link() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("shipdeck-owner-{}-{suffix}", std::process::id()));
    let outside = std::env::temp_dir().join(format!(
        "shipdeck-owner-outside-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir(&root).expect("root");
    std::fs::write(root.join("file"), b"x").expect("file");
    std::fs::write(&outside, b"outside").expect("outside");
    std::os::unix::fs::symlink(&outside, root.join("link")).expect("link");
    apply(&root).expect("apply");
    apply_tree(&root).expect("tree");
    let outside_after = std::fs::read(&outside).expect("outside again");
    assert_eq!(outside_after, b"outside");
    std::fs::remove_dir_all(root).expect("cleanup");
    std::fs::remove_file(outside).expect("outside cleanup");
}
