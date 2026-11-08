use super::{Action, Installed, Version, choose};

fn version(text: &str) -> Version {
    Version::parse(text).expect("a version")
}

fn installed(text: &str) -> Installed {
    let version = version(text);
    Installed {
        path: format!("/opt/shipdeck/agent/v{text}/agent"),
        version,
        label: text.to_string(),
    }
}

#[test]
fn reads_three_number_versions() {
    assert_eq!(
        version("5.2.1"),
        Version {
            major: 5,
            minor: 2,
            patch: 1
        }
    );
    assert_eq!(version("1.2.3-rc.1"), version("1.2.3"));
    assert!(Version::parse("5.2").is_err());
    assert!(Version::parse("5.2.1.0").is_err());
    assert!(Version::parse("five.2.1").is_err());
    assert!(Version::parse("").is_err());
}

#[test]
fn same_major_different_minor_and_patch_is_used() {
    let cli = version("5.1.0");
    let found = vec![installed("5.2.3")];
    assert_eq!(choose(&cli, &found), Action::Use(found[0].clone()));
}

#[test]
fn exact_version_is_not_required() {
    let cli = version("5.2.3");
    let found = vec![installed("5.1.0")];
    assert_eq!(choose(&cli, &found), Action::Use(found[0].clone()));
}

#[test]
fn a_newer_agent_major_refuses_the_connection() {
    let cli = version("5.1.0");
    let found = vec![installed("6.0.0")];
    assert_eq!(choose(&cli, &found), Action::Refuse(found[0].clone()));
}

#[test]
fn a_newer_major_wins_over_a_same_major_install() {
    let cli = version("5.1.0");
    let newer = installed("6.0.0");
    let found = vec![installed("5.9.9"), newer.clone()];
    assert_eq!(choose(&cli, &found), Action::Refuse(newer));
}

#[test]
fn an_older_major_is_cleared_before_installing() {
    let cli = version("5.1.0");
    let found = vec![installed("4.9.0"), installed("3.0.0")];
    assert_eq!(choose(&cli, &found), Action::Replace);
}

#[test]
fn no_agent_gets_an_install() {
    assert_eq!(choose(&version("5.1.0"), &[]), Action::Install);
}

#[test]
fn the_newest_same_major_install_is_used() {
    let cli = version("5.1.0");
    let newest = installed("5.7.2");
    let found = vec![installed("5.2.0"), newest.clone(), installed("5.4.1")];
    assert_eq!(choose(&cli, &found), Action::Use(newest));
}
