use super::*;

fn values<'a>(release: Option<&'a str>, app: Option<&'a str>) -> Values<'a> {
    Values {
        root: "/opt/shop",
        name: "shop",
        host: "prod",
        project: "/home/me/shop",
        config: "/home/me/shop/deploy",
        release,
        version: Some("1.4.0"),
        app: app.map(|name| AppValues {
            name,
            source: "/home/me/shop/apps/api".into(),
            process: Some("shop-api".into()),
        }),
    }
}

#[test]
fn names_are_found_with_spaces_and_left_alone_when_they_are_code() {
    let found = scan("cd {{ PROJECT }} && node -e 'return {{a: 1}}' && echo {{RELEASE_ID}}");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "PROJECT");
    assert_eq!(found[1].name, "RELEASE_ID");
}

#[test]
fn expansion_uses_the_values() {
    let values = values(Some("v1.4.0-abc-20260101.120000"), Some("api"));
    let text = expand(
        "{{ROOT}}/releases/{{RELEASE_ID}}/apps/{{APP}} -> {{APP_SHARED}} ({{VERSION}})",
        &values,
    )
    .unwrap();
    assert_eq!(
        text,
        "/opt/shop/releases/v1.4.0-abc-20260101.120000/apps/api -> /opt/shop/shared/apps/api (1.4.0)"
    );
}

#[test]
fn a_value_that_is_not_there_is_an_error() {
    let values = values(None, None);
    assert!(expand("{{RELEASE}}", &values).is_err());
    assert!(expand("{{APP}}", &values).is_err());
    assert!(expand("{{INSTANCE}}", &values).is_err());
}

#[test]
fn checks_follow_the_scope() {
    assert!(check("{{APP}}", Scope::RELEASE).is_err());
    assert!(check("{{APP}}", Scope::app(true, true)).is_ok());
    assert!(check("{{PROCESS}}", Scope::app(false, true)).is_err());
    assert!(check("{{PROCESS}}", Scope::app(true, true)).is_ok());
    assert!(check("{{RELEASE}}", Scope::EARLY).is_err());
    assert!(check("{{RELEASE}}", Scope::RELEASE).is_ok());
    assert!(check("{{INSTANCE}}", Scope::app(true, true)).is_err());
    assert!(check("{{NOPE}}", Scope::RELEASE).is_err());
    assert!(check("{{a: 1}}", Scope::RELEASE).is_ok());
}

#[test]
fn environment_names_are_the_placeholders() {
    let values = values(Some("v1"), Some("api"));
    let env = env(&values);
    let get = |key: &str| {
        env.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(get("SHIPDECK_ROOT"), Some("/opt/shop"));
    assert_eq!(get("SHIPDECK_RELEASE"), Some("/opt/shop/releases/v1"));
    assert_eq!(
        get("SHIPDECK_APP_CURRENT"),
        Some("/opt/shop/current/apps/api")
    );
    assert_eq!(get("SHIPDECK_PROCESS"), Some("shop-api"));
}
