use std::path::Path;

#[test]
fn the_published_schema_is_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join(super::FILE_NAME);
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        found,
        super::text(),
        "{} is out of date; run: just protocol",
        super::FILE_NAME
    );
}
