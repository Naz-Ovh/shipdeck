use crate::config::*;

/// The examples in the repository must stay valid and carry the schema
/// line for this version.
#[test]
fn examples_parse_and_point_at_this_schema() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0;
    for entry in std::fs::read_dir(&examples).unwrap() {
        let file = entry.unwrap().path().join(FILE_NAMES[0]);
        let text = std::fs::read_to_string(&file).unwrap();
        if let Err(problems) = parse(&text) {
            panic!("{}: {problems:#?}", file.display());
        }
        assert_eq!(
            text.lines().next(),
            Some(schema::modeline().as_str()),
            "{}",
            file.display()
        );
        seen += 1;
    }
    assert!(seen >= 4, "expected the rust, go, node and python examples");
}

/// `deploy.schema.json` at the repository root, and the website's copy, are
/// what these types say.
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
fn the_schema_closes_every_object_and_documents_every_field() {
    let schema = schema::generate();
    assert_eq!(schema["$id"], schema::url());
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    let mut objects = vec![&schema];
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
