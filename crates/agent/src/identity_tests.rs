use super::{current_user, is_root, lookup, safe_user};

#[test]
fn user_names_are_plain() {
    assert!(safe_user("deploy"));
    assert!(safe_user("app.user-1_2"));
    assert!(!safe_user(""));
    assert!(!safe_user("-root"));
    assert!(!safe_user("two words"));
    assert!(!safe_user("root;id"));
}

#[test]
fn the_current_user_can_be_looked_up() {
    let name = current_user().expect("current user");
    let account = lookup(&name).expect("account");
    assert_eq!(account.name, name);
    assert!(account.home.starts_with('/'));
    if is_root() {
        assert_eq!(account.uid, 0);
    }
}

#[test]
fn a_missing_user_is_an_error() {
    let err = lookup("shipdeck-no-such-user").expect_err("missing user");
    assert!(err.to_string().contains("does not exist"), "{err}");
}
