//! Sizes and durations the way people read them.

use std::time::Duration;

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

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
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(31 * 1024 * 1024), "31.0 MiB");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(duration(Duration::from_millis(1500)), "1.5s");
        assert_eq!(duration(Duration::from_secs(42)), "42s");
        assert_eq!(duration(Duration::from_secs(72)), "1m12s");
        assert_eq!(duration(Duration::from_secs(3720)), "1h02m");
    }
}
