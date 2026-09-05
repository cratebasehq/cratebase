//! `/api/utils/avatar/{seed}` — a small cosmetic image generator,
//! unauthenticated on purpose.
//!
//! The endpoint is a pure function of its input: the same `seed` always
//! draws the same avatar. There is nothing here an API rule could
//! usefully gate — no record, no stored data, no per-user secret — so,
//! unlike every other route in this module tree, it skips auth entirely
//! rather than inventing a rule to check against. Being unauthenticated
//! also means the browser can request it directly as `<img src>` without
//! a token.
//!
//! # The avatar style is not designed here
//!
//! The look (one hashed shape, one hashed color from a small hand-picked
//! palette, two dot eyes) mirrors grokbots.ai's avatar grid, which is the
//! reference the admin dashboard follows. This module only implements
//! that already-decided direction — resist the urge to add gradients,
//! more shapes, or a "nicer" renderer here; the point of the style is
//! that it is intentionally minimal and instantly recognizable as a
//! hash, not an illustration.
//!
//! Nothing is cached server-side: generation is cheap (a `Sha256` and a
//! few thousand pixel writes), so every request re-renders from scratch.
//! What *is* cached is the client's copy, via a year-long `immutable`
//! `Cache-Control` — safe because a given `seed` value can never produce
//! different bytes later.

use axum::extract::Path;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use image::{ImageFormat, Rgba, RgbaImage};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::http_error::{ApiQuery, ApiResult};

pub fn router() -> Router<App> {
    Router::new().route("/utils/avatar/{seed}", get(avatar))
}

/// Hand-picked so every entry reads clearly on both a light and a dark
/// dashboard background — no pale pastels that wash out on white, no
/// near-blacks that vanish on the dark theme.
const PALETTE: [[u8; 3]; 8] = [
    [0xEF, 0x47, 0x6F], // rose
    [0xF2, 0x8C, 0x18], // amber
    [0xE0, 0xC3, 0x1E], // yellow
    [0x3C, 0xB0, 0x71], // green
    [0x1E, 0xB8, 0xB8], // teal
    [0x3B, 0x82, 0xF6], // blue
    [0x8B, 0x5C, 0xF6], // violet
    [0xEC, 0x48, 0xB6], // pink
];

#[derive(Debug, Default, Deserialize)]
struct AvatarQuery {
    #[serde(default)]
    size: Option<u32>,
}

async fn avatar(
    Path(seed): Path<String>,
    ApiQuery(query): ApiQuery<AvatarQuery>,
) -> ApiResult<Response> {
    let size = query.size.unwrap_or(64).clamp(16, 512);

    let hash = Sha256::digest(seed.as_bytes());
    let shape = Shape::from_index(hash[0] as usize % 3);
    let [r, g, b] = PALETTE[hash[1] as usize % PALETTE.len()];
    let color = Rgba([r, g, b, 0xFF]);

    let png = render_avatar(shape, color, size);
    Ok(png_response(png))
}

#[derive(Clone, Copy)]
enum Shape {
    Circle,
    Hexagon,
    RoundedSquare,
}

impl Shape {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Shape::Circle,
            1 => Shape::Hexagon,
            _ => Shape::RoundedSquare,
        }
    }

    /// Whether `(x, y)`, expressed as coordinates in a unit square
    /// `[0, 1) x [0, 1)`, falls inside the shape. A small inset keeps every
    /// shape clear of the image edge so nothing gets clipped.
    fn contains(self, x: f64, y: f64) -> bool {
        const INSET: f64 = 0.06;
        let (x, y) = (x, y);
        if x < INSET || x > 1.0 - INSET || y < INSET || y > 1.0 - INSET {
            return false;
        }
        // Re-normalize to [0, 1] within the inset box for the shape math.
        let span = 1.0 - 2.0 * INSET;
        let (nx, ny) = ((x - INSET) / span, (y - INSET) / span);
        let (cx, cy) = (nx - 0.5, ny - 0.5);

        match self {
            Shape::Circle => cx * cx + cy * cy <= 0.25,
            Shape::RoundedSquare => {
                let radius = 0.16;
                let hx = (cx.abs() - (0.5 - radius)).max(0.0);
                let hy = (cy.abs() - (0.5 - radius)).max(0.0);
                hx * hx + hy * hy <= radius * radius
            }
            Shape::Hexagon => {
                // Flat-top regular hexagon via the standard three-way
                // point-in-hexagon test against its half-width/half-height.
                let (hx, hy) = (cx.abs(), cy.abs());
                let half_width = 0.5;
                let half_height = 0.5;
                hx <= half_width
                    && hy <= half_height
                    && half_height * half_width - half_height * hx - 0.5 * half_width * hy >= 0.0
            }
        }
    }
}

/// Draws `shape` filled with `color` and two round eyes on a transparent
/// background, at `size x size` pixels.
fn render_avatar(shape: Shape, color: Rgba<u8>, size: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let s = size as f64;

    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f64 + 0.5) / s, (y as f64 + 0.5) / s);
            if shape.contains(u, v) {
                img.put_pixel(x, y, color);
            }
        }
    }

    // Two simple dot eyes, sitting slightly above center so the shape
    // still reads as a "face" rather than a random blob.
    let eye_radius = s * 0.07;
    let eye_y = s * 0.44;
    for eye_x in [s * 0.36, s * 0.64] {
        draw_dot(
            &mut img,
            eye_x,
            eye_y,
            eye_radius,
            Rgba([0x11, 0x11, 0x11, 0xFF]),
        );
    }

    img
}

fn draw_dot(img: &mut RgbaImage, cx: f64, cy: f64, radius: f64, color: Rgba<u8>) {
    let (w, h) = (img.width(), img.height());
    let x0 = (cx - radius).floor().max(0.0) as u32;
    let x1 = (cx + radius).ceil().min(w as f64) as u32;
    let y0 = (cy - radius).floor().max(0.0) as u32;
    let y1 = (cy + radius).ceil().min(h as f64) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            if dx * dx + dy * dy <= radius * radius {
                img.put_pixel(x, y, color);
            }
        }
    }
}

fn png_response(img: RgbaImage) -> Response {
    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    // `RgbaImage::write_to` only fails for an I/O error from the writer,
    // and a `Vec` cursor never fails, so this is infallible in practice.
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut cursor, ImageFormat::Png)
        .expect("encoding to an in-memory buffer cannot fail");

    (
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_is_byte_identical() {
        let hash_a = Sha256::digest(b"user-123");
        let hash_b = Sha256::digest(b"user-123");
        assert_eq!(hash_a, hash_b);

        let shape = Shape::from_index(hash_a[0] as usize % 3);
        let [r, g, b] = PALETTE[hash_a[1] as usize % PALETTE.len()];
        let img_a = render_avatar(shape, Rgba([r, g, b, 0xFF]), 64);
        let img_b = render_avatar(shape, Rgba([r, g, b, 0xFF]), 64);
        assert_eq!(img_a.into_raw(), img_b.into_raw());
    }

    #[test]
    fn different_seeds_pick_different_looks() {
        let hash_a = Sha256::digest(b"seed-a");
        let hash_b = Sha256::digest(b"totally-different-seed");
        let shape_a = hash_a[0] as usize % 3;
        let shape_b = hash_b[0] as usize % 3;
        let color_a = hash_a[1] as usize % PALETTE.len();
        let color_b = hash_b[1] as usize % PALETTE.len();
        assert!(shape_a != shape_b || color_a != color_b);
    }
}
