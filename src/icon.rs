//! The game's icon, a steering wheel, drawn here at any size: the window's icon at
//! runtime (natively), and `assets/icon/car-sim.ico`, which `build.rs` puts into the
//! Windows executable (written by the ignored test `write_icon`; `the_icon_file_is_current`
//! checks it).

use bevy::prelude::*;

/// The window's icon, set once the primary window exists. Wayland ignores it (the
/// desktop file names one), the web has none.
pub struct WindowIconPlugin;

impl Plugin for WindowIconPlugin {
    fn build(&self, _app: &mut App) {
        #[cfg(not(target_arch = "wasm32"))]
        _app.add_systems(Update, set_window_icon);
    }
}

/// The size of the window's icon in pixels.
#[cfg(not(target_arch = "wasm32"))]
const WINDOW_ICON: u32 = 64;

#[cfg(not(target_arch = "wasm32"))]
fn set_window_icon(
    windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut done: Local<bool>,
    _main_thread: bevy::ecs::system::NonSendMarker,
) {
    if *done {
        return;
    }
    let Ok(entity) = windows.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|winit| {
        let Some(window) = winit.get_window(entity) else {
            return;
        };
        match winit::window::Icon::from_rgba(wheel(WINDOW_ICON), WINDOW_ICON, WINDOW_ICON) {
            Ok(icon) => window.set_window_icon(Some(icon)),
            Err(error) => warn!("Couldn't make the window's icon: {error}"),
        }
        *done = true;
    });
}

/// Colours, straight (not premultiplied) sRGB.
const RIM_TOP: [f32; 3] = [0.38, 0.40, 0.44];
const RIM_BOTTOM: [f32; 3] = [0.22, 0.23, 0.26];
const SPOKE: [f32; 3] = [0.80, 0.82, 0.85];
const HUB: [f32; 3] = [0.17, 0.18, 0.20];
const MARK: [f32; 3] = [0.93, 0.30, 0.15];
const OUTLINE: [f32; 3] = [0.05, 0.05, 0.06];

/// Shapes in a square from −1 to 1, y down.
const RIM_RADIUS: f32 = 0.78;
const RIM_HALF: f32 = 0.14;
const OUTLINE_WIDTH: f32 = 0.05;
/// The rim's top-centre mark, half its angle (rad).
const MARK_HALF: f32 = 0.16;
const HUB_RADIUS: f32 = 0.27;
const EMBLEM_RADIUS: f32 = 0.11;
const SPOKE_HALF: f32 = 0.105;
/// Where the side spokes meet the rim, below the middle (rad below horizontal).
const SPOKE_DROP: f32 = 0.22;

/// Samples per pixel along each axis.
const SUPERSAMPLE: u32 = 5;

