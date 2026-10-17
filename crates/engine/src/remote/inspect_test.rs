use super::*;

#[test]
fn reads_a_folder_listing() {
    let text = "drwxr-xr-x\t4096\tdist\t\0\
                -rwxr-xr-x\t8613\tdist/my-api\t\0\
                lrwxrwxrwx\t15\tuploads\t/opt/my-api/shared/uploads\0\
                -rw-r--r--\t12\ta name with spaces\t\0";
    let files = parse_find(text);
    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["a name with spaces", "dist", "dist/my-api", "uploads"]
    );
    assert_eq!(files[1].kind, FileKind::Dir);
    assert_eq!((files[2].kind, files[2].size), (FileKind::File, 8613));
    assert_eq!(files[2].mode, "-rwxr-xr-x");
    assert_eq!(files[3].kind, FileKind::Link);
    assert_eq!(
        files[3].target.as_deref(),
        Some("/opt/my-api/shared/uploads")
    );
    assert_eq!(files[0].target, None);
}

#[test]
fn reads_a_tar_listing() {
    let text = "\
drwxr-xr-x root/root         0 2026-10-17 18:37 ./
drwxr-xr-x root/root         0 2026-10-17 18:37 ./dist/
-rwxr-xr-x root/root   8613000 2026-10-17 18:37 ./dist/my-api
-rw------- root/root        42 2026-10-17 18:37 ./.env
lrwxrwxrwx root/root         0 2026-10-17 18:37 ./latest -> dist/my-api
-rw-r--r-- 1000/1000        12 2026-10-17 18:37 ./a name with spaces
";
    let files = parse_tar_listing(text);
    let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            ".env",
            "a name with spaces",
            "dist",
            "dist/my-api",
            "latest"
        ]
    );
    assert_eq!(files[0].mode, "-rw-------");
    assert_eq!(files[2].kind, FileKind::Dir);
    assert_eq!(files[3].size, 8_613_000);
    assert_eq!(files[4].kind, FileKind::Link);
    assert_eq!(files[4].target.as_deref(), Some("dist/my-api"));
}

#[test]
fn skips_lines_that_are_not_entries() {
    assert!(parse_tar_listing("tar: something went wrong\n\n").is_empty());
    assert!(parse_find("").is_empty());
}
