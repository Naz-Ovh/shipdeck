use super::*;

fn target(host: &str, port: Option<u16>, user: Option<&str>) -> SshTarget {
    SshTarget {
        host: host.into(),
        port,
        user: user.map(str::to_string),
    }
}

fn prod() -> HostConfig {
    HostConfig {
        host_name: Some("203.0.113.10".into()),
        port: None,
        user: Some("root".into()),
        identity_file: Some(PathBuf::from("/home/me/.ssh/id_deploy")),
        proxy: None,
    }
}

#[test]
fn alias_fills_in_what_deploy_json_leaves_out() {
    let endpoint = Endpoint::from_config(&target("prod", None, None), prod()).unwrap();
    assert_eq!(endpoint.host, "203.0.113.10");
    assert_eq!(endpoint.port, 22);
    assert_eq!(endpoint.user, "root");
    assert_eq!(
        endpoint.identity,
        Some(PathBuf::from("/home/me/.ssh/id_deploy"))
    );
    assert!(endpoint.is_alias());
    assert_eq!(endpoint.label(), "root@prod");
    assert_eq!(endpoint.address(), "203.0.113.10:22");
}

#[test]
fn deploy_json_wins_over_the_alias() {
    let endpoint =
        Endpoint::from_config(&target("prod", Some(2222), Some("deploy")), prod()).unwrap();
    assert_eq!(endpoint.user, "deploy");
    assert_eq!(endpoint.port, 2222);
    assert_eq!(endpoint.config_user.as_deref(), Some("root"));
    assert_eq!(endpoint.label(), "deploy@prod:2222");
}

#[test]
fn plain_address_without_config() {
    let endpoint = Endpoint::from_config(
        &target("203.0.113.10", None, Some("deploy")),
        HostConfig::default(),
    )
    .unwrap();
    assert_eq!(endpoint.host, "203.0.113.10");
    assert!(!endpoint.is_alias());
    assert_eq!(endpoint.identity, None);
}

#[test]
fn needs_a_user_from_somewhere() {
    let err = Endpoint::from_config(&target("box", None, None), HostConfig::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("no User for box"), "{err}");
}

#[test]
fn refuses_proxies_and_odd_users() {
    let proxied = HostConfig {
        proxy: Some("ProxyJump bastion".into()),
        ..prod()
    };
    assert!(Endpoint::from_config(&target("prod", None, None), proxied).is_err());
    assert!(Endpoint::from_config(&target("prod", None, Some("a b")), prod()).is_err());
}
