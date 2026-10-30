//! Startup banner (PRD CLI-01..06, architecture §17.3). Written to stderr,
//! at most once per process, only for people at a terminal.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

pub const ART: [&str; 6] = [
    r" __        __         _",
    r" \ \      / /__  _ __| | ___ __ ___   __ _ _ __ ___",
    r"  \ \ /\ / / _ \| '__| |/ / '__/ _ \ / _` | '_ ` _ \",
    r"   \ V  V / (_) | |  |   <| | | (_) | (_| | | | | | |",
    r"    \_/\_/ \___/|_|  |_|\_\_|  \___/ \__,_|_| |_| |_|",
    r" (o)----------------------*-----------------------(o)",
];

pub const WIDTH: usize = 52;

const BLUE: (u8, u8, u8) = (0x2F, 0x6B, 0xFF);
const LAGOON: (u8, u8, u8) = (0x1F, 0xB6, 0xE8);
const MINT: (u8, u8, u8) = (0x2E, 0xD3, 0xB7);
const AMBER: (u8, u8, u8) = (0xFF, 0xB5, 0x47);

const fn lerp(a: u8, b: u8, num: usize, den: usize) -> u8 {
    (a as isize + (b as isize - a as isize) * num as isize / den as isize) as u8
}

const fn mix(a: (u8, u8, u8), b: (u8, u8, u8), num: usize, den: usize) -> (u8, u8, u8) {
    (
        lerp(a.0, b.0, num, den),
        lerp(a.1, b.1, num, den),
        lerp(a.2, b.2, num, den),
    )
}

