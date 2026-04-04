use image::{Rgba, RgbaImage};
use imageproc::drawing::{
    draw_antialiased_line_segment_mut, draw_filled_circle_mut, draw_filled_rect_mut,
    draw_hollow_circle_mut, draw_hollow_rect_mut, draw_line_segment_mut, draw_polygon_mut,
};
use imageproc::pixelops::interpolate;
use imageproc::point::Point;
use imageproc::rect::Rect;

/// Draw an icon onto an image. Returns true if the icon was recognized.
pub fn draw_icon(img: &mut RgbaImage, name: &str, color: Rgba<u8>, size: u32) {
    let s = size as f32;
    let c = (size / 2) as i32;
    let dim = |frac: f32| (s * frac) as i32;

    match name {
        "volume" | "speaker" => draw_speaker(img, color, c, dim),
        "mute" | "mic_mute" => draw_mic_mute(img, color, c, dim),
        "mic" | "microphone" => draw_mic(img, color, c, dim),
        "music" | "headphones" => draw_headphones(img, color, c, dim),
        "light" | "bulb" | "sun" => draw_sun(img, color, c, dim),
        "moon" => draw_moon(img, color, c, dim),
        "camera" | "cam" => draw_camera(img, color, c, dim),
        "zoom_in" => draw_zoom(img, color, c, dim, true),
        "zoom_out" => draw_zoom(img, color, c, dim, false),
        "arrow_left" | "left" => draw_arrow(img, color, c, dim, 0),
        "arrow_right" | "right" => draw_arrow(img, color, c, dim, 1),
        "arrow_up" | "up" => draw_arrow(img, color, c, dim, 2),
        "arrow_down" | "down" => draw_arrow(img, color, c, dim, 3),
        "back" => draw_back_arrow(img, color, c, dim),
        "git" | "pr" | "merge" => draw_git_branch(img, color, c, dim),
        "review" | "eye" => draw_eye(img, color, c, dim),
        "ship" | "rocket" => draw_rocket(img, color, c, dim),
        "terminal" | "console" => draw_terminal(img, color, c, dim),
        "chat" | "slack" => draw_chat(img, color, c, dim),
        "globe" | "web" | "browser" => draw_globe(img, color, c, dim),
        "link" | "url" => draw_link(img, color, c, dim),
        "list" | "tasks" => draw_tasks(img, color, c, dim),
        "grid" | "more" | "dots" => draw_grid(img, color, c, dim),
        "power" => draw_power(img, color, c, dim),
        "reset" | "refresh" => draw_refresh(img, color, c, dim),
        "focus" => draw_crosshair(img, color, c, dim),
        "af" | "autofocus" => draw_af(img, color, c, dim),
        "pan_left" => draw_pan_arrow(img, color, c, dim, 0),
        "pan_right" => draw_pan_arrow(img, color, c, dim, 1),
        "tilt_up" => draw_pan_arrow(img, color, c, dim, 2),
        "tilt_down" => draw_pan_arrow(img, color, c, dim, 3),
        _ => {} // Unknown icon, skip
    }
}

type DimFn = dyn Fn(f32) -> i32;

// ── Icon implementations ────────────────────────────────────────

fn draw_speaker(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Speaker body
    let x = dim(0.30);
    let y1 = dim(0.28);
    let y2 = dim(0.42);
    draw_filled_rect_mut(img, Rect::at(x, y1).of_size(dim(0.10) as u32, (y2 - y1) as u32), color);
    // Speaker cone (triangle)
    let pts = vec![
        Point::new(x + dim(0.10), y1),
        Point::new(x + dim(0.10), y2),
        Point::new(dim(0.55), dim(0.20)),
    ];
    draw_polygon_mut(img, &[
        Point::new(dim(0.40), y1 - dim(0.04)),
        Point::new(dim(0.55), dim(0.20)),
        Point::new(dim(0.55), dim(0.50)),
        Point::new(dim(0.40), y2 + dim(0.04)),
    ], color);
    // Sound waves
    draw_hollow_circle_mut(img, (dim(0.58), dim(0.35)), dim(0.06), color);
    draw_hollow_circle_mut(img, (dim(0.58), dim(0.35)), dim(0.12), color);
}

