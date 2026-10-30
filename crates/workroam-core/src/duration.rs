//! Human durations such as `10m`, `1h`, `24h`, `30d`.

use crate::{Error, Result};
use std::time::Duration;

pub fn parse(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::invalid("empty duration"));
    }
    let mut total = 0u64;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        let n: u64 = num
            .parse()
            .map_err(|_| Error::invalid(format!("invalid duration: {s}")))?;
        num.clear();
        let mult = match ch {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86400,
            'w' => 7 * 86400,
            _ => {
                return Err(Error::invalid(format!(
                    "invalid duration unit '{ch}' in {s}"
                )))
            }
        };
        total += n * mult;
    }
    if !num.is_empty() {
        // bare number = seconds
        total += num
            .parse::<u64>()
            .map_err(|_| Error::invalid(format!("invalid duration: {s}")))?;
    }
    Ok(Duration::from_secs(total))
}

pub fn format(d: Duration) -> String {
    let s = d.as_secs();
    if s % 86400 == 0 && s >= 86400 {
        format!("{}d", s / 86400)
    } else if s % 3600 == 0 && s >= 3600 {
        format!("{}h", s / 3600)
    } else if s % 60 == 0 && s >= 60 {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

/// Short human form for progress output: `1m 05s`, `42s`.
pub fn human(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse("10m").unwrap().as_secs(), 600);
        assert_eq!(parse("1h30m").unwrap().as_secs(), 5400);
        assert_eq!(parse("24h").unwrap().as_secs(), 86400);
        assert_eq!(parse("45").unwrap().as_secs(), 45);
        assert!(parse("1x").is_err());
        assert_eq!(format(Duration::from_secs(3600)), "1h");
        assert_eq!(human_bytes(1536), "1.5 KB");
    }
}
