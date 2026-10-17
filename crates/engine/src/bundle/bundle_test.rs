use std::collections::{BTreeMap, HashMap};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use chrono::{TimeZone, Utc};
use zstd::stream::read::Decoder;

use super::*;
use crate::config::{DeployConfig, Deployable};
use crate::release::{Manifest, ReleaseName};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: PathBuf, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A deployable in `folder/deploy` whose project is `folder/app`.
fn deployable(folder: &Path, service: &str, pack: &str) -> Deployable {
    let yaml = format!(
        "
name: Demo
project: ../app
ssh: {{ host: example.com, user: deploy }}
service: {{ name: demo, dir: /opt/demo, {service} }}
pipeline:
  pack:
{pack}
"
    );
    let config: DeployConfig = crate::config::parse(&yaml).unwrap();
    let endpoint = crate::ssh::Endpoint::from_config(
        &config.ssh,
        crate::ssh::ssh_config::HostConfig::default(),
    )
    .unwrap();
    let deploy = folder.join("deploy");
    std::fs::create_dir_all(&deploy).unwrap();
    std::fs::create_dir_all(folder.join("app")).unwrap();
    std::fs::write(deploy.join("deploy.yaml"), &yaml).unwrap();
    Deployable {
        folder: deploy,
        slug: "demo".into(),
        config,
        yaml,
        endpoint,
    }
}

fn release() -> ReleaseName {
    ReleaseName::new(
        "0.3.1",
        "ab12cd3",
        false,
        Utc.with_ymd_and_hms(2026, 10, 14, 15, 30, 12).unwrap(),
    )
}

/// Each archive entry: mode, body, and link target for links.
type Entries = BTreeMap<String, (u32, Vec<u8>, Option<String>)>;

fn unpack(data: &[u8]) -> Entries {
    let mut archive = tar::Archive::new(Decoder::new(data).unwrap());
    let mut files = BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mode = entry.header().mode().unwrap();
        let link = entry
            .link_name()
            .unwrap()
            .map(|t| t.to_string_lossy().into_owned());
        let mut body = Vec::new();
        entry.read_to_end(&mut body).unwrap();
        files.insert(path, (mode, body, link));
    }
    files
}

fn unit_file() -> ServiceFile {
    ServiceFile {
        name: "demo.service".into(),
        text: "[Unit]\n".into(),
    }
}

/// Gathers and packs as a deploy does, with nothing on the server.
fn bundle_of(deployable: &Deployable, own: Option<&ServiceFile>) -> Result<(Contents, Bundle)> {
    let contents = gather(deployable, own, |_, _| {})?;
    let manifest = Manifest::new(&release(), "tester");
    let bundle = pack(&contents, &manifest, &HashMap::new(), |_, _| {})?;
    Ok((contents, bundle))
}

fn packed(deployable: &Deployable) -> (Bundle, Entries) {
    let (_, bundle) = bundle_of(deployable, Some(&unit_file())).unwrap();
    let files = unpack(&bundle.data);
    (bundle, files)
}

fn pack_error(deployable: &Deployable) -> String {
    let own = service_file(deployable, &release(), None).unwrap();
    let err = bundle_of(deployable, own.as_ref()).unwrap_err();
    format!("{err:#}")
}