fn draw_mic(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Mic capsule (rounded rect approximation)
    let cx = c;
    let top = dim(0.18);
    draw_filled_circle_mut(img, (cx, top + dim(0.06)), dim(0.08), color);
    draw_filled_rect_mut(img, Rect::at(cx - dim(0.08), top + dim(0.06)).of_size(dim(0.16) as u32, dim(0.14) as u32), color);
    draw_filled_circle_mut(img, (cx, top + dim(0.20)), dim(0.08), color);
    // U-shaped holder
    draw_hollow_circle_mut(img, (cx, top + dim(0.22)), dim(0.14), color);
    // Stand
    draw_line_segment_mut(img, (cx as f32, (top + dim(0.36)) as f32), (cx as f32, (top + dim(0.44)) as f32), color);
    draw_line_segment_mut(img, ((cx - dim(0.10)) as f32, (top + dim(0.44)) as f32), ((cx + dim(0.10)) as f32, (top + dim(0.44)) as f32), color);
}

fn draw_mic_mute(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    draw_mic(img, color, c, &dim);
    // Slash through
    let red = Rgba([255, 60, 60, 255]);
    for offset in -1..=1 {
        draw_line_segment_mut(img,
            ((dim(0.25) + offset) as f32, dim(0.15) as f32),
            ((dim(0.75) + offset) as f32, dim(0.55) as f32), red);
    }
}

fn draw_headphones(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    draw_hollow_circle_mut(img, (c, dim(0.32)), dim(0.18), color);
    // Earcups
    draw_filled_rect_mut(img, Rect::at(dim(0.22), dim(0.32)).of_size(dim(0.08) as u32, dim(0.18) as u32), color);
    draw_filled_rect_mut(img, Rect::at(dim(0.70), dim(0.32)).of_size(dim(0.08) as u32, dim(0.18) as u32), color);
}

fn draw_sun(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    draw_filled_circle_mut(img, (c, cy), dim(0.10), color);
    // Rays
    for i in 0..8 {
        let angle = std::f32::consts::PI * 2.0 * i as f32 / 8.0;
        let inner = dim(0.14) as f32;
        let outer = dim(0.22) as f32;
        let x1 = c as f32 + angle.cos() * inner;
        let y1 = cy as f32 + angle.sin() * inner;
        let x2 = c as f32 + angle.cos() * outer;
        let y2 = cy as f32 + angle.sin() * outer;
        draw_line_segment_mut(img, (x1, y1), (x2, y2), color);
    }
}

fn draw_moon(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    draw_filled_circle_mut(img, (c, cy), dim(0.14), color);
    // Cut out a circle to make crescent
    let bg = img.get_pixel(0, 0).clone();
    draw_filled_circle_mut(img, (c + dim(0.08), cy - dim(0.04)), dim(0.12), bg);
}

fn draw_camera(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Camera body
    draw_hollow_rect_mut(img, Rect::at(dim(0.22), dim(0.26)).of_size(dim(0.56) as u32, dim(0.32) as u32), color);
    // Lens
    draw_hollow_circle_mut(img, (c, dim(0.42)), dim(0.10), color);
    draw_filled_circle_mut(img, (c, dim(0.42)), dim(0.04), color);
    // Flash bump
    draw_filled_rect_mut(img, Rect::at(dim(0.38), dim(0.20)).of_size(dim(0.12) as u32, dim(0.06) as u32), color);
}

fn draw_zoom(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32, zoom_in: bool) {
    let cy = dim(0.32);
    draw_hollow_circle_mut(img, (c, cy), dim(0.14), color);
    // Handle
    draw_line_segment_mut(img, ((c + dim(0.10)) as f32, (cy + dim(0.10)) as f32),
        ((c + dim(0.20)) as f32, (cy + dim(0.20)) as f32), color);
    // + or -
    draw_line_segment_mut(img, ((c - dim(0.08)) as f32, cy as f32), ((c + dim(0.08)) as f32, cy as f32), color);
    if zoom_in {
        draw_line_segment_mut(img, (c as f32, (cy - dim(0.08)) as f32), (c as f32, (cy + dim(0.08)) as f32), color);
    }
}