/// The icon at `size` × `size` pixels, RGBA (straight alpha), row by row from the top.
pub fn wheel(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let scale = 2.0 / size as f32;
    for row in 0..size {
        for column in 0..size {
            // Premultiplied sums over the samples.
            let mut sum = [0.0f32; 4];
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = (column as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32) * scale - 1.0;
                    let y = (row as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32) * scale - 1.0;
                    if let Some(colour) = colour_at(Vec2::new(x, y)) {
                        for (s, c) in sum.iter_mut().zip(colour) {
                            *s += c;
                        }
                        sum[3] += 1.0;
                    }
                }
            }
            let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
            let alpha = sum[3] / samples;
            for channel in &sum[..3] {
                let straight = if sum[3] > 0.0 { channel / sum[3] } else { 0.0 };
                rgba.push((straight * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            rgba.push((alpha * 255.0).round() as u8);
        }
    }
    rgba
}

/// The colour of the topmost shape at a point, if any.
fn colour_at(p: Vec2) -> Option<[f32; 3]> {
    let r = p.length();
    let rim = (r - RIM_RADIUS).abs() - RIM_HALF;
    let hub = r - HUB_RADIUS;
    let spokes = spoke_distance(p);
    if rim <= 0.0 {
        // Straight up is −y.
        let angle = p.x.atan2(-p.y);
        if angle.abs() < MARK_HALF {
            return Some(MARK);
        }
        let t = (p.y / RIM_RADIUS * 0.5 + 0.5).clamp(0.0, 1.0);
        return Some(mix(RIM_TOP, RIM_BOTTOM, t));
    }
    if r <= EMBLEM_RADIUS {
        return Some(MARK);
    }
    if hub <= 0.0 {
        return Some(HUB);
    }
    if spokes <= 0.0 {
        return Some(SPOKE);
    }
    // A dark edge round everything, so it stands out on light backgrounds as the grey
    // does on dark ones.
    if rim.min(hub).min(spokes) <= OUTLINE_WIDTH {
        return Some(OUTLINE);
    }
    None
}

/// How far outside the three spokes a point is (negative inside).
fn spoke_distance(p: Vec2) -> f32 {
    let ends = [
        Vec2::new(-SPOKE_DROP.cos(), SPOKE_DROP.sin()),
        Vec2::new(SPOKE_DROP.cos(), SPOKE_DROP.sin()),
        Vec2::new(0.0, 1.0),
    ];
    ends.iter()
        .map(|end| segment_distance(p, Vec2::ZERO, *end * RIM_RADIUS) - SPOKE_HALF)
        .fold(f32::INFINITY, f32::min)
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
    (p - (a + (b - a) * along)).length()
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// The sizes in the `.ico` file.
#[cfg(test)]
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Where the `.ico` file is, from the crate's root.
#[cfg(test)]
const ICO_FILE: &str = "assets/icon/car-sim.ico";

/// A Windows `.ico` of the icon at `sizes`, each entry a PNG (Windows Vista on).
#[cfg(test)]
fn ico(sizes: &[u32]) -> Vec<u8> {
    let pngs: Vec<Vec<u8>> = sizes.iter().map(|&size| png(size, &wheel(size))).collect();
    let mut out = Vec::new();
    // ICONDIR: reserved, type 1 (icon), count.
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&size, png) in sizes.iter().zip(&pngs) {
        // 256 is written as 0.
        let side = if size >= 256 { 0 } else { size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        // Colour planes, bits per pixel.
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for png in pngs {
        out.extend_from_slice(&png);
    }
    out
}

/// A PNG of RGBA pixels, 8 bits a channel.
#[cfg(test)]
fn png(size: u32, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut header = Vec::new();
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&size.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), compression, filter, no interlace.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    // Every row with filter 0 (none).
    let mut raw = Vec::with_capacity(rgba.len() + size as usize);
    for row in rgba.chunks(size as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(
        &mut out,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9),
    );
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ICO_FILE)
    }

    /// `cargo test --profile release-dev write_icon -- --ignored` after changing the drawing.
    #[test]
    #[ignore]
    fn write_icon() {
        std::fs::create_dir_all(path().parent().unwrap()).unwrap();
        std::fs::write(path(), ico(&ICO_SIZES)).unwrap();
    }

    #[test]
    fn the_icon_file_is_current() {
        let written = std::fs::read(path()).expect("the icon file (run write_icon)");
        assert!(
            written == ico(&ICO_SIZES),
            "{ICO_FILE} isn't the icon drawn now: run the ignored test write_icon"
        );
    }

    #[test]
    fn the_crc_is_png_s() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn the_wheel_is_round_with_a_hole() {
        let size = 64;
        let rgba = wheel(size);
        let alpha = |x: u32, y: u32| rgba[((y * size + x) * 4 + 3) as usize];
        // Corners empty, the rim's top solid, between rim and hub above the middle empty.
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(32, 32 - 25), 255);
        assert_eq!(alpha(32, 32 - 14), 0);
        // The hub.
        assert_eq!(alpha(32, 32), 255);
    }
}
