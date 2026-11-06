use super::*;
use crate::config::monorepo::parse;

#[test]
fn uses_each_apps_backup_rules_and_keeps_shared_data_out() {
    let config = parse("name: product\nssh: {host: prod}\ndir: /opt/product\napps:\n  api:\n    start: ./api\n    shared: [uploads]\n    pipeline: {backup: {files: {exclude: [secret]}}}\n  web: {start: ./web}\n").unwrap();
    let paths = [
        "release.json",
        "shipdeck-monorepo.json",
        "apps",
        "apps/api",
        "apps/api/server",
        "apps/api/secret",
        "apps/api/uploads",
        "apps/api/uploads/photo",
        "apps/web",
        "apps/web/index",
    ];
    let listing: Vec<ReleaseFile> = paths
        .iter()
        .map(|path| ReleaseFile {
            path: (*path).into(),
            is_dir: matches!(*path, "apps" | "apps/api" | "apps/api/uploads" | "apps/web"),
        })
        .collect();
    let deployed: Vec<String> = paths.iter().map(|path| (*path).into()).collect();
    let selected = pick_apps(&config, &listing, &deployed, &["release.json".into()]).unwrap();
    assert!(selected.paths.contains(&"apps/api/server".into()));
    assert!(selected.paths.contains(&"shipdeck-monorepo.json".into()));
    assert!(selected.paths.contains(&"apps/web/index".into()));
    assert!(!selected.paths.contains(&"apps/api/secret".into()));
    assert!(!selected.paths.contains(&"apps/api/uploads/photo".into()));
}

#[test]
fn keeps_release_records_even_when_backup_rules_exclude_them() {
    let config = parse("name: product\nssh: {host: prod}\ndir: /opt/product\npipeline: {backup: {files: {deployed: false, exclude: ['*']}}}\napps: {api: {start: ./api}}\n").unwrap();
    let listing = [
        "release.json",
        "shipdeck-monorepo.json",
        "product-api.service",
        "secret",
    ]
    .iter()
    .map(|path| ReleaseFile {
        path: (*path).into(),
        is_dir: false,
    })
    .collect::<Vec<_>>();
    let selected = pick_apps(&config, &listing, &[], &["product-api.service".into()]).unwrap();
    assert_eq!(
        selected.paths,
        [
            "product-api.service",
            "release.json",
            "shipdeck-monorepo.json"
        ]
    );
}