fn draw_arrow(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32, dir: u8) {
    let cy = dim(0.35);
    match dir {
        0 => { // left
            draw_line_segment_mut(img, (dim(0.30) as f32, cy as f32), (dim(0.70) as f32, cy as f32), color);
            draw_line_segment_mut(img, (dim(0.30) as f32, cy as f32), (dim(0.45) as f32, (cy - dim(0.10)) as f32), color);
            draw_line_segment_mut(img, (dim(0.30) as f32, cy as f32), (dim(0.45) as f32, (cy + dim(0.10)) as f32), color);
        }
        1 => { // right
            draw_line_segment_mut(img, (dim(0.30) as f32, cy as f32), (dim(0.70) as f32, cy as f32), color);
            draw_line_segment_mut(img, (dim(0.70) as f32, cy as f32), (dim(0.55) as f32, (cy - dim(0.10)) as f32), color);
            draw_line_segment_mut(img, (dim(0.70) as f32, cy as f32), (dim(0.55) as f32, (cy + dim(0.10)) as f32), color);
        }
        2 => { // up
            draw_line_segment_mut(img, (c as f32, dim(0.22) as f32), (c as f32, dim(0.50) as f32), color);
            draw_line_segment_mut(img, (c as f32, dim(0.22) as f32), ((c - dim(0.10)) as f32, dim(0.35) as f32), color);
            draw_line_segment_mut(img, (c as f32, dim(0.22) as f32), ((c + dim(0.10)) as f32, dim(0.35) as f32), color);
        }
        _ => { // down
            draw_line_segment_mut(img, (c as f32, dim(0.22) as f32), (c as f32, dim(0.50) as f32), color);
            draw_line_segment_mut(img, (c as f32, dim(0.50) as f32), ((c - dim(0.10)) as f32, dim(0.38) as f32), color);
            draw_line_segment_mut(img, (c as f32, dim(0.50) as f32), ((c + dim(0.10)) as f32, dim(0.38) as f32), color);
        }
    }
}

fn draw_pan_arrow(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32, dir: u8) {
    // Thicker, bolder arrows for camera pan/tilt
    for offset in -1..=1 {
        let c2 = c + offset;
        draw_arrow(img, color, c2, &dim, dir);
    }
}

fn draw_back_arrow(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    // Curved back arrow
    draw_hollow_circle_mut(img, (c, cy), dim(0.14), color);
    // Arrow head on left
    draw_line_segment_mut(img, ((c - dim(0.14)) as f32, cy as f32), ((c - dim(0.06)) as f32, (cy - dim(0.08)) as f32), color);
    draw_line_segment_mut(img, ((c - dim(0.14)) as f32, cy as f32), ((c - dim(0.06)) as f32, (cy + dim(0.08)) as f32), color);
}

fn draw_git_branch(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Main branch line
    let x1 = dim(0.38);
    draw_line_segment_mut(img, (x1 as f32, dim(0.18) as f32), (x1 as f32, dim(0.52) as f32), color);
    draw_filled_circle_mut(img, (x1, dim(0.18)), dim(0.04), color);
    draw_filled_circle_mut(img, (x1, dim(0.52)), dim(0.04), color);
    // Branch
    let x2 = dim(0.58);
    draw_filled_circle_mut(img, (x2, dim(0.28)), dim(0.04), color);
    draw_line_segment_mut(img, (x2 as f32, dim(0.28) as f32), (x1 as f32, dim(0.40) as f32), color);
}

fn draw_eye(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    // Eye outline (two arcs approximated with lines)
    draw_hollow_ellipse_mut(img, (c, cy), dim(0.18), dim(0.10), color);
    // Pupil
    draw_filled_circle_mut(img, (c, cy), dim(0.06), color);
}

