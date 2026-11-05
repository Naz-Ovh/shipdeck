//! Sizes and durations the way people read them.

use std::time::Duration;

/// Sizes in binary units with one decimal place: `512 B`, `1.5 KiB`.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit = 0;
    let mut divisor = 1;
    while n / divisor >= 1024 && unit < UNITS.len() - 1 {
        divisor *= 1024;
        unit += 1;
    }
    if unit == 0 {
        return format!("{n} B");
    }
    // Round the tenth with whole numbers; every divisor here is a power of
    // two, so a tie cannot happen and no float is needed.
    let whole = n / divisor;
    let tenth = (n % divisor * 10 + divisor / 2) / divisor;
    if tenth == 10 {
        format!("{}.0 {}", whole + 1, UNITS[unit])
    } else {
        format!("{whole}.{tenth} {}", UNITS[unit])
    }
}

#[must_use]
pub fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=9 => format!("{:.1}s", d.as_secs_f64()),
        10..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(31 * 1024 * 1024), "31.0 MiB");
        // Rounding carries into the whole part, as it did with floats.
        assert_eq!(bytes(1024 * 1024 - 1), "1024.0 KiB");
        assert_eq!(bytes(u64::MAX), "16777216.0 TiB");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(Duration::from_millis(1500)), "1.5s");
        assert_eq!(duration(Duration::from_secs(42)), "42s");
        assert_eq!(duration(Duration::from_secs(72)), "1m12s");
        assert_eq!(duration(Duration::from_mins(62)), "1h02m");
    }
}
