use super::*;

fn home(name: &str, config: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("shipdeck-sshcfg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), config).unwrap();
    home
}

fn lookup_at(home: &Path, host: &str) -> HostConfig {
    lookup_in(&home.join(".ssh/config"), home, host)
}

#[test]
fn resolves_an_alias() {
    let home = home(
        "alias",
        "Include \"missing.d/config\"\n\nHost prod\n  HostName 203.0.113.10\n  User root\n  IdentityFile ~/.ssh/id_deploy\n\nHost staging\n  HostName 198.51.100.20\n  User admin\n",
    );
    let found = lookup_at(&home, "prod");
    assert_eq!(found.host_name.as_deref(), Some("203.0.113.10"));
    assert_eq!(found.user.as_deref(), Some("root"));
    assert_eq!(found.port, None);
    assert_eq!(found.identity_file, Some(home.join(".ssh/id_deploy")));
    assert_eq!(found.proxy, None);
    assert_eq!(lookup_at(&home, "other"), HostConfig::default());
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn first_value_wins_and_wildcards_fill_the_rest() {
    let home = home(
        "first",
        "Host prod-*\n  User deploy\n  Port 2222\n\nHost prod-api\n  HostName 10.0.0.5\n  User root\n\nHost *\n  User fallback\n  IdentityFile %d/.ssh/id_%h\n",
    );
    let found = lookup_at(&home, "prod-api");
    assert_eq!(found.user.as_deref(), Some("deploy"));
    assert_eq!(found.port, Some(2222));
    assert_eq!(found.host_name.as_deref(), Some("10.0.0.5"));
    assert_eq!(found.identity_file, Some(home.join(".ssh/id_10.0.0.5")));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn negated_patterns_and_case_and_equals_syntax() {
    let home = home(
        "negate",
        "Host *.example.com !secret.example.com\n  User web\n\nHOST Box\n  hostname=192.0.2.7\n  PORT = 2200\n",
    );
    assert_eq!(
        lookup_at(&home, "a.example.com").user.as_deref(),
        Some("web")
    );
    assert_eq!(lookup_at(&home, "secret.example.com").user, None);
    let found = lookup_at(&home, "box");
    assert_eq!(found.host_name.as_deref(), Some("192.0.2.7"));
    assert_eq!(found.port, Some(2200));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn reads_includes_and_skips_match_blocks() {
    let home = home(
        "include",
        "Include config.d/*\n\nMatch host prod\n  User nobody\n\nHost prod\n  Port 2022\n",
    );
    std::fs::create_dir_all(home.join(".ssh/config.d")).unwrap();
    std::fs::write(
        home.join(".ssh/config.d/10-prod"),
        "Host prod\n  HostName 203.0.113.4\n  User admin\n",
    )
    .unwrap();
    let found = lookup_at(&home, "prod");
    assert_eq!(found.host_name.as_deref(), Some("203.0.113.4"));
    assert_eq!(found.user.as_deref(), Some("admin"));
    assert_eq!(found.port, Some(2022));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn reports_proxies() {
    let home = home(
        "proxy",
        "Host inner\n  ProxyJump bastion\n\nHost direct\n  ProxyCommand none\n",
    );
    assert_eq!(
        lookup_at(&home, "inner").proxy.as_deref(),
        Some("ProxyJump bastion")
    );
    assert_eq!(lookup_at(&home, "direct").proxy, None);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn missing_config_means_no_settings() {
    let home = std::env::temp_dir().join("shipdeck-sshcfg-none");
    assert_eq!(lookup_at(&home, "prod"), HostConfig::default());
}

#[test]
fn wildcards() {
    assert!(wildcard("*", "anything"));
    assert!(wildcard("prod-?", "prod-1"));
    assert!(!wildcard("prod-?", "prod-12"));
    assert!(wildcard("*.example.com", "a.b.example.com"));
    assert!(!wildcard("*.example.com", "example.com"));
    assert!(wildcard("a*b*c", "aXXbYYc"));
}
