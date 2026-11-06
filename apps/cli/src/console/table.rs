//! The release list as aligned columns, for people and for `awk`.

use chrono::DateTime;
use shipdeck_engine::human;
use shipdeck_protocol::message::common::ReleaseKind;
use shipdeck_protocol::message::request::ReleasesListResult;

/// The release list as aligned columns, for people and for `awk`.
pub(crate) fn table(listing: &ReleasesListResult) -> String {
    let mut rows = vec![[
        "RELEASE".to_string(),
        "VERSION".into(),
        "COMMIT".into(),
        "BUILT (UTC)".into(),
        "SIZE".into(),
        "STATE".into(),
        "DEPLOYED BY".into(),
    ]];
    for entry in &listing.entries {
        let (version, commit, built) = match &entry.name {
            Some(name) => (
                name.version.clone(),
                format!("{}{}", name.git, if name.dirty { "*" } else { "" }),
                built_at(&name.built_at),
            ),
            None => ("-".into(), "-".into(), "-".into()),
        };
        let state = match (entry.current, entry.kind, entry.name.is_some()) {
            (true, _, _) => "live",
            (_, _, false) => "not-ours",
            (_, ReleaseKind::Open, _) => "unpacked",
            (_, ReleaseKind::Compressed, _) => "compressed",
        };
        rows.push([
            entry.id.clone(),
            version,
            commit,
            built,
            entry.size_bytes.map_or("-".into(), human::bytes),
            state.into(),
            entry
                .manifest
                .as_ref()
                .map_or("-".into(), |m| m.deployed_by.clone()),
        ]);
    }
    let widths: Vec<usize> = (0..rows[0].len())
        .map(|col| {
            rows.iter()
                .map(|row| row[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// `2026-10-14 15:30` in UTC, or `-` when the engine sent no time.
fn built_at(text: &str) -> String {
    DateTime::parse_from_rfc3339(text).map_or_else(
        |_| "-".to_string(),
        |at| at.format("%Y-%m-%d %H:%M").to_string(),
    )
}

#[cfg(test)]
#[path = "table_test.rs"]
mod tests;
