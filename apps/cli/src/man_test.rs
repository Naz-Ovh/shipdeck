use std::collections::BTreeSet;

use serde_json::Value;

use super::{CONFIGS, configs, details, index, suggest, topics};
use shipdeck_engine::config::schema;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Every property path the JSON Schema has, blocks included.
fn schema_paths() -> BTreeSet<String> {
    let root = schema::generate();
    let mut paths = BTreeSet::new();
    walk(&root, &root, "", &mut paths);
    paths
}

fn walk(root: &Value, node: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    let node = resolve(root, node);
    let Some(properties) = node.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, child) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        walk(root, child, &path, out);
        out.insert(path);
    }
}

/// Follows `$ref`, and the lone `allOf` schemars wraps a described one in.
fn resolve<'a>(root: &'a Value, mut node: &'a Value) -> &'a Value {
    loop {
        if let Some(name) = node.get("$ref").and_then(Value::as_str) {
            let name = name.rsplit('/').next().unwrap_or_default();
            node = &root["definitions"][name];
        } else if let Some([only]) = node
            .get("allOf")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
        {
            node = only;
        } else {
            return node;
        }
    }
}

#[test]
fn configs_name_exactly_the_schema_properties() {
    let manual: BTreeSet<String> = CONFIGS.iter().map(|c| c.key.to_string()).collect();
    let schema = schema_paths();
    let missing: Vec<_> = schema.difference(&manual).collect();
    let extra: Vec<_> = manual.difference(&schema).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "manual/configs.yaml lacks {missing:?} and has {extra:?} the schema does not"
    );
}

#[test]
fn the_index_lists_every_topic_once() {
    let text = index();
    for topic in topics() {
        assert_eq!(
            text.matches(&format!("\n{}: ", topic.key)).count(),
            1,
            "{}",
            topic.key
        );
    }
}

#[test]
fn configs_print_one_line_per_property_as_a_tree() {
    let text = configs();
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(lines.len(), CONFIGS.len());
    assert!(
        lines.contains(
            &"ssh:  # where and as whom to log in (the login method itself is never here)"
        )
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("  shell_env: bool, default true  # "))
    );
}

#[test]
fn details_take_keys_apart_or_comma_separated_in_order() {
    let text = details(&args(&["steps,", "service.env", "agents,step.health"])).unwrap();
    let opened: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("<topic ") || l.starts_with("<config "))
        .collect();
    assert_eq!(opened.len(), 4);
    assert!(opened[0].starts_with("<topic key=\"steps\""));
    assert!(opened[1].starts_with("<config key=\"service.env\" type=\"map of text, default {}\""));
    assert!(opened[2].starts_with("<topic key=\"agents\""));
    assert!(opened[3].starts_with("<topic key=\"step.health\""));
    assert_eq!(text.matches("</topic>").count(), 3);
    assert_eq!(text.matches("</config>").count(), 1);
}

#[test]
fn a_quoted_list_with_spaces_works_too() {
    let text = details(&args(&["agents, cli.deploy,  exit-codes"])).unwrap();
    assert_eq!(text.matches("</topic>").count(), 3);
}

#[test]
fn a_block_lists_what_is_inside_it() {
    let text = details(&args(&["pipeline.health"])).unwrap();
    assert!(text.contains("fields:\n  url: "), "{text}");
    assert!(text.contains("\n  check:  # "));
    assert!(text.contains("\n    server: commands  # "));
    assert!(!text.contains("done"), "only the block's own fields");
}

#[test]
fn an_unknown_key_prints_nothing_and_suggests() {
    let err = details(&args(&["agents", "env", "stepz"])).unwrap_err();
    assert!(
        err.lines().any(
            |l| l.starts_with("error: `env` is no topic or property; did you mean")
                && l.contains("service.env")
        ),
        "{err}"
    );
    assert!(err.contains("`stepz`"));
    assert!(err.contains("Nothing was printed"));
}

#[test]
fn suggestions_find_typos_and_parts() {
    assert!(suggest("quickstrat").contains(&"quickstart"));
    assert!(suggest("timeout_secs").contains(&"pipeline.health.timeout_secs"));
    assert!(suggest("zzzzzzzz").is_empty());
}

#[test]
fn details_end_with_the_online_docs_once() {
    let text = details(&args(&["agents", "service.env"])).unwrap();
    assert!(text.ends_with("</config>\n\nFor more, check out https://forgedoc.naz.ovh/llms.txt\n"));
    assert_eq!(text.matches("forgedoc.naz.ovh").count(), 1);
}