/// Column 1 = Roam Blue, middle = Lagoon, column 52 = Mint (computed at compile time).
pub const GRADIENT: [(u8, u8, u8); WIDTH] = {
    let mut out = [(0u8, 0u8, 0u8); WIDTH];
    let mid = (WIDTH - 1) / 2;
    let mut i = 0;
    while i < WIDTH {
        out[i] = if i <= mid {
            mix(BLUE, LAGOON, i, mid)
        } else {
            mix(LAGOON, MINT, i - mid, WIDTH - 1 - mid)
        };
        i += 1;
    }
    out
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

pub fn detect_depth(when: crate::cli::ColorWhen) -> Depth {
    use crate::cli::ColorWhen;
    if when == ColorWhen::Never || std::env::var_os("NO_COLOR").is_some() {
        return Depth::None;
    }
    if when == ColorWhen::Auto && !std::io::stderr().is_terminal() {
        return Depth::None;
    }
    let ct = std::env::var("COLORTERM")
        .unwrap_or_default()
        .to_lowercase();
    if ct == "truecolor" || ct == "24bit" || std::env::var_os("WT_SESSION").is_some() {
        return Depth::TrueColor;
    }
    if std::env::var("TERM")
        .unwrap_or_default()
        .contains("256color")
    {
        return Depth::Ansi256;
    }
    Depth::Ansi16
}

/// Nearest xterm-256 color (6×6×6 cube or gray ramp).
pub fn to_256((r, g, b): (u8, u8, u8)) -> u8 {
    let q = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let (qr, qg, qb) = (q(r), q(g), q(b));
    let lv = [0u8, 95, 135, 175, 215, 255];
    let cube = (lv[qr as usize], lv[qg as usize], lv[qb as usize]);
    let d = |a: (u8, u8, u8)| {
        let dr = a.0 as i32 - r as i32;
        let dg = a.1 as i32 - g as i32;
        let db = a.2 as i32 - b as i32;
        dr * dr + dg * dg + db * db
    };
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi = if avg > 238 {
        23
    } else {
        (avg.saturating_sub(3)) / 10
    } as u8;
    let gv = 8 + 10 * gi;
    if d((gv, gv, gv)) < d(cube) {
        232 + gi
    } else {
        16 + 36 * qr + 6 * qg + qb
    }
}

fn paint(c: char, col: usize, depth: Depth, star: bool) -> String {
    if c == ' ' || depth == Depth::None {
        return c.to_string();
    }
    let rgb = if star {
        AMBER
    } else {
        GRADIENT[col.min(WIDTH - 1)]
    };
    let bold = if star { "1;" } else { "" };
    match depth {
        Depth::TrueColor => format!("\x1b[{bold}38;2;{};{};{}m{c}\x1b[0m", rgb.0, rgb.1, rgb.2),
        Depth::Ansi256 => format!("\x1b[{bold}38;5;{}m{c}\x1b[0m", to_256(rgb)),
        Depth::Ansi16 => {
            let code = if star {
                93
            } else if col < WIDTH / 3 {
                94
            } else if col < 2 * WIDTH / 3 {
                96
            } else {
                92
            };
            format!("\x1b[{bold}{code}m{c}\x1b[0m")
        }
        Depth::None => unreachable!(),
    }
}

pub fn render(version: &str, tagline: &str, depth: Depth, width: Option<usize>) -> String {
    let mut out = String::new();
    let bold = |s: &str| {
        if depth == Depth::None {
            s.to_string()
        } else {
            format!("\x1b[1m{s}\x1b[0m")
        }
    };
    let dim = |s: &str| {
        if depth == Depth::None {
            s.to_string()
        } else {
            format!("\x1b[2m{s}\x1b[0m")
        }
    };
    if matches!(width, Some(w) if w < 60) {
        let route = " (o)-*-(o)";
        for (i, c) in route.chars().enumerate() {
            out.push_str(&paint(c, i.saturating_sub(1) * WIDTH / 9, depth, c == '*'));
        }
        out.push_str(&format!("  {} {}\n", bold("workroam"), dim(version)));
        return out;
    }
    for (row, line) in ART.iter().enumerate() {
        for (i, c) in line.chars().enumerate() {
            let col = i.saturating_sub(1);
            out.push_str(&paint(c, col, depth, row == 5 && c == '*'));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        " {} {} · {}\n",
        bold("workroam"),
        dim(version),
        tagline
    ));
    out
}

static SHOWN: AtomicBool = AtomicBool::new(false);

/// CLI-02: only for people at a terminal, never for agents, scripts or CI.
pub fn should_show(json: bool, quiet: bool, no_banner: bool, setting: bool) -> bool {
    if json || quiet || no_banner || !setting {
        return false;
    }
    if !std::io::stdout().is_terminal() || !std::io::stderr().is_terminal() {
        return false;
    }
    if std::env::var("WORKROAM_NO_BANNER").as_deref() == Ok("1")
        || std::env::var_os("CI").is_some()
        || std::env::var("TERM").as_deref() == Ok("dumb")
    {
        return false;
    }
    true
}

pub fn show(when: crate::cli::ColorWhen) {
    if SHOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let width = terminal_size::terminal_size().map(|(w, _)| w.0 as usize);
    let text = render(
        crate::VERSION,
        &workroam_core::text::tr("tagline"),
        detect_depth(when),
        width,
    );
    let mut e = anstream::stderr();
    let _ = writeln!(e, "{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_is_52_columns() {
        for l in ART {
            assert!(l.len() <= WIDTH + 1, "{l}");
            assert!(l.is_ascii());
        }
        assert_eq!(ART[5].len(), WIDTH + 1);
        assert_eq!(GRADIENT[0], BLUE);
        assert_eq!(GRADIENT[WIDTH - 1], MINT);
    }

    #[test]
    fn plain_and_narrow() {
        let s = render("0.1.0", "tag", Depth::None, Some(120));
        assert!(s.contains("(o)----------------------*-----------------------(o)"));
        assert!(s.ends_with(" workroam 0.1.0 · tag\n"));
        let n = render("0.1.0", "tag", Depth::None, Some(40));
        assert_eq!(n, " (o)-*-(o)  workroam 0.1.0\n");
        let c = render("0.1.0", "tag", Depth::TrueColor, Some(120));
        assert!(c.contains("38;2;255;181;71m*"));
        assert_eq!(to_256((0xFF, 0xB5, 0x47)), 215);
    }
}
