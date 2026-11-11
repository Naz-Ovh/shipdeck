use super::*;

fn target(host: &str, port: Option<Port>, user: Option<&str>) -> SshTarget {
    SshTarget {
        host: host.into(),
        port,
        user: user.map(str::to_string),
        shell_env: true,
    }
}

fn text(written: &str) -> Port {
    Port::Text(written.into())
}

fn inputs(vars: &[(&str, &str)], flags: SshOverride) -> ServerInputs {
    ServerInputs::new(Env::from_pairs(vars.iter().copied()), flags, Vec::new())
}

fn flag_host(host: &str) -> SshOverride {
    SshOverride {
        host: Some(host.into()),
        ..SshOverride::default()
    }
}

fn apply(
    inputs: &ServerInputs,
    overridden: bool,
    target: &SshTarget,
) -> Result<Applied, Vec<Failure>> {
    inputs.details(overridden).apply(target, "shop")
}

#[test]
fn a_file_with_no_variables_is_left_as_it_is() {
    let given = inputs(&[], SshOverride::default());
    let done = apply(
        &given,
        false,
        &target("example.com", Some(Port::Number(2222)), Some("deploy")),
    )
    .unwrap();
    assert_eq!(
        done.ssh,
        target("example.com", Some(Port::Number(2222)), Some("deploy"))
    );
    assert_eq!(done.from, Vec::<String>::new());
}

#[test]
fn one_variable_fills_host_port_and_user() {
    let given = inputs(
        &[("H", "10.0.0.5"), ("P", "2200"), ("U", "ops")],
        SshOverride::default(),
    );
    let done = apply(
        &given,
        false,
        &target("${H}", Some(text("${P}")), Some("${U}")),
    )
    .unwrap();
    assert_eq!(
        done.ssh,
        target("10.0.0.5", Some(Port::Number(2200)), Some("ops"))
    );
    assert!(
        done.from.is_empty(),
        "a variable named in the file is the file's own value"
    );
}

#[test]
fn text_around_and_several_variables_work() {
    let given = inputs(
        &[("TEAM", "red"), ("DOMAIN", "example.com")],
        SshOverride::default(),
    );
    let done = apply(
        &given,
        false,
        &target("deploy-${TEAM}.${DOMAIN}", None, None),
    )
    .unwrap();
    assert_eq!(done.ssh.host, "deploy-red.example.com");
}

#[test]
fn a_value_is_not_expanded_again() {
    let given = inputs(&[("A", "${B}"), ("B", "x")], SshOverride::default());
    let done = apply(&given, false, &target("${A}", None, None)).unwrap();
    assert_eq!(done.ssh.host, "${B}");
}

#[test]
fn an_unset_variable_names_the_variable_the_field_and_the_deploy() {
    let given = inputs(&[], SshOverride::default());
    let failures = apply(&given, false, &target("${SHOP_HOST}", None, None)).unwrap_err();
    assert_eq!(failures[0].field, "host");
    let message = &failures[0].message;
    assert!(message.contains("SHOP_HOST"), "{message}");
    assert!(message.contains("server.ssh.host"), "{message}");
    assert!(message.contains("shop"), "{message}");
}

#[test]
fn an_empty_variable_is_an_error_too() {
    let given = inputs(&[("SHOP_USER", "")], SshOverride::default());
    let failures = apply(
        &given,
        false,
        &target("example.com", None, Some("${SHOP_USER}")),
    )
    .unwrap_err();
    assert_eq!(failures[0].field, "user");
    assert!(failures[0].message.contains("SHOP_USER"));
}

#[test]
fn every_bad_field_is_reported() {
    let given = inputs(&[], SshOverride::default());
    let failures = apply(
        &given,
        false,
        &target("${A}", Some(text("${B}")), Some("${C}")),
    )
    .unwrap_err();
    let fields: Vec<&str> = failures.iter().map(|failure| failure.field).collect();
    assert_eq!(fields, ["host", "user", "port"]);
}

#[test]
fn a_bad_variable_name_is_an_error() {
    let given = inputs(&[("1X", "v")], SshOverride::default());
    for written in ["${1X}", "${}", "${A-B}", "${a b}"] {
        let failures = apply(&given, false, &target(written, None, None)).unwrap_err();
        assert!(
            failures[0].message.contains("not a variable name"),
            "{written}: {failures:?}"
        );
    }
}

#[test]
fn an_unclosed_brace_is_plain_text() {
    let given = inputs(&[], SshOverride::default());
    let done = apply(&given, false, &target("a${b", None, None)).unwrap();
    assert_eq!(done.ssh.host, "a${b");
}

