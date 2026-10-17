use super::*;

/// `day` doubles as the build day, so a higher number is newer.
fn id(day: u32) -> String {
    format!("v0.1.{day}-abc{day:04}-202610{day:02}.120000")
}

/// `open` days have a folder and a backup; `backups`
/// days have only their tarball.
fn listing(open: &[u32], backups: &[u32], current: Option<u32>) -> Listing {
    listing_of(open, backups, current, &[])
}

/// Like [`listing`]; `without_backup` are open releases that have none, as
/// when `keep.compressed` is 0.
fn listing_of(
    open: &[u32],
    backups: &[u32],
    current: Option<u32>,
    without_backup: &[u32],
) -> Listing {
    let mut text = String::new();
    if let Some(day) = current {
        text.push_str(&format!("current\treleases/{}\n", id(day)));
    }
    for day in open {
        text.push_str(&format!("open\t{}\t100\t\n", id(*day)));
        if !without_backup.contains(day) {
            text.push_str(&format!("backup\t{}\t4\n", id(*day)));
        }
    }
    for day in backups {
        text.push_str(&format!("backup\t{}\t40\n", id(*day)));
    }
    parse_listing(&text)
}

fn keep(open: usize, compressed: usize) -> Keep {
    Keep { open, compressed }
}

#[test]
fn parses_and_sorts_newest_first() {
    let listing = listing(&[3, 10, 7], &[1], Some(7));
    let ids: Vec<_> = listing.entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, [id(10), id(7), id(3), id(1)]);
    assert_eq!(listing.current, Some(id(7)));
    assert!(listing.entries[1].current);
    assert_eq!(listing.entries[0].size_bytes, Some(100 * 1024));
    assert!(listing.entries[0].backup);
    assert_eq!(listing.entries[3].kind, Kind::Compressed);
    assert_eq!(listing.entries[3].size_bytes, Some(40 * 1024));
}

#[test]
fn reads_release_json() {
    let json = r#"{  "id": "x",  "version": "0.1.3",  "git": "abc0003",  "dirty": false,  "built_at": "t",  "deployed_by": "me"}"#;
    let listing = parse_listing(&format!("open\t{}\t5\t{json}\n", id(3)));
    assert_eq!(
        listing.entries[0].manifest.as_ref().unwrap().deployed_by,
        "me"
    );
}

#[test]
fn ignores_unsafe_and_foreign_names() {
    let listing = parse_listing("open\tbad;name\t1\t\nopen\tmy-notes\t1\t\n");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].id, "my-notes");
    assert!(listing.entries[0].name.is_none());
    assert!(plan_rotation(&listing, keep(1, 0)).is_empty());
}

#[test]
fn nothing_to_do_within_limits() {
    let listing = listing(&[1, 2, 3], &[], Some(3));
    assert!(plan_rotation(&listing, keep(5, 15)).is_empty());
}

#[test]
fn closes_beyond_open_and_drops_beyond_compressed() {
    // 7 open (current = newest), 2 backups only, keep 5 + 3.
    let listing = listing(&[3, 4, 5, 6, 7, 8, 9], &[1, 2], Some(9));
    let plan = plan_rotation(&listing, keep(5, 3));
    // 4 and 3 leave the open tier; their backups stay.
    assert_eq!(plan.close, [id(4), id(3)]);
    assert!(plan.drop_open.is_empty());
    // Backups afterwards, newest first: 4, 3, 2 kept; 1 dropped.
    assert_eq!(plan.drop_backup, [id(1)]);
}

#[test]
fn an_open_release_too_old_for_a_backup_goes_with_its_backup() {
    let listing = listing(&[1, 5, 6], &[2, 3, 4], Some(6));
    let plan = plan_rotation(&listing, keep(2, 3));
    // Open 1 leaves; backups would be 4, 3, 2, 1 → 1 is past the limit.
    assert!(plan.close.is_empty());
    assert_eq!(plan.drop_open, [id(1)]);
    assert_eq!(plan.drop_backup, [id(1)]);
}

#[test]
fn an_open_release_without_a_backup_goes_when_it_leaves_the_open_tier() {
    let listing = listing_of(&[1, 2, 3], &[], Some(3), &[1]);
    let plan = plan_rotation(&listing, keep(1, 5));
    assert_eq!(plan.close, [id(2)]);
    assert_eq!(plan.drop_open, [id(1)]);
}

#[test]
fn keeping_no_backups_deletes_what_leaves_the_open_tier() {
    let listing = listing_of(&[1, 2, 3], &[], Some(3), &[1, 2, 3]);
    let plan = plan_rotation(&listing, keep(2, 0));
    assert_eq!(plan.drop_open, [id(1)]);
    assert!(plan.drop_backup.is_empty() && plan.close.is_empty());
}

#[test]
fn current_is_kept_after_rolling_back_to_an_older_release() {
    let listing = listing(&[1, 2, 3, 4], &[], Some(1));
    let plan = plan_rotation(&listing, keep(2, 10));
    // Current (1) plus the newest other one (4) stay open.
    assert_eq!(plan.close, [id(3), id(2)]);
    assert!(!plan.close.contains(&id(1)));
}
