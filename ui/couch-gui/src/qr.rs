//! The setup screen's QR code.
//!
//! LVGL had a QR widget built in; Slint does not, so the modules are encoded
//! here and handed over as an image. qrcodegen is the same encoder LVGL
//! vendored, so the output is identical to what the C version produced.

use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

/// A "WIFI:" join record - what a phone camera acts on directly, so nobody has
/// to read an SSID off a 3.1" panel and type it in. T:nopass because the setup
/// network is open.
pub fn wifi_join_record(ssid: &str) -> String {
    // Semicolons, commas, colons and backslashes are the separators in this
    // format and have to be escaped or the record silently truncates.
    let escaped: String = ssid
        .chars()
        .flat_map(|c| {
            let esc = matches!(c, '\\' | ';' | ',' | ':' | '"');
            esc.then_some('\\').into_iter().chain(std::iter::once(c))
        })
        .collect();
    format!("WIFI:S:{escaped};T:nopass;;")
}

/// Render `text` as a QR at roughly `target_px`, scaled by a whole number of
/// pixels per module so the edges stay hard - a fractionally scaled QR is
/// measurably harder for a phone to read.
pub fn render(text: &str, target_px: u32) -> Option<Image> {
    let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).ok()?;
    let modules = qr.size() as u32;
    let quiet = 4u32; // the standard quiet zone, in modules
    let total = modules + quiet * 2;
    let scale = (target_px / total).max(1);
    let side = total * scale;

    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(side, side);
    let px = buf.make_mut_slice();
    for p in px.iter_mut() {
        *p = Rgb8Pixel { r: 255, g: 255, b: 255 };
    }
    for y in 0..modules {
        for x in 0..modules {
            if !qr.get_module(x as i32, y as i32) {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let px_x = (x + quiet) * scale + dx;
                    let px_y = (y + quiet) * scale + dy;
                    px[(px_y * side + px_x) as usize] = Rgb8Pixel { r: 0, g: 0, b: 0 };
                }
            }
        }
    }
    Some(Image::from_rgb8(buf))
}