fn draw_hollow_ellipse_mut(img: &mut RgbaImage, center: (i32, i32), rx: i32, ry: i32, color: Rgba<u8>) {
    imageproc::drawing::draw_hollow_ellipse_mut(img, center, rx, ry, color);
}

fn draw_rocket(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Simple rocket pointing up
    let cx = c;
    // Body
    draw_filled_rect_mut(img, Rect::at(cx - dim(0.06), dim(0.24)).of_size(dim(0.12) as u32, dim(0.24) as u32), color);
    // Nose cone (triangle)
    draw_polygon_mut(img, &[
        Point::new(cx - dim(0.06), dim(0.24)),
        Point::new(cx + dim(0.06), dim(0.24)),
        Point::new(cx, dim(0.14)),
    ], color);
    // Fins
    draw_polygon_mut(img, &[
        Point::new(cx - dim(0.06), dim(0.48)),
        Point::new(cx - dim(0.14), dim(0.54)),
        Point::new(cx - dim(0.06), dim(0.40)),
    ], color);
    draw_polygon_mut(img, &[
        Point::new(cx + dim(0.06), dim(0.48)),
        Point::new(cx + dim(0.14), dim(0.54)),
        Point::new(cx + dim(0.06), dim(0.40)),
    ], color);
}

fn draw_terminal(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Terminal window
    draw_hollow_rect_mut(img, Rect::at(dim(0.22), dim(0.20)).of_size(dim(0.56) as u32, dim(0.38) as u32), color);
    // Prompt >_
    draw_line_segment_mut(img, (dim(0.30) as f32, dim(0.34) as f32), (dim(0.40) as f32, dim(0.40) as f32), color);
    draw_line_segment_mut(img, (dim(0.40) as f32, dim(0.40) as f32), (dim(0.30) as f32, dim(0.46) as f32), color);
    // Cursor
    draw_line_segment_mut(img, (dim(0.45) as f32, dim(0.46) as f32), (dim(0.55) as f32, dim(0.46) as f32), color);
}

fn draw_chat(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Speech bubble
    draw_hollow_rect_mut(img, Rect::at(dim(0.22), dim(0.20)).of_size(dim(0.56) as u32, dim(0.28) as u32), color);
    // Tail
    draw_polygon_mut(img, &[
        Point::new(dim(0.34), dim(0.48)),
        Point::new(dim(0.28), dim(0.56)),
        Point::new(dim(0.44), dim(0.48)),
    ], color);
    // Lines inside
    draw_line_segment_mut(img, (dim(0.30) as f32, dim(0.30) as f32), (dim(0.65) as f32, dim(0.30) as f32), color);
    draw_line_segment_mut(img, (dim(0.30) as f32, dim(0.38) as f32), (dim(0.55) as f32, dim(0.38) as f32), color);
}

fn draw_globe(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    draw_hollow_circle_mut(img, (c, cy), dim(0.16), color);
    // Meridians
    draw_hollow_ellipse_mut(img, (c, cy), dim(0.08), dim(0.16), color);
    // Equator
    draw_line_segment_mut(img, ((c - dim(0.16)) as f32, cy as f32), ((c + dim(0.16)) as f32, cy as f32), color);
}

fn draw_link(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    // Two chain links
    draw_hollow_ellipse_mut(img, (c - dim(0.06), cy), dim(0.10), dim(0.06), color);
    draw_hollow_ellipse_mut(img, (c + dim(0.06), cy), dim(0.10), dim(0.06), color);
}