#[test]
fn the_port_must_be_from_1_to_65535_after_the_replacement() {
    for bad in ["0", "65536", "abc", "22.5", "-1"] {
        let given = inputs(&[("P", bad)], SshOverride::default());
        let failures = apply(&given, false, &target("h", Some(text("${P}")), None)).unwrap_err();
        assert_eq!(failures[0].field, "port", "{bad}");
        assert!(failures[0].message.contains("1 to 65535"), "{bad}");
    }
    for good in ["1", "22", "65535"] {
        let given = inputs(&[("P", good)], SshOverride::default());
        let done = apply(&given, false, &target("h", Some(text("${P}")), None)).unwrap();
        assert_eq!(done.ssh.port, Some(Port::Number(good.parse().unwrap())));
    }
    let given = inputs(&[], SshOverride::default());
    assert!(apply(&given, false, &target("h", Some(Port::Number(0)), None)).is_err());
}

#[test]
fn a_flag_wins_over_a_variable_and_a_variable_over_the_file() {
    let vars = [
        ("SHIPDECK_SSH_HOST", "from-var"),
        ("SHIPDECK_SSH_USER", "var-user"),
    ];
    let flags = SshOverride {
        host: Some("from-flag".into()),
        ..SshOverride::default()
    };
    let given = inputs(&vars, flags);
    let done = apply(
        &given,
        true,
        &target("from-file", Some(Port::Number(22)), Some("file-user")),
    )
    .unwrap();
    assert_eq!(done.ssh.host, "from-flag");
    assert_eq!(done.ssh.user.as_deref(), Some("var-user"));
    assert_eq!(done.ssh.port, Some(Port::Number(22)));
    assert_eq!(
        done.from,
        ["host from --host", "user from SHIPDECK_SSH_USER"]
    );
}

#[test]
fn a_port_flag_and_variable_are_checked_like_a_file_value() {
    let given = inputs(&[("SHIPDECK_SSH_PORT", "70000")], SshOverride::default());
    assert!(apply(&given, true, &target("h", None, None)).is_err());
    let given = inputs(&[("SHIPDECK_SSH_PORT", "2201")], SshOverride::default());
    let done = apply(&given, true, &target("h", None, None)).unwrap();
    assert_eq!(done.ssh.port, Some(Port::Number(2201)));
    assert_eq!(done.from, ["port from SHIPDECK_SSH_PORT"]);
}

#[test]
fn a_missing_file_variable_is_no_error_when_a_flag_replaces_it() {
    let given = inputs(&[], flag_host("real.example.com"));
    let done = apply(&given, true, &target("${NOT_SET}", None, None)).unwrap();
    assert_eq!(done.ssh.host, "real.example.com");
    let given = inputs(
        &[("SHIPDECK_SSH_HOST", "v.example.com")],
        SshOverride::default(),
    );
    assert!(apply(&given, true, &target("${NOT_SET}", None, None)).is_ok());
}

#[test]
fn a_file_value_is_still_checked_for_the_fields_nothing_replaces() {
    let given = inputs(&[], flag_host("real.example.com"));
    let failures = apply(
        &given,
        true,
        &target("${NOT_SET}", None, Some("${USER_VAR}")),
    )
    .unwrap_err();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].field, "user");
}

#[test]
fn a_deploy_that_does_not_take_the_overrides_keeps_its_file() {
    let given = inputs(&[("SHIPDECK_SSH_HOST", "v")], flag_host("f"));
    let done = apply(&given, false, &target("file.example.com", None, None)).unwrap();
    assert_eq!(done.ssh.host, "file.example.com");
    assert_eq!(done.from, Vec::<String>::new());
}

#[test]
fn the_flags_and_variables_need_exactly_one_deploy() {
    let given = inputs(&[], flag_host("x"));
    assert_eq!(given.single(1), Ok(true));
    let message = given.single(2).unwrap_err();
    assert!(message.contains("--host"), "{message}");
    assert!(message.contains("~/.ssh/config"), "{message}");
    assert!(message.contains("${NAME}"), "{message}");
    let given = inputs(&[("SHIPDECK_SSH_USER", "u")], SshOverride::default());
    let message = given.single(3).unwrap_err();
    assert!(message.contains("SHIPDECK_SSH_USER"), "{message}");
}

#[test]
fn with_nothing_set_any_number_of_deploys_is_fine() {
    let given = inputs(&[("SHIPDECK_SSH_KEY", "text")], SshOverride::default());
    assert_eq!(given.single(0), Ok(false));
    assert_eq!(given.single(1), Ok(false));
    assert_eq!(given.single(5), Ok(false));
}

#[test]
fn the_run_is_the_named_deploys_without_repeats_or_else_every_deploy() {
    let listed = ["a".to_string(), "b".to_string()];
    let all = ServerInputs::default();
    assert_eq!(all.run(listed.iter()), ["a", "b"]);
    let some = ServerInputs::new(
        Env::empty(),
        SshOverride::default(),
        vec!["b".into(), "b".into()],
    );
    assert_eq!(some.run(listed.iter()), ["b"]);
}
