//! QR rendering of pairing codes (terminal and SVG).

use qrcode::render::unicode;
use qrcode::{EcLevel, QrCode};

/// Half-block rendering for terminals (`workroam setup --qr`).
pub fn terminal(data: &str) -> Option<String> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::L).ok()?;
    Some(
        code.render::<unicode::Dense1x2>()
            .dark_color(unicode::Dense1x2::Light)
            .light_color(unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
    )
}

/// SVG for the desktop app.
pub fn svg(data: &str) -> Option<String> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::L).ok()?;
    let w = code.width();
    let colors = code.to_colors();
    let mut path = String::new();
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            path.push_str(&format!("M{},{}h1v1h-1z", i % w + 2, i / w + 2));
        }
    }
    let size = w + 4;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#000\"/></svg>"
    ))
}
