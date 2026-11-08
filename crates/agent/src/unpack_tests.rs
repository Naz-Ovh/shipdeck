use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::UnpackRelease;

use super::unpack_sync;

fn root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("shipdeck-unpack-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(root.join("releases")).expect("releases");
    root
}

fn archive(root: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(root.join("releases/.v1.tar.zst")).expect("archive");
    let encoder = zstd::stream::write::Encoder::new(file, 1).expect("encoder");
    let mut tar = tar::Builder::new(encoder);
    for (name, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(u64::try_from(bytes.len()).expect("size"));
        header.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut header, name, *bytes)
            .expect("append file");
    }
    tar.into_inner()
        .expect("tar")
        .finish()
        .expect("finish encoder");
}

fn receipt(root: &Path, entries: &[(&str, &[u8])]) {
    let mut file = std::fs::File::create(root.join("releases/.v1.sha256")).expect("receipt");
    for (name, bytes) in entries {
        writeln!(
            file,
            "{}  ./{name}",
            base16ct::lower::encode_string(&Sha256::digest(bytes))
        )
        .expect("receipt line");
    }
}

fn request(root: &Path, bytes: u64, entries: u64, modes: Vec<u32>) -> UnpackRelease {
    UnpackRelease {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        modes,
        max_bytes: bytes,
        max_entries: entries,
    }
}

#[test]
fn mismatch_keeps_staging_and_a_full_retry_succeeds() {
    let root = root();
    std::fs::create_dir(root.join("current")).expect("current");
    std::fs::write(root.join("current/keep"), b"evil").expect("changed file");
    archive(&root, &[("new", b"abc")]);
    receipt(&root, &[("new", b"abc"), ("keep", b"good")]);
    std::fs::write(root.join("releases/.v1.reuse.0644"), b"./keep\0").expect("reuse list");
    let result = unpack_sync(&request(&root, 7, 2, vec![0o644]), "first").expect("unpack mismatch");
    assert!(!result.verified);
    assert_eq!(result.mismatches, ["keep"]);
    assert!(root.join("releases/.v1.first.partial/keep").exists());
    assert!(!root.join("releases/v1").exists());
    archive(&root, &[("new", b"abc"), ("keep", b"good")]);
    let result = unpack_sync(&request(&root, 7, 2, vec![]), "second").expect("full retry");
    assert!(result.verified);
    assert_eq!(
        std::fs::read(root.join("releases/v1/keep")).expect("file"),
        b"good"
    );
    assert!(root.join("releases/.v1.first.partial").exists());
    std::fs::remove_dir_all(root).expect("remove test folder");
}

#[test]
fn refuses_oversized_archives_and_escaping_links() {
    let root = root();
    archive(&root, &[("new", b"abc")]);
    receipt(&root, &[("new", b"abc")]);
    assert!(unpack_sync(&request(&root, 2, 1, vec![]), "large").is_err());
    assert!(!root.join("releases/v1").exists());
    let file = std::fs::File::create(root.join("releases/.v1.tar.zst")).expect("archive");
    let encoder = zstd::stream::write::Encoder::new(file, 1).expect("encoder");
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o777);
    header.set_size(0);
    header.set_entry_type(tar::EntryType::Symlink);
    tar.append_link(&mut header, "dir/link", "../../outside")
        .expect("link");
    tar.into_inner()
        .expect("tar")
        .finish()
        .expect("finish encoder");
    assert!(unpack_sync(&request(&root, 0, 1, vec![]), "link").is_err());
    assert!(!root.join("releases/v1").exists());
    std::fs::remove_dir_all(root).expect("remove test folder");
}

#[cfg(unix)]
#[test]
fn allows_internal_symlinks_and_preserves_them() {
    let root = root();
    let file = std::fs::File::create(root.join("releases/.v1.tar.zst")).expect("archive");
    let encoder = zstd::stream::write::Encoder::new(file, 1).expect("encoder");
    let mut tar = tar::Builder::new(encoder);
    let mut file_header = tar::Header::new_gnu();
    file_header.set_mode(0o644);
    file_header.set_size(3);
    file_header.set_entry_type(tar::EntryType::Regular);
    tar.append_data(&mut file_header, "target", b"abc".as_slice())
        .expect("file");
    let mut link_header = tar::Header::new_gnu();
    link_header.set_mode(0o777);
    link_header.set_size(0);
    link_header.set_entry_type(tar::EntryType::Symlink);
    tar.append_link(&mut link_header, "dir/link", "../target")
        .expect("link");
    tar.into_inner()
        .expect("tar")
        .finish()
        .expect("finish encoder");
    receipt(&root, &[("target", b"abc")]);
    assert!(
        unpack_sync(&request(&root, 3, 2, vec![]), "internal")
            .expect("unpack")
            .verified
    );
    assert_eq!(
        std::fs::read_link(root.join("releases/v1/dir/link")).expect("link target"),
        Path::new("../target")
    );
    assert_eq!(
        std::fs::read(root.join("releases/v1/dir/link")).expect("follow link"),
        b"abc"
    );
    std::fs::remove_dir_all(root).expect("remove test folder");
}

#[test]
fn rejects_parent_traversal_and_duplicate_paths() {
    let root = root();
    let file = std::fs::File::create(root.join("releases/.v1.tar.zst")).expect("archive");
    let encoder = zstd::stream::write::Encoder::new(file, 1).expect("encoder");
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o644);
    header.set_size(1);
    header.set_entry_type(tar::EntryType::Regular);
    header.as_mut_bytes()[..7].copy_from_slice(b"../evil");
    header.set_cksum();
    tar.append(&header, b"x".as_slice())
        .expect("append raw entry");
    tar.into_inner()
        .expect("tar")
        .finish()
        .expect("finish encoder");
    receipt(&root, &[("file", b"x")]);
    assert!(unpack_sync(&request(&root, 1, 1, vec![]), "escape").is_err());
    assert!(!root.join("evil").exists());
    archive(&root, &[("same", b"x"), ("same", b"y")]);
    assert!(unpack_sync(&request(&root, 2, 2, vec![]), "duplicate").is_err());
    assert!(!root.join("releases/v1").exists());
    std::fs::remove_dir_all(root).expect("remove test folder");
}
