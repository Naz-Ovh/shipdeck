use super::*;

#[test]
fn glibc_versions_compare_by_number() {
    let v = |s| GlibcVersion::parse(s).unwrap();
    assert!(v("2.38") > v("2.9"));
    assert!(v("2.31") < v("2.31.1"));
    assert_eq!(v("2.31").to_string(), "2.31");
    assert_eq!(GlibcVersion::parse("musl"), None);
    assert_eq!(GlibcVersion::parse(""), None);
}

#[test]
fn finds_the_newest_glibc_in_a_string_table() {
    let dynstr = b"\0libc.so.6\0GLIBC_2.2.5\0GLIBC_2.38\0GLIBC_2.34\0memcpy\0GLIBC_PRIVATE\0";
    assert_eq!(newest_glibc(dynstr), GlibcVersion::parse("2.38"));
    assert_eq!(newest_glibc(b"\0libc.so\0"), None);
}

#[test]
fn normalizes_uname_spellings() {
    assert_eq!(normalize_arch("x86_64\n"), "x86_64");
    assert_eq!(normalize_arch("arm64"), "aarch64");
    assert_eq!(normalize_arch("armv7l"), "arm");
}

#[test]
fn inspects_its_own_test_binary() {
    let me = std::env::current_exe().unwrap();
    let info = inspect(&me).unwrap();
    assert_ne!(info.arch, "unknown");
    assert!(info.size > 0);
}
