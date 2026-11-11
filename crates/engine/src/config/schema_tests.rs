use std::path::Path;

use crate::config::schema;

const EXAMPLES: [&str; 2] = [
    "docs/feature/config-v2/reference.yaml",
    "docs/feature/config-v2/standalone.yaml",
];

#[test]
fn the_design_examples_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for example in EXAMPLES {
        let file = root.join(example);
        let text = std::fs::read_to_string(&file).unwrap();
        // The reference file names files that need not exist here; the
        // reader still has to understand every key.
        if let Err(problems) = crate::config::parse_deploy(&text) {
            assert!(
                problems.iter().all(|problem| {
                    problem.message.contains("outside")
                        || problem.message.contains("source")
                        || problem.message.contains("does not exist")
                }),
                "{example}: {problems:#?}"
            );
        }
    }
}

/// Every folder in `examples/`, with its root file and deploy files.
#[test]
fn every_example_parses_and_resolves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let examples = root.join("examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(&examples).unwrap() {
        let folder = entry.unwrap().path();
        if !folder.is_dir() {
            continue;
        }
        seen += 1;
        let name = folder.file_name().unwrap().to_string_lossy().to_string();
        let root = crate::config::load::Root::read(&folder, None)
            .unwrap_or_else(|problems| panic!("{name}: {problems:#?}"));
        assert!(!root.deploys.is_empty(), "{name} names no deploy");
        for (deploy, entry) in &root.deploys {
            match entry {
                crate::config::Deploy::File(file) => {
                    let text = std::fs::read_to_string(file)
                        .unwrap_or_else(|err| panic!("{name}: {}: {err}", file.display()));
                    let parsed = crate::config::parse_deploy(&text)
                        .unwrap_or_else(|problems| panic!("{name}/{deploy}: {problems:#?}"));
                    assert!(!parsed.apps.is_empty(), "{name}/{deploy} has no apps");
                }
                crate::config::Deploy::InPlace(parsed) => {
                    assert!(!parsed.apps.is_empty(), "{name}/{deploy} has no apps");
                }
            }
        }
    }
    assert!(seen >= 8, "the examples folder lost one of its folders");
}

#[test]
fn the_published_schema_is_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [schema::FILE_NAME, schema::WEBSITE_COPY] {
        let committed = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        assert!(
            committed == schema::text(),
            "{file} is out of date; run: just schema"
        );
    }
}

#[test]
fn every_object_is_closed_and_every_field_documented() {
    let schema = schema::published();
    assert_eq!(schema["$id"], schema::url());
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    let mut objects = vec![&schema];
    objects.extend(schema["oneOf"].as_array().unwrap());
    objects.extend(schema["definitions"].as_object().unwrap().values());
    for object in objects {
        let Some(properties) = object.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        assert_eq!(object["additionalProperties"], false, "{object}");
        for (name, property) in properties {
            assert!(
                property.get("description").is_some(),
                "`{name}` needs a doc comment: editors show it"
            );
        }
    }
}