fn draw_tasks(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Checklist
    for i in 0..3 {
        let y = dim(0.22) + i * dim(0.12);
        // Checkbox
        draw_hollow_rect_mut(img, Rect::at(dim(0.28), y).of_size(dim(0.08) as u32, dim(0.08) as u32), color);
        // Line
        draw_line_segment_mut(img, (dim(0.42) as f32, (y + dim(0.04)) as f32), (dim(0.68) as f32, (y + dim(0.04)) as f32), color);
    }
    // Check mark in first box
    draw_line_segment_mut(img, (dim(0.30) as f32, (dim(0.22) + dim(0.04)) as f32),
        (dim(0.32) as f32, (dim(0.22) + dim(0.06)) as f32), color);
    draw_line_segment_mut(img, (dim(0.32) as f32, (dim(0.22) + dim(0.06)) as f32),
        (dim(0.36) as f32, dim(0.22) as f32), color);
}

fn draw_grid(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // 3x3 grid of dots
    for row in 0..3 {
        for col in 0..3 {
            let x = dim(0.30) + col * dim(0.14);
            let y = dim(0.22) + row * dim(0.12);
            draw_filled_circle_mut(img, (x, y), dim(0.03), color);
        }
    }
}

fn draw_power(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.36);
    draw_hollow_circle_mut(img, (c, cy), dim(0.14), color);
    // Line through top
    let bg = img.get_pixel(0, 0).clone();
    draw_filled_rect_mut(img, Rect::at(c - dim(0.02), dim(0.18)).of_size(dim(0.04) as u32, dim(0.12) as u32), bg);
    draw_filled_rect_mut(img, Rect::at(c - dim(0.01), dim(0.18)).of_size(dim(0.02) as u32, dim(0.20) as u32), color);
}

fn draw_refresh(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    draw_hollow_circle_mut(img, (c, cy), dim(0.14), color);
    // Arrow at top
    draw_line_segment_mut(img, ((c + dim(0.10)) as f32, (cy - dim(0.14)) as f32),
        ((c + dim(0.18)) as f32, (cy - dim(0.10)) as f32), color);
    draw_line_segment_mut(img, ((c + dim(0.10)) as f32, (cy - dim(0.14)) as f32),
        ((c + dim(0.04)) as f32, (cy - dim(0.08)) as f32), color);
}

fn draw_crosshair(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    let cy = dim(0.35);
    draw_hollow_circle_mut(img, (c, cy), dim(0.14), color);
    // Crosshair lines
    draw_line_segment_mut(img, ((c - dim(0.20)) as f32, cy as f32), ((c - dim(0.06)) as f32, cy as f32), color);
    draw_line_segment_mut(img, ((c + dim(0.06)) as f32, cy as f32), ((c + dim(0.20)) as f32, cy as f32), color);
    draw_line_segment_mut(img, (c as f32, (cy - dim(0.20)) as f32), (c as f32, (cy - dim(0.06)) as f32), color);
    draw_line_segment_mut(img, (c as f32, (cy + dim(0.06)) as f32), (c as f32, (cy + dim(0.20)) as f32), color);
}

fn draw_af(img: &mut RgbaImage, color: Rgba<u8>, c: i32, dim: impl Fn(f32) -> i32) {
    // Autofocus brackets
    let l = dim(0.24);
    let r = dim(0.56);
    let t = dim(0.20);
    let b = dim(0.50);
    let blen = dim(0.10);
    // Top-left
    draw_line_segment_mut(img, (l as f32, t as f32), ((l + blen) as f32, t as f32), color);
    draw_line_segment_mut(img, (l as f32, t as f32), (l as f32, (t + blen) as f32), color);
    // Top-right
    draw_line_segment_mut(img, (r as f32, t as f32), ((r - blen) as f32, t as f32), color);
    draw_line_segment_mut(img, (r as f32, t as f32), (r as f32, (t + blen) as f32), color);
    // Bottom-left
    draw_line_segment_mut(img, (l as f32, b as f32), ((l + blen) as f32, b as f32), color);
    draw_line_segment_mut(img, (l as f32, b as f32), (l as f32, (b - blen) as f32), color);
    // Bottom-right
    draw_line_segment_mut(img, (r as f32, b as f32), ((r - blen) as f32, b as f32), color);
    draw_line_segment_mut(img, (r as f32, b as f32), (r as f32, (b - blen) as f32), color);
}
