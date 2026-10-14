use super::*;

fn pattern(text: &str) -> Pattern {
    Pattern::parse(text).unwrap()
}

fn matches(text: &str, path: &str) -> bool {
    pattern(text).matches(path, false)
}

#[test]
fn reads_like_fast_glob_and_tailwind() {
    assert!(matches("**/*.js", "app.js"));
    assert!(matches("**/*.js", "src/deep/app.js"));
    assert!(matches("*.js", "app.js"));
    assert!(!matches("*.js", "src/app.js"));
    assert!(matches("src/*.{js,ts}", "src/a.ts"));
    assert!(!matches("src/*.{js,ts}", "src/a.css"));
    assert!(matches("src/{lib,bin/{x,y}}/*.rs", "src/bin/y/main.rs"));
    assert!(matches("file[0-9].txt", "file7.txt"));
    assert!(!matches("file[!0-9].txt", "file7.txt"));
    assert!(matches("?.md", "a.md"));
    assert!(matches(r"a\*b", "a*b"));
    assert!(!matches(r"a\*b", "axb"));
    // Names starting with a dot are not special.
    assert!(matches("*", ".env"));
    assert!(matches("**/*.map", "dist/.hidden.map"));
}

#[test]
fn reads_shipdeck_prefixes_and_suffixes() {
    let negated = pattern("!src/**/*.test.js");
    assert!(negated.negated);
    assert!(negated.matches("src/a/b.test.js", false));
    assert!(!pattern("!!dist").negated);

    let folders = pattern("dist/");
    assert!(folders.matches("dist", true));
    assert!(!folders.matches("dist", false));

    for root in [".", "./", "./."] {
        let whole = pattern(root);
        assert!(whole.is_root());
        assert!(whole.matches("", true));
        assert!(!whole.matches("a", false));
    }
    assert!(pattern("./dist/main.js").matches("dist/main.js", false));
}

#[test]
fn literal_paths_are_known() {
    assert_eq!(pattern("./dist/").literal().as_deref(), Some("dist"));
    assert_eq!(pattern(".").literal().as_deref(), Some(""));
    assert_eq!(pattern("dist/*.js").literal(), None);
    assert_eq!(pattern("{a,b}").literal(), None);
    assert_eq!(pattern("!dist").literal(), None);
}

#[test]
fn refuses_patterns_that_cannot_work() {
    for (text, why) in [
        ("", "is empty"),
        ("!", "is empty"),
        ("/etc/passwd", "starts with /"),
        ("../x", "climbs out"),
        ("a/../../x", "climbs out"),
        ("a//b", "empty or `.` folder"),
        ("a/./b", "empty or `.` folder"),
        ("src/{a,b", "never closed"),
        ("src/[ab", "never closed"),
        ("trailing\\", "lone"),
    ] {
        let err = Pattern::parse(text).unwrap_err();
        assert!(err.contains(why), "{text:?}: {err}");
    }
}

#[test]
fn knows_where_matches_can_be() {
    let js = pattern("dist/*.js");
    assert!(js.may_match_below(""));
    assert!(js.may_match_below("dist"));
    assert!(!js.may_match_below("dist/sub"));
    assert!(!js.may_match_below("src"));

    let deep = pattern("dist/**/*.map");
    assert!(deep.may_match_below("dist/a/b/c"));
    assert!(!deep.may_match_below("node_modules"));
    assert!(pattern("**/*.map").may_match_below("anything/at/all"));
    assert!(pattern("src/{a,b/c}/x").may_match_below("src/b/c"));
    assert!(!pattern(".").may_match_below(""));
}

/// Judges `path` and each folder above it, as a walk would.
fn ships(rules: &Rules, path: &str, is_dir: bool) -> bool {
    let mut hits = vec![0; rules.include.len()];
    let mut verdict = rules.judge(Verdict::default(), "", true, &mut hits);
    let parts: Vec<&str> = path.split('/').collect();
    for depth in 1..=parts.len() {
        let here = parts[..depth].join("/");
        let last = depth == parts.len();
        verdict = rules.judge(verdict, &here, !last || is_dir, &mut hits);
    }
    verdict.ships()
}

fn rules(include: &[&str], exclude: &[&str]) -> Rules {
    let own = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Rules::parse(&own(include), &own(exclude)).unwrap()
}

#[test]
fn a_matched_folder_brings_everything_in_it() {
    let rules = rules(&["dist"], &[]);
    assert!(ships(&rules, "dist/a/b.js", false));
    assert!(!ships(&rules, "src/a.js", false));
}

#[test]
fn bang_leaves_out_in_include_and_keeps_in_exclude() {
    let rules = rules(
        &[".", "!tests"],
        &["**/*.map", "!dist/keep.map", "**/__pycache__"],
    );
    assert!(ships(&rules, "app/main.py", false));
    assert!(!ships(&rules, "tests/test_a.py", false));
    assert!(!ships(&rules, "dist/a.map", false));
    assert!(!ships(&rules, "a.map", false));
    assert!(ships(&rules, "dist/keep.map", false));
    assert!(!ships(&rules, "app/__pycache__/x.pyc", false));
    // Order does not matter.
    let other = super::tests::rules(&["!tests", "."], &[]);
    assert!(!ships(&other, "tests/a.py", false));
}

#[test]
fn walks_only_where_something_can_match() {
    let narrow = rules(&["dist/*.js"], &[]);
    let root = Verdict::default();
    assert!(narrow.enter(root, "dist"));
    assert!(!narrow.enter(root, "node_modules"));

    let wide = rules(&["."], &["node_modules"]);
    let mut hits = vec![0];
    let verdict = wide.judge(Verdict::default(), "", true, &mut hits);
    let excluded = wide.judge(verdict, "node_modules", true, &mut hits);
    assert!(!wide.enter(excluded, "node_modules"));
    // Unless a `!` in exclude keeps something inside.
    let keep = rules(&["."], &["node_modules", "!node_modules/.prisma"]);
    let verdict = keep.judge(Verdict::default(), "", true, &mut hits);
    let excluded = keep.judge(verdict, "node_modules", true, &mut hits);
    assert!(keep.enter(excluded, "node_modules"));
    assert!(ships(&keep, "node_modules/.prisma/client.js", false));
    assert!(!ships(&keep, "node_modules/left-pad/index.js", false));
}