#[test]
fn packs_binary_files_manifest_and_unit_with_modes() {
    let dir = scratch("pack");
    write(dir.join("app/out/demo"), "\x7fELF fake");
    write(dir.join("app/files/sub/a.txt"), "a");
    write(dir.join("deploy/.env"), "SECRET=1");
    let deployable = deployable(
        &dir,
        "",
        "    binary: out/demo\n    files:\n      include:\n        project: [./files/]\n        deploy: [\".env\"]",
    );
    let manifest = Manifest::new(&release(), "tester");
    let contents = gather(&deployable, Some(&unit_file()), |_, _| {}).unwrap();
    let mut last = (0, 0);
    let bundle = pack(&contents, &manifest, &HashMap::new(), |done, total| {
        last = (done, total)
    })
    .unwrap();

    let files = unpack(&bundle.data);
    assert_eq!(files["demo"].0, 0o755);
    assert_eq!(files[".env"].0, 0o600);
    assert_eq!(files["files/sub/a.txt"].1, b"a");
    assert!(files.contains_key("files/sub"));
    assert_eq!(files["demo.service"].1, b"[Unit]\n");
    let written: Manifest = serde_json::from_slice(&files["release.json"].1).unwrap();
    assert_eq!(written, manifest);
    // deploy.yaml never ships, even next to .env.
    assert!(!files.contains_key("deploy.yaml"));

    assert_eq!(bundle.files, 5);
    assert_eq!(last.0, last.1);
    assert_eq!(last.1, bundle.input_bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_bundle_is_one_zstd_frame_with_a_checksum() {
    let dir = scratch("zstd");
    write(dir.join("app/out/demo"), "\x7fELF fake");
    let deployable = deployable(&dir, "", "    binary: out/demo");
    let (bundle, _) = packed(&deployable);

    assert_eq!(bundle.data[..4], [0x28, 0xb5, 0x2f, 0xfd], "zstd magic");
    // Content_Checksum_flag, bit 2 of the frame header descriptor.
    assert_ne!(bundle.data[4] & 0b100, 0, "no checksum");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ships_the_whole_project_minus_excludes_and_never_git() {
    let dir = scratch("whole");
    for path in [
        "app/main.py",
        "app/pkg/mod.py",
        "app/pkg/__pycache__/mod.pyc",
        "app/__pycache__/main.pyc",
        "app/tests/test_main.py",
        "app/.venv/bin/python",
        "app/.git/HEAD",
        "app/pkg/.git",
        "app/dist/a.map",
        "app/dist/keep.map",
        "app/src/a.test.js",
        "app/src/a.js",
        "deploy/.env",
        "deploy/service.template",
    ] {
        write(dir.join(path), "x");
    }
    let deployable = deployable(
        &dir,
        "start: python3 main.py",
        "    files:
      include:
        project: [., \"!src/**/*.test.js\"]
        deploy: [.]
      exclude:
        project: [tests, \"**/__pycache__\", .venv, \"**/*.map\", \"!dist/keep.map\"]",
    );
    let (_, files) = packed(&deployable);
    let shipped: Vec<&str> = files
        .keys()
        .map(String::as_str)
        .filter(|p| !p.ends_with(".service") && *p != "release.json")
        .collect();
    assert_eq!(
        shipped,
        [
            ".env",
            "dist",
            "dist/keep.map",
            "main.py",
            "pkg",
            "pkg/mod.py",
            "src",
            "src/a.js"
        ]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn globs_pick_files_and_a_deploy_file_replaces_the_project_one() {
    let dir = scratch("globs");
    write(dir.join("app/dist/main.js"), "main");
    write(dir.join("app/dist/vendor/lib.js"), "lib");
    write(dir.join("app/dist/style.css"), "css");
    write(dir.join("app/config.json"), "dev");
    write(dir.join("deploy/config.json"), "prod");
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:
      include:
        project: [\"dist/**/*.{js,json}\", config.json]
        deploy: [config.json]",
    );
    let (contents, bundle) = bundle_of(&deployable, Some(&unit_file())).unwrap();
    let files = unpack(&bundle.data);
    assert!(files.contains_key("dist/main.js"));
    assert!(files.contains_key("dist/vendor/lib.js"));
    assert!(!files.contains_key("dist/style.css"));
    assert_eq!(files["config.json"].1, b"prod");
    assert_eq!(contents.replaced, ["config.json"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn follows_a_named_link_and_keeps_inner_links() {
    let dir = scratch("links");
    write(dir.join("app/out/main.js"), "console.log(1)");
    // `dist` itself is a link: it is followed.
    std::os::unix::fs::symlink("out", dir.join("app/dist")).unwrap();
    write(dir.join("app/node_modules/tool/cli.js"), "#!");
    std::fs::create_dir_all(dir.join("app/node_modules/.bin")).unwrap();
    // A link inside a shipped folder stays a link.
    std::os::unix::fs::symlink("../tool/cli.js", dir.join("app/node_modules/.bin/tool")).unwrap();
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:\n      include:\n        project: [dist, node_modules]",
    );
    let (_, files) = packed(&deployable);
    assert!(files.contains_key("dist/main.js"), "{:?}", files.keys());
    assert_eq!(files["dist"].2, None);
    assert_eq!(
        files["node_modules/.bin/tool"].2.as_deref(),
        Some("../tool/cli.js")
    );
    assert!(!files.keys().any(|p| p.starts_with("out")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_links_point_with_slashes_and_stay_relative() {
    let link = Path::new(r"app\node_modules\.bin\tool");
    let target = |t: &str| files::link_target(link, PathBuf::from(t));
    assert_eq!(
        target(r"..\tool\cli.js").unwrap(),
        Path::new("../tool/cli.js")
    );
    assert_eq!(target(r".\out").unwrap(), Path::new("./out"));
    for absolute in [
        r"C:\app\out",
        r"\app\out",
        r"\\?\C:\app\out",
        r"\\server\share\out",
    ] {
        let err = target(absolute).unwrap_err().to_string();
        assert!(err.contains("a path on this PC"), "{absolute}: {err}");
    }
}

#[test]
fn names_what_the_build_did_not_produce() {
    let dir = scratch("missing");
    write(dir.join("app/package.json"), "{}");
    let deployable = deployable(
        &dir,
        "start: node dist/main.js",
        "    files:\n      include:\n        project: [dist, package.json, \"build/*.js\"]\n        deploy: [.env]",
    );
    let err = pack_error(&deployable);
    assert!(err.contains("dist (not found)"), "{err}");
    assert!(err.contains("build/*.js (matched nothing)"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_clashes_with_the_release_and_shared_folders() {
    let dir = scratch("clash");
    write(dir.join("app/out/demo"), "bin");
    write(dir.join("app/demo"), "a file named like the binary");
    write(dir.join("app/uploads/a.png"), "png");
    let binary = deployable(
        &dir,
        "",
        "    binary: out/demo\n    files:\n      include:\n        project: [\"dem?\"]",
    );
    assert!(pack_error(&binary).contains("demo would be in the release twice"));

    let shared = deployable(
        &dir,
        "start: ./out/demo, shared: [uploads]",
        "    files:\n      include:\n        project: [\"**/*.png\"]",
    );
    let err = pack_error(&shared);
    assert!(
        err.contains("uploads/a.png is packed, but service.shared"),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn renders_the_default_unit() {
    let dir = scratch("unit");
    let deployable = deployable(
        &dir,
        "start: ./demo -E=.env",
        "    binary: target/release/demo",
    );
    let unit = unit::render_for(&deployable, &release(), None).unwrap();
    assert!(unit.contains("Description=Demo v0.3.1 (ab12cd3)"));
    assert!(unit.contains("ExecStart=/opt/demo/current/demo -E=.env"));
    assert!(unit.contains("WorkingDirectory=/opt/demo/current"));
    assert!(unit.contains("Environment=APP_RELEASE=v0.3.1-ab12cd3-20261014.153012"));
    assert!(unit.contains("User=deploy"));
    assert!(unit.contains("ReadWritePaths=/opt/demo/shared"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_placeholder_is_an_error() {
    let err = unit::render("Hello {{nope}}", &[("name", Some("x".into()))]).unwrap_err();
    assert!(err.to_string().contains("nope"));
    assert_eq!(
        unit::render("A={{ name }}!", &[("name", Some("x".into()))]).unwrap(),
        "A=x!"
    );
    let err = unit::render("{{binary}}", &[("binary", None)]).unwrap_err();
    assert!(err.to_string().contains("no pipeline.pack.binary"), "{err}");
}

#[test]
fn exec_start_uses_where_the_server_has_the_program() {
    let dir = scratch("exec");
    let pack = "    files:\n      include:\n        project: [dist]";
    let node = deployable(&dir, "start: node dist/main.js", pack);
    assert_eq!(
        unit::exec_start(&node, Some("/usr/bin/node")),
        "/usr/bin/node dist/main.js"
    );
    // Before preflight the name is all there is.
    assert_eq!(unit::exec_start(&node, None), "node dist/main.js");
    let unit = unit::render_for(&node, &release(), Some("/usr/local/bin/node")).unwrap();
    assert!(!unit.contains("EnvironmentFile"));
    assert!(
        unit.contains("ExecStart=/usr/local/bin/node dist/main.js\n"),
        "{unit}"
    );

    let with_file = deployable(
        &dir,
        "start: node dist/main.js, env_file: ./.env, env: { NODE_ENV: production }",
        pack,
    );
    let with_file = unit::render_for(&with_file, &release(), None).unwrap();
    assert!(
        with_file.contains(
            "EnvironmentFile=/opt/demo/current/.env\nEnvironment=\"NODE_ENV=production\"\n"
        ),
        "{with_file}"
    );

    let python = deployable(&dir, "start: .venv/bin/python -m app", pack);
    assert_eq!(
        unit::exec_start(&python, None),
        "/opt/demo/current/.venv/bin/python -m app"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pm2_runs_node_scripts_through_node_and_the_rest_as_they_are() {
    let dir = scratch("pm2-launch");
    let pack = "    files:\n      include:\n        project: [dist]";
    let node = deployable(
        &dir,
        "manager: pm2, start: node dist/main.js --port 3000",
        pack,
    );
    assert_eq!(
        pm2::launch(&node, Some("/usr/bin/node")),
        pm2::Launch {
            script: "dist/main.js".into(),
            args: vec!["--port".into(), "3000".into()],
            interpreter: "/usr/bin/node".into(),
        }
    );
    let binary = deployable(
        &dir,
        "manager: pm2, start: './demo -E=.env --title \"my api\"'",
        pack,
    );
    assert_eq!(
        pm2::launch(&binary, None),
        pm2::Launch {
            script: "/opt/demo/current/demo".into(),
            args: vec!["-E=.env".into(), "--title".into(), "my api".into()],
            interpreter: "none".into(),
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pm2_file_runs_from_current_with_the_options() {
    let dir = scratch("pm2-file");
    let deployable = deployable(
        &dir,
        "manager: pm2, start: node dist/main.js, env: { NODE_ENV: production }, \
         pm2: { app: web, options: { instances: max, exec_mode: cluster } }",
        "    files:\n      include:\n        project: [dist]",
    );
    let own = service_file(&deployable, &release(), Some("/usr/bin/node"))
        .unwrap()
        .unwrap();
    assert_eq!(own.name, "demo.pm2.json");
    let file: serde_json::Value = serde_json::from_str(&own.text).unwrap();
    assert_eq!(
        file,
        serde_json::json!({ "apps": [{
            "name": "web",
            "cwd": "/opt/demo/current",
            "script": "dist/main.js",
            "args": [],
            "interpreter": "/usr/bin/node",
            "instances": "max",
            "exec_mode": "cluster",
        }]})
    );
    assert_eq!(
        start_line(&deployable, Some("/usr/bin/node")),
        "pm2 app web: /usr/bin/node dist/main.js"
    );
    write(dir.join("app/dist/main.js"), "");
    let (contents, files) = packed_with(&deployable, &own);
    assert!(files.contains_key("demo.pm2.json"));
    assert!(contents.unused_ecosystem.is_empty());

    // An ecosystem file shipped by mistake is named, since PM2 never reads it.
    write(dir.join("app/ecosystem.config.js"), "module.exports = {}");
    let shipped = self::deployable(
        &dir,
        "manager: pm2, start: node dist/main.js",
        "    files:\n      include:\n        project: [dist, ecosystem.config.js]",
    );
    let own = service_file(&shipped, &release(), None).unwrap().unwrap();
    let (contents, _) = packed_with(&shipped, &own);
    assert_eq!(contents.unused_ecosystem, ["ecosystem.config.js"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pm2_ecosystem_must_ship_and_needs_no_file_of_ours() {
    let dir = scratch("pm2-ecosystem");
    let deployable = deployable(
        &dir,
        "manager: pm2, pm2: { ecosystem: ecosystem.config.js }",
        "    files:\n      include:\n        project: [dist, ecosystem.config.js]",
    );
    assert_eq!(service_file(&deployable, &release(), None).unwrap(), None);
    assert_eq!(
        start_line(&deployable, None),
        "pm2 app demo from ecosystem.config.js"
    );
    write(dir.join("app/dist/main.js"), "");
    write(dir.join("app/ecosystem.config.js"), "module.exports = {}");
    let left_out = self::deployable(
        &dir,
        "manager: pm2, pm2: { ecosystem: ecosystem.config.js }",
        "    files:\n      include:\n        project: [dist]",
    );
    let err = pack_error(&left_out);
    assert!(
        err.contains("the release has no ecosystem.config.js"),
        "{err}"
    );
    let (_, bundle) = bundle_of(&deployable, None).unwrap();
    let files = unpack(&bundle.data);
    assert!(files.contains_key("ecosystem.config.js"));
    assert!(!files.keys().any(|path| path.ends_with(".service")));
    std::fs::remove_dir_all(dir).unwrap();
}

fn packed_with(deployable: &Deployable, own: &ServiceFile) -> (Contents, Entries) {
    let (contents, bundle) = bundle_of(deployable, Some(own)).unwrap();
    let files = unpack(&bundle.data);
    (contents, files)
}

#[cfg(unix)]
fn sha(text: &str) -> String {
    content::sha256(text.as_bytes())
}

/// A small release: a folder with a file, a link, and a file at the root.
#[cfg(unix)]
fn delta_deployable(dir: &Path) -> Deployable {
    write(dir.join("app/same.txt"), "same");
    write(dir.join("app/lib/changed.js"), "new");
    for file in ["app/same.txt", "app/lib/changed.js"] {
        std::fs::set_permissions(dir.join(file), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::os::unix::fs::symlink("changed.js", dir.join("app/lib/link.js")).unwrap();
    deployable(
        dir,
        "start: node lib/changed.js",
        "    files:\n      include:\n        project: [same.txt, lib]",
    )
}

#[cfg(unix)]
#[test]
fn sends_only_what_the_server_lacks_and_still_lists_every_path() {
    let dir = scratch("delta");
    let deployable = delta_deployable(&dir);
    let contents = gather(&deployable, Some(&unit_file()), |_, _| {}).unwrap();
    assert_eq!(contents.candidates(), ["lib/changed.js", "same.txt"]);
    let on_server = HashMap::from([
        ("same.txt".to_string(), sha("same")),
        ("lib/changed.js".to_string(), sha("old")),
        // shipdeck's own files always go, whatever the server says.
        ("demo.service".to_string(), sha("[Unit]\n")),
    ]);
    let manifest = Manifest::new(&release(), "tester");
    let bundle = pack(&contents, &manifest, &on_server, |_, _| {}).unwrap();
    let files = unpack(&bundle.data);

    assert!(!files.contains_key("same.txt"));
    assert_eq!(files["lib/changed.js"].1, b"new");
    assert!(files.contains_key("lib"));
    assert_eq!(files["lib/link.js"].2.as_deref(), Some("changed.js"));
    assert!(files.contains_key("release.json"));
    assert!(files.contains_key("demo.service"));

    let mut paths = bundle.paths.clone();
    paths.sort();
    assert_eq!(
        paths,
        [
            "demo.service",
            "lib",
            "lib/changed.js",
            "lib/link.js",
            "release.json",
            "same.txt"
        ]
    );
    assert_eq!((bundle.files, bundle.reused, bundle.sent()), (4, 1, 3));
    assert_eq!(bundle.reused_bytes, 4);
    assert_eq!(
        bundle.reuse,
        BTreeMap::from([(0o644, b"./same.txt\0".to_vec())])
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn the_receipt_lists_every_file_sent_or_reused() {
    let dir = scratch("receipt");
    let deployable = delta_deployable(&dir);
    let contents = gather(&deployable, Some(&unit_file()), |_, _| {}).unwrap();
    let on_server = HashMap::from([("same.txt".to_string(), sha("same"))]);
    let manifest = Manifest::new(&release(), "tester");
    let bundle = pack(&contents, &manifest, &on_server, |_, _| {}).unwrap();
    let json = serde_json::to_vec_pretty(&manifest).unwrap();
    let expected = format!(
        "{}  ./demo.service\n{}  ./lib/changed.js\n{}  ./release.json\n{}  ./same.txt\n",
        sha("[Unit]\n"),
        sha("new"),
        content::sha256(&json),
        sha("same"),
    );
    assert_eq!(String::from_utf8(bundle.receipt).unwrap(), expected);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn reused_files_keep_the_mode_this_release_gives_them() {
    let dir = scratch("modes");
    write(dir.join("app/a.sh"), "a");
    write(dir.join("app/b.txt"), "b");
    std::fs::set_permissions(dir.join("app/a.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        dir.join("app/b.txt"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let deployable = deployable(
        &dir,
        "start: ./a.sh",
        "    files:\n      include:\n        project: [a.sh, b.txt]",
    );
    let contents = gather(&deployable, None, |_, _| {}).unwrap();
    let on_server = HashMap::from([
        ("a.sh".to_string(), sha("a")),
        ("b.txt".to_string(), sha("b")),
    ]);
    let bundle = pack(
        &contents,
        &Manifest::new(&release(), "t"),
        &on_server,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(
        bundle.reuse,
        BTreeMap::from([
            (0o640, b"./b.txt\0".to_vec()),
            (0o755, b"./a.sh\0".to_vec()),
        ])
    );
    std::fs::remove_dir_all(dir).unwrap();
}

// Windows file names cannot hold `\` or a newline.
#[cfg(unix)]
#[test]
fn names_sha256sum_cannot_read_back_always_ship_and_go_unlisted() {
    let dir = scratch("odd-names");
    write(dir.join("app/files/back\\slash"), "x");
    write(dir.join("app/files/new\nline"), "x");
    write(dir.join("app/files/plain"), "x");
    let deployable = deployable(
        &dir,
        "start: ./files/plain",
        "    files:\n      include:\n        project: [files]",
    );
    let contents = gather(&deployable, None, |_, _| {}).unwrap();
    assert_eq!(contents.candidates(), ["files/plain"]);
    let on_server: HashMap<String, String> =
        ["files/back\\slash", "files/new\nline", "files/plain"]
            .into_iter()
            .map(|path| (path.to_string(), sha("x")))
            .collect();
    let bundle = pack(
        &contents,
        &Manifest::new(&release(), "t"),
        &on_server,
        |_, _| {},
    )
    .unwrap();
    let files = unpack(&bundle.data);
    assert!(files.contains_key("files/back\\slash"));
    assert!(files.contains_key("files/new\nline"));
    assert!(!files.contains_key("files/plain"));
    let receipt = String::from_utf8(bundle.receipt).unwrap();
    assert!(receipt.contains("./files/plain\n"), "{receipt}");
    assert!(
        !receipt.contains("slash") && !receipt.contains("new"),
        "{receipt}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_file_that_changes_after_it_was_hashed_is_not_packed() {
    let dir = scratch("changed-under");
    let deployable = delta_deployable(&dir);
    let contents = gather(&deployable, None, |_, _| {}).unwrap();
    write(dir.join("app/lib/changed.js"), "newer");
    let err = pack(
        &contents,
        &Manifest::new(&release(), "t"),
        &HashMap::new(),
        |_, _| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("changed while"), "{err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn the_fingerprint_follows_content_and_settings_not_the_release_name() {
    let dir = scratch("fingerprint");
    let deployable = delta_deployable(&dir);
    let print = |deployable: &Deployable, name: &ReleaseName| {
        let own = service_file(deployable, name, None).unwrap();
        let contents = gather(deployable, own.as_ref(), |_, _| {}).unwrap();
        contents.fingerprint(deployable, None).unwrap()
    };
    let first = print(&deployable, &release());
    assert!(
        first.starts_with("sha256:") && first.len() == 7 + 64,
        "{first}"
    );
    assert_eq!(print(&deployable, &release()), first);
    let later = ReleaseName::new(
        "0.4.0",
        "9f8e7d6",
        true,
        Utc.with_ymd_and_hms(2026, 11, 1, 8, 0, 0).unwrap(),
    );
    assert_eq!(print(&deployable, &later), first, "the name changed it");

    write(dir.join("app/same.txt"), "edited");
    assert_ne!(print(&deployable, &release()), first, "a file");
    write(dir.join("app/same.txt"), "same");
    assert_eq!(print(&deployable, &release()), first);

    std::fs::set_permissions(
        dir.join("app/same.txt"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_ne!(print(&deployable, &release()), first, "a mode");
    std::fs::set_permissions(
        dir.join("app/same.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(print(&deployable, &release()), first);

    let mut edited = deployable.clone();
    edited.yaml.push_str("# a comment\n");
    assert_ne!(print(&edited, &release()), first, "deploy.yaml");

    // The same text, but the unit runs with another variable.
    let mut env = deployable.clone();
    env.config
        .service
        .env
        .insert("NODE_ENV".into(), "production".into());
    assert_ne!(print(&env, &release()), first, "the service's environment");

    std::fs::remove_file(dir.join("app/lib/link.js")).unwrap();
    std::os::unix::fs::symlink("same.txt", dir.join("app/lib/link.js")).unwrap();
    assert_ne!(print(&deployable, &release()), first, "a link's target");
    std::fs::remove_dir_all(dir).unwrap();
}
