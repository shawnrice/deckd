use std::collections::HashMap;

use ab_glyph::{Font as AbFont, FontRef, PxScale, ScaleFont};
use elgato_streamdeck::StreamDeck;
use elgato_streamdeck::images::ImageRect;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use log::{error, warn};

use crate::config::{ButtonConfig, EncoderConfig};
use crate::dashboard::DashboardState;
use crate::tamagotchi::SharedPet;
use crate::timer::SharedTimer;

const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");
const FONT_BOLD_DATA: &[u8] = include_bytes!("../assets/font-bold.ttf");

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_DATA).unwrap_or_else(|e| {
        error!("Failed to load font: {}, falling back to bold", e);
        FontRef::try_from_slice(FONT_BOLD_DATA).expect("All embedded fonts are corrupt")
    })
}

fn font_bold() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_BOLD_DATA).unwrap_or_else(|e| {
        error!("Failed to load bold font: {}, falling back to regular", e);
        FontRef::try_from_slice(FONT_DATA).expect("All embedded fonts are corrupt")
    })
}

fn parse_hex(hex: &str) -> Rgba<u8> {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return Rgba([255, 255, 255, 255]);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    Rgba([r, g, b, 255])
}

fn blend(bg: Rgba<u8>, amount: f32) -> Rgba<u8> {
    let lighten = |c: u8, a: f32| (c as f32 + (255.0 - c as f32) * a).min(255.0) as u8;
    Rgba([
        lighten(bg.0[0], amount),
        lighten(bg.0[1], amount),
        lighten(bg.0[2], amount),
        255,
    ])
}

fn darken(c: Rgba<u8>, amount: f32) -> Rgba<u8> {
    Rgba([
        (c.0[0] as f32 * (1.0 - amount)) as u8,
        (c.0[1] as f32 * (1.0 - amount)) as u8,
        (c.0[2] as f32 * (1.0 - amount)) as u8,
        255,
    ])
}

/// Measure text width using glyph advances
fn text_width(text: &str, f: &FontRef, scale: PxScale) -> f32 {
    let scaled = f.as_scaled(scale);
    text.chars()
        .map(|c| {
            let glyph_id = scaled.glyph_id(c);
            scaled.h_advance(glyph_id)
        })
        .sum()
}

// ── Button rendering ──────────────────────────────────────────────

/// Render a beautiful button with accent bar, optional icon, and centered text
fn render_button(label: &str, icon_name: Option<&str>, size: u32, bg: Rgba<u8>, fg: Rgba<u8>) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 255]));

    // Main button area with 4px margin for separation
    let margin = 4_i32;
    let inner = size as i32 - margin * 2;
    draw_filled_rect_mut(
        &mut img,
        Rect::at(margin, margin).of_size(inner as u32, inner as u32),
        bg,
    );

    // Subtle top highlight (lighter stripe)
    let highlight = blend(bg, 0.15);
    draw_filled_rect_mut(
        &mut img,
        Rect::at(margin, margin).of_size(inner as u32, 3),
        highlight,
    );

    // Bottom accent bar using the foreground color
    let accent = fg;
    let bar_height = 4_u32;
    draw_filled_rect_mut(
        &mut img,
        Rect::at(margin, size as i32 - margin - bar_height as i32)
            .of_size(inner as u32, bar_height),
        accent,
    );

    // Subtle bottom shadow area
    let shadow = darken(bg, 0.3);
    draw_filled_rect_mut(
        &mut img,
        Rect::at(margin, size as i32 - margin - bar_height as i32 - 2)
            .of_size(inner as u32, 2),
        shadow,
    );

    // Draw icon if specified — load from assets/icons/<name>.png
    let has_icon = icon_name.is_some();
    if let Some(icon) = icon_name {
        let icon_path = format!(
            "{}/assets/icons/{}.png",
            env!("CARGO_MANIFEST_DIR"),
            icon
        );
        match image::open(&icon_path) {
            Ok(icon_img) => {
                let icon_size = (size as f32 * 0.55) as u32;
                let resized = icon_img.resize(icon_size, icon_size, image::imageops::FilterType::Lanczos3);
                let ox = ((size - resized.width()) / 2) as i64;
                let oy = (margin as u32 + 2) as i64;
                image::imageops::overlay(&mut img, &resized.to_rgba8(), ox, oy);
            }
            Err(e) => {
                warn!("Icon '{}' not found: {}, rendering text only", icon, e);
            }
        }
    }

    // Text rendering — use bold font, properly centered
    // When there's an icon, shift text to bottom third
    let f = font_bold();
    let text_scale = if has_icon { 0.17 } else { 0.22 };
    let scale = PxScale::from(size as f32 * text_scale);
    let tw = text_width(label, &f, scale);

    // If text is too wide, try smaller scale
    let (scale, tw) = if tw > (inner as f32 - 8.0) {
        let s = PxScale::from(size as f32 * 0.14);
        (s, text_width(label, &f, s))
    } else {
        (scale, tw)
    };

    let x = ((size as f32 - tw) / 2.0).max(margin as f32 + 2.0) as i32;
    let y = if has_icon {
        // Below icon, above accent bar
        (size as f32 * 0.62) as i32
    } else {
        let scaled = f.as_scaled(scale);
        let ascent = scaled.ascent();
        ((size as f32 - ascent) / 2.0 - 4.0).max(margin as f32) as i32
    };

    draw_text_mut(&mut img, fg, x, y, scale, &f, label);

    DynamicImage::ImageRgba8(img)
}

pub fn render_buttons(deck: &mut StreamDeck, buttons: &HashMap<String, ButtonConfig>) {
    let key_count = deck.kind().key_count();
    let key_size = 120_u32;

    deck.clear_all_button_images().ok();

    for (key_str, button) in buttons {
        let key: u8 = match key_str.parse() {
            Ok(k) if k < key_count => k,
            _ => {
                warn!("Invalid button key: {}", key_str);
                continue;
            }
        };

        if let Some(icon_path) = &button.icon {
            match image::open(icon_path) {
                Ok(img) => {
                    // Compose icon onto a dark background, centered, with padding
                    let bg_color = button.bg_color.as_deref().map(parse_hex).unwrap_or(Rgba([10, 10, 18, 255]));
                    let mut canvas = RgbaImage::from_pixel(key_size, key_size, bg_color);
                    let pad = 12_u32;
                    let icon_size = key_size - pad * 2;
                    let resized = img.resize(icon_size, icon_size, image::imageops::FilterType::Lanczos3);
                    let ox = (key_size - resized.width()) / 2;
                    let oy = (key_size - resized.height()) / 2;
                    image::imageops::overlay(&mut canvas, &resized.to_rgba8(), ox as i64, oy as i64);
                    if let Err(e) = deck.set_button_image(key, DynamicImage::ImageRgba8(canvas)) {
                        error!("Failed to set image for key {}: {}", key, e);
                    }
                }
                Err(e) => error!("Failed to load icon '{}': {}", icon_path, e),
            }
        } else if let Some(label) = &button.label {
            let bg = button
                .bg_color
                .as_deref()
                .map(parse_hex)
                .unwrap_or(Rgba([20, 20, 35, 255]));
            let fg = button
                .fg_color
                .as_deref()
                .map(parse_hex)
                .unwrap_or(Rgba([220, 220, 220, 255]));

            let icon_name = button.icon_name.as_deref();
            let img = render_button(label, icon_name, key_size, bg, fg);
            if let Err(e) = deck.set_button_image(key, img) {
                error!("Failed to set label for key {}: {}", key, e);
            }
        }
    }

    if let Err(e) = deck.flush() {
        error!("Failed to flush button images: {}", e);
    }
}

// ── LCD strip rendering ───────────────────────────────────────────

/// Render a single encoder segment on the LCD strip
fn render_lcd_segment(label: &str, value: Option<&str>, width: u32, height: u32) -> RgbaImage {
    let bg = Rgba([12, 12, 20, 255]);
    let mut img = RgbaImage::from_pixel(width, height, bg);

    let f = font();
    let fb = font_bold();

    // Separator line on the right edge
    let sep_color = Rgba([40, 40, 55, 255]);
    draw_filled_rect_mut(
        &mut img,
        Rect::at(width as i32 - 1, 8).of_size(1, height - 16),
        sep_color,
    );

    if let Some(val) = value {
        // Two-line layout: value (large, bold) on top, label (small) below
        let val_scale = PxScale::from(32.0);
        let val_tw = text_width(val, &fb, val_scale);
        let val_x = ((width as f32 - val_tw) / 2.0).max(4.0) as i32;
        draw_text_mut(&mut img, Rgba([255, 255, 255, 255]), val_x, 16, val_scale, &fb, val);

        let label_scale = PxScale::from(16.0);
        let label_tw = text_width(label, &f, label_scale);
        let label_x = ((width as f32 - label_tw) / 2.0).max(4.0) as i32;
        draw_text_mut(&mut img, Rgba([120, 120, 150, 255]), label_x, 62, label_scale, &f, label);
    } else {
        // Single label centered
        let label_scale = PxScale::from(22.0);
        let label_tw = text_width(label, &fb, label_scale);
        let label_x = ((width as f32 - label_tw) / 2.0).max(4.0) as i32;
        let label_y = ((height as f32 - 22.0) / 2.0) as i32;
        draw_text_mut(&mut img, Rgba([180, 180, 200, 255]), label_x, label_y, label_scale, &fb, label);
    }

    img
}

pub fn render_lcd_strip(deck: &mut StreamDeck, encoders: &HashMap<String, EncoderConfig>) {
    let segment_width = 200_u32;
    let strip_height = 100_u32;

    for (key_str, encoder) in encoders {
        let idx: u8 = match key_str.parse() {
            Ok(k) if k < 4 => k,
            _ => continue,
        };

        if let Some(label) = &encoder.label {
            let img = render_lcd_segment(label, None, segment_width, strip_height);
            write_lcd_segment(deck, idx, &img, segment_width);
        }
    }
}

/// Render the LCD strip with live dashboard data
pub fn render_lcd_dashboard(
    deck: &mut StreamDeck,
    encoders: &HashMap<String, EncoderConfig>,
    dashboard: &DashboardState,
    timer: &SharedTimer,
    pet: &SharedPet,
) {
    // Check for active notifications — render banner if any are fresh (< 5 seconds)
    let notification_duration = std::time::Duration::from_secs(5);
    if let Some(notif) = dashboard.notifications.last()
        && notif.created.elapsed() < notification_duration
    {
        render_notification_banner(deck, &notif.message, notif.created);
        return;
    }

    let seg_w = 200_u32;
    let strip_h = 100_u32;

    let paused_recently = dashboard.now_playing_state == "paused"
        && !dashboard.now_playing_title.is_empty()
        && dashboard.now_playing_changed.elapsed() < std::time::Duration::from_secs(300);

    let has_music = dashboard.now_playing_state == "playing" || paused_recently;

    let timer_display = timer.lock().ok().and_then(|t| {
        if t.is_running() { Some(t.display()) } else { None }
    });

    // Segment 0: Volume / Mute
    {
        let (l, v) = if dashboard.mic_muted {
            ("MUTED", "MIC".to_string())
        } else {
            ("Volume", format!("{}%", &dashboard.volume))
        };
        let img = render_lcd_segment(l, Some(&v), seg_w, strip_h);
        write_lcd_segment(deck, 0, &img, seg_w);
    }

    // Segment 1: Audio device
    {
        let (l, v) = if let Some((ref input_name, when)) = dashboard.input_flash {
            if when.elapsed() < std::time::Duration::from_secs(2) {
                ("Input", shorten_device_name(input_name))
            } else {
                ("Output", shorten_device_name(&dashboard.audio_output))
            }
        } else {
            ("Output", shorten_device_name(&dashboard.audio_output))
        };
        let img = render_lcd_segment(l, Some(&v), seg_w, strip_h);
        write_lcd_segment(deck, 1, &img, seg_w);
    }

    // Segments 2-3: Now playing (wide) OR two separate info segments
    if has_music {
        let is_playing = dashboard.now_playing_state == "playing";
        let img = render_now_playing(
            &dashboard.now_playing_title,
            &dashboard.now_playing_artist,
            is_playing,
            seg_w * 2,
            strip_h,
        );
        // Write as two segments
        let left = img.view(0, 0, seg_w, strip_h).to_image();
        let right = img.view(seg_w, 0, seg_w, strip_h).to_image();
        write_lcd_segment(deck, 2, &left, seg_w);
        write_lcd_segment(deck, 3, &right, seg_w);
    } else {
        // Wide pet scene (400x100) with info overlay
        // Timer/meeting info appears overlaid in the scene
        let info_text = if let Some(ref time_str) = timer_display {
            Some(format!("⏱ {}", time_str))
        } else if dashboard.next_meeting_mins >= 0 && dashboard.next_meeting_mins <= 60 {
            Some(format!("{} in {}m", truncate(&dashboard.next_meeting, 12), dashboard.next_meeting_mins))
        } else if dashboard.review_requests > 0 {
            Some(format!("{} reviews waiting", dashboard.review_requests))
        } else if dashboard.mergeable_count > 0 {
            Some(format!("{} ready to merge", dashboard.mergeable_count))
        } else {
            None
        };

        if let Ok(p) = pet.lock() {
            let img = render_pet_wide(&p, info_text.as_deref(), seg_w * 2, strip_h);
            let left = img.view(0, 0, seg_w, strip_h).to_image();
            let right = img.view(seg_w, 0, seg_w, strip_h).to_image();
            write_lcd_segment(deck, 2, &left, seg_w);
            write_lcd_segment(deck, 3, &right, seg_w);
        }
    }
}

fn write_lcd_segment(deck: &mut StreamDeck, idx: u8, img: &RgbaImage, segment_width: u32) {
    let x_offset = idx as u16 * segment_width as u16;
    let dyn_img = DynamicImage::ImageRgba8(img.clone());
    match ImageRect::from_image(dyn_img) {
        Ok(rect) => {
            if let Err(e) = deck.write_lcd(x_offset, 0, &rect) {
                error!("Failed to write LCD segment {}: {}", idx, e);
            }
        }
        Err(e) => error!("Failed to convert LCD image {}: {}", idx, e),
    }
}

/// Render a wide now-playing display across two LCD segments
fn render_now_playing(title: &str, artist: &str, is_playing: bool, width: u32, height: u32) -> RgbaImage {
    let bg = Rgba([12, 12, 20, 255]);
    let mut img = RgbaImage::from_pixel(width, height, bg);

    let fb = font_bold();
    let f = font();

    // Accent line on top — purple for music
    let accent = if is_playing {
        Rgba([123, 104, 238, 255]) // medium slate blue
    } else {
        Rgba([80, 80, 100, 255])   // dim when paused
    };
    draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(width, 2), accent);

    // Play/pause indicator
    let state_icon = if is_playing { "▶" } else { "❚❚" };
    let icon_scale = PxScale::from(18.0);
    draw_text_mut(&mut img, accent, 12, 38, icon_scale, &fb, state_icon);

    // Track title — large
    let title_scale = PxScale::from(30.0);
    let display_title = truncate(title, 28);
    let tw = text_width(&display_title, &fb, title_scale);

    // If still too wide, shrink
    let (title_scale, display_title) = if tw > (width as f32 - 50.0) {
        let s = PxScale::from(24.0);
        (s, truncate(title, 24))
    } else {
        (title_scale, display_title)
    };

    draw_text_mut(&mut img, Rgba([240, 240, 255, 255]), 36, 14, title_scale, &fb, &display_title);

    // Artist — smaller, dimmer
    let artist_scale = PxScale::from(20.0);
    let display_artist = truncate(artist, 32);
    draw_text_mut(&mut img, Rgba([140, 140, 170, 255]), 36, 56, artist_scale, &f, &display_artist);

    // Separator from segment 1
    let sep = Rgba([40, 40, 55, 255]);
    draw_filled_rect_mut(&mut img, Rect::at(0, 8).of_size(1, height - 16), sep);

    img
}

fn render_notification_banner(deck: &mut StreamDeck, message: &str, created: std::time::Instant) {
    let width = 800_u32;
    let height = 100_u32;

    // Pulse effect — accent color fades over time
    let age = created.elapsed().as_secs_f32();
    let pulse = ((age * 3.0).sin() * 0.3 + 0.7).clamp(0.4, 1.0);

    let accent_r = (76.0 * pulse) as u8;
    let accent_g = (201.0 * pulse) as u8;
    let accent_b = (240.0 * pulse) as u8;

    let bg = Rgba([15, 15, 30, 255]);
    let accent = Rgba([accent_r, accent_g, accent_b, 255]);

    let mut img = RgbaImage::from_pixel(width, height, bg);

    // Top accent line
    draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(width, 3), accent);
    // Bottom accent line
    draw_filled_rect_mut(&mut img, Rect::at(0, height as i32 - 3).of_size(width, 3), accent);

    // Bell/notification icon (simple circle with dot)
    let icon_x = 30_i32;
    let icon_y = 50_i32;
    imageproc::drawing::draw_hollow_circle_mut(&mut img, (icon_x, icon_y), 14, accent);
    imageproc::drawing::draw_filled_circle_mut(&mut img, (icon_x, icon_y - 2), 4, accent);
    // Bell clapper
    imageproc::drawing::draw_filled_circle_mut(&mut img, (icon_x, icon_y + 14), 3, accent);

    // Message text — large, centered
    let fb = font_bold();
    let scale = PxScale::from(28.0);
    let tw = text_width(message, &fb, scale);

    // If too wide, shrink
    let (scale, tw) = if tw > (width as f32 - 100.0) {
        let s = PxScale::from(22.0);
        (s, text_width(message, &fb, s))
    } else {
        (scale, tw)
    };

    let text_x = (60.0 + (width as f32 - 60.0 - tw) / 2.0) as i32;
    let text_y = 35;
    draw_text_mut(&mut img, Rgba([255, 255, 255, 255]), text_x, text_y, scale, &fb, message);

    // Write full strip
    let dyn_img = DynamicImage::ImageRgba8(img);
    if let Ok(rect) = ImageRect::from_image(dyn_img) {
        deck.write_lcd(0, 0, &rect).ok();
    }
}

/// Full-width pet scene for dedicated pet pages (800x100)
pub fn render_pet_lcd(
    deck: &mut StreamDeck,
    pet: &crate::tamagotchi::Pet,
    width: u32,
    height: u32,
) {
    let img = render_pet_wide(pet, None, width, height);
    // Write as 4 segments
    let seg_w = 200;
    for i in 0..4u8 {
        let seg = img.view(i as u32 * seg_w, 0, seg_w, height).to_image();
        write_lcd_segment(deck, i, &seg, seg_w);
    }
}

#[allow(dead_code)]
fn render_pet_segment_old(pet: &crate::tamagotchi::Pet, width: u32, height: u32) -> RgbaImage {
    let bg = Rgba([12, 12, 20, 255]);
    let mut img = RgbaImage::from_pixel(width, height, bg);

    let fb = font_bold();
    let f = font();

    // Separator
    draw_filled_rect_mut(
        &mut img,
        Rect::at(width as i32 - 1, 8).of_size(1, height - 16),
        Rgba([40, 40, 55, 255]),
    );

    // Pet face — large, centered
    let face = pet.sprite();
    let face_scale = PxScale::from(28.0);
    let face_tw = text_width(face, &fb, face_scale);
    let face_x = ((width as f32 - face_tw) / 2.0).max(4.0) as i32;

    // Color based on mood
    let face_color = match pet.mood {
        crate::tamagotchi::Mood::Happy => Rgba([100, 255, 100, 255]),
        crate::tamagotchi::Mood::Excited => Rgba([255, 215, 0, 255]),
        crate::tamagotchi::Mood::Sad => Rgba([100, 100, 200, 255]),
        crate::tamagotchi::Mood::Hungry => Rgba([255, 140, 60, 255]),
        crate::tamagotchi::Mood::Sleeping => Rgba([120, 120, 160, 255]),
        crate::tamagotchi::Mood::Coding => Rgba([76, 201, 240, 255]),
        crate::tamagotchi::Mood::Neutral => Rgba([180, 180, 200, 255]),
    };
    draw_text_mut(&mut img, face_color, face_x, 10, face_scale, &fb, face);

    // Status line — small
    let status = pet.status();
    let status_scale = PxScale::from(14.0);
    let status_tw = text_width(&status, &f, status_scale);
    let status_x = ((width as f32 - status_tw) / 2.0).max(4.0) as i32;
    draw_text_mut(&mut img, Rgba([120, 120, 150, 255]), status_x, 50, status_scale, &f, &status);

    // Stat bars — tiny at the bottom
    let bar_y = 72;
    let bar_w = (width - 16) as i32;
    let bar_h = 4;

    // Happiness bar (green)
    let hp_filled = (pet.happiness as i32 * bar_w / 100).max(1);
    draw_filled_rect_mut(&mut img, Rect::at(8, bar_y).of_size(bar_w as u32, bar_h as u32), Rgba([30, 30, 45, 255]));
    draw_filled_rect_mut(&mut img, Rect::at(8, bar_y).of_size(hp_filled as u32, bar_h as u32), Rgba([80, 200, 80, 255]));

    // Hunger bar (orange) — fills up as hunger increases
    let hunger_filled = (pet.hunger as i32 * bar_w / 100).max(1);
    draw_filled_rect_mut(&mut img, Rect::at(8, bar_y + 8).of_size(bar_w as u32, bar_h as u32), Rgba([30, 30, 45, 255]));
    draw_filled_rect_mut(&mut img, Rect::at(8, bar_y + 8).of_size(hunger_filled as u32, bar_h as u32), Rgba([200, 120, 40, 255]));

    img
}

/// Render the pet in a wide 400x100 scene with pixel art
fn render_pet_wide(
    pet: &crate::tamagotchi::Pet,
    info: Option<&str>,
    width: u32,
    height: u32,
) -> RgbaImage {
    use crate::tamagotchi::{Action, Mood, Species};
    use imageproc::drawing::{draw_filled_circle_mut, draw_hollow_circle_mut};

    let bg = Rgba([10, 10, 22, 255]);
    let mut img = RgbaImage::from_pixel(width, height, bg);
    let f = font();
    let fb = font_bold();

    // Ground line
    let ground_y = 72_i32;
    draw_filled_rect_mut(
        &mut img,
        Rect::at(0, ground_y).of_size(width, 2),
        Rgba([30, 30, 50, 255]),
    );

    // Pet position — moves based on action
    let frame = pet.sprite().len() as i32; // use sprite len as frame proxy
    let base_x = match pet.action {
        Action::Walking => {
            let cycle = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() / 400) as i32;
            let range = (width as i32 / 2) - 40; // use most of the available width
            let center = width as i32 / 4;
            center + ((cycle % 16) - 8).abs() * range / 8
        }
        Action::Dancing => {
            let cycle = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() / 300) as i32;
            160 + (cycle % 4 - 2) * 8 // small bounce
        }
        _ => 160, // centered-ish
    };

    let blink = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() / 100) as u32;
    let is_blink_frame = blink % 30 == 0; // blink every ~3 seconds

    // Colors
    let body_color = match pet.species {
        Species::Cat => Rgba([255, 180, 100, 255]),     // orange tabby
        Species::Dog => Rgba([180, 140, 100, 255]),     // golden
        Species::Penguin => Rgba([60, 60, 80, 255]),    // dark blue-grey
        Species::Ghost => Rgba([200, 200, 240, 180]),   // translucent white
    };
    let accent_color = match pet.species {
        Species::Cat => Rgba([255, 220, 180, 255]),     // lighter belly
        Species::Dog => Rgba([220, 190, 150, 255]),
        Species::Penguin => Rgba([240, 240, 255, 255]), // white belly
        Species::Ghost => Rgba([230, 230, 255, 200]),
    };
    let eye_color = Rgba([30, 30, 40, 255]);
    let nose_color = match pet.species {
        Species::Cat | Species::Dog => Rgba([200, 100, 100, 255]),
        Species::Penguin => Rgba([255, 180, 50, 255]),  // orange beak
        Species::Ghost => Rgba([100, 100, 160, 255]),
    };

    let cx = base_x;
    let cy = ground_y - 20; // body center

    match pet.species {
        Species::Cat => {
            // Body — oval
            draw_filled_circle_mut(&mut img, (cx, cy), 16, body_color);
            draw_filled_circle_mut(&mut img, (cx, cy + 4), 14, body_color);
            // Belly
            draw_filled_circle_mut(&mut img, (cx, cy + 6), 10, accent_color);
            // Head
            draw_filled_circle_mut(&mut img, (cx, cy - 14), 12, body_color);
            // Ears (triangles via small circles)
            draw_filled_circle_mut(&mut img, (cx - 10, cy - 24), 5, body_color);
            draw_filled_circle_mut(&mut img, (cx + 10, cy - 24), 5, body_color);
            draw_filled_circle_mut(&mut img, (cx - 10, cy - 24), 3, Rgba([255, 140, 120, 255]));
            draw_filled_circle_mut(&mut img, (cx + 10, cy - 24), 3, Rgba([255, 140, 120, 255]));
            // Eyes
            if is_blink_frame || pet.mood == Mood::Sleeping {
                // Closed eyes — happy lines
                draw_filled_rect_mut(&mut img, Rect::at(cx - 7, cy - 15).of_size(4, 2), eye_color);
                draw_filled_rect_mut(&mut img, Rect::at(cx + 3, cy - 15).of_size(4, 2), eye_color);
            } else {
                draw_filled_circle_mut(&mut img, (cx - 5, cy - 15), 3, eye_color);
                draw_filled_circle_mut(&mut img, (cx + 5, cy - 15), 3, eye_color);
                // Pupils — look direction based on action
                let pupil_offset = match pet.action {
                    Action::LookingAround => if blink % 6 < 3 { -1 } else { 1 },
                    Action::Walking => if cx > 200 { 1 } else { -1 },
                    _ => 0,
                };
                draw_filled_circle_mut(&mut img, (cx - 5 + pupil_offset, cy - 15), 1, Rgba([255, 255, 255, 255]));
                draw_filled_circle_mut(&mut img, (cx + 5 + pupil_offset, cy - 15), 1, Rgba([255, 255, 255, 255]));
            }
            // Nose
            draw_filled_circle_mut(&mut img, (cx, cy - 11), 2, nose_color);
            // Whiskers
            draw_filled_rect_mut(&mut img, Rect::at(cx - 16, cy - 12).of_size(8, 1), Rgba([200, 160, 120, 255]));
            draw_filled_rect_mut(&mut img, Rect::at(cx + 8, cy - 12).of_size(8, 1), Rgba([200, 160, 120, 255]));
            // Tail
            let tail_wave = if blink % 4 < 2 { 0 } else { 3 };
            draw_filled_circle_mut(&mut img, (cx + 20, cy - 2 + tail_wave), 3, body_color);
            draw_filled_circle_mut(&mut img, (cx + 24, cy - 6 + tail_wave), 3, body_color);
            draw_filled_circle_mut(&mut img, (cx + 26, cy - 10 + tail_wave), 3, body_color);
            // Feet
            draw_filled_circle_mut(&mut img, (cx - 8, cy + 14), 4, body_color);
            draw_filled_circle_mut(&mut img, (cx + 8, cy + 14), 4, body_color);
        }
        Species::Dog => {
            // Body
            draw_filled_circle_mut(&mut img, (cx, cy), 16, body_color);
            draw_filled_circle_mut(&mut img, (cx, cy + 4), 14, body_color);
            draw_filled_circle_mut(&mut img, (cx, cy + 6), 10, accent_color);
            // Head
            draw_filled_circle_mut(&mut img, (cx, cy - 14), 13, body_color);
            // Floppy ears
            draw_filled_circle_mut(&mut img, (cx - 13, cy - 10), 6, Rgba([150, 110, 70, 255]));
            draw_filled_circle_mut(&mut img, (cx + 13, cy - 10), 6, Rgba([150, 110, 70, 255]));
            // Snout
            draw_filled_circle_mut(&mut img, (cx, cy - 10), 6, accent_color);
            // Eyes
            if is_blink_frame || pet.mood == Mood::Sleeping {
                draw_filled_rect_mut(&mut img, Rect::at(cx - 7, cy - 17).of_size(4, 2), eye_color);
                draw_filled_rect_mut(&mut img, Rect::at(cx + 3, cy - 17).of_size(4, 2), eye_color);
            } else {
                draw_filled_circle_mut(&mut img, (cx - 5, cy - 16), 3, eye_color);
                draw_filled_circle_mut(&mut img, (cx + 5, cy - 16), 3, eye_color);
                draw_filled_circle_mut(&mut img, (cx - 4, cy - 16), 1, Rgba([255, 255, 255, 255]));
                draw_filled_circle_mut(&mut img, (cx + 6, cy - 16), 1, Rgba([255, 255, 255, 255]));
            }
            // Nose
            draw_filled_circle_mut(&mut img, (cx, cy - 8), 3, Rgba([40, 40, 40, 255]));
            // Tongue when happy
            if pet.mood == Mood::Happy || pet.mood == Mood::Excited {
                draw_filled_circle_mut(&mut img, (cx + 2, cy - 4), 3, Rgba([255, 120, 120, 255]));
            }
            // Tail — wagging
            let wag = if blink % 4 < 2 { -4 } else { 4 };
            draw_filled_circle_mut(&mut img, (cx + 18 + wag, cy - 8), 4, body_color);
            draw_filled_circle_mut(&mut img, (cx + 22 + wag, cy - 14), 3, body_color);
            // Feet
            draw_filled_circle_mut(&mut img, (cx - 8, cy + 14), 4, body_color);
            draw_filled_circle_mut(&mut img, (cx + 8, cy + 14), 4, body_color);
        }
        Species::Penguin => {
            // Body — tall oval
            draw_filled_circle_mut(&mut img, (cx, cy - 2), 16, body_color);
            draw_filled_circle_mut(&mut img, (cx, cy + 6), 14, body_color);
            // White belly
            draw_filled_circle_mut(&mut img, (cx, cy + 2), 10, accent_color);
            // Head
            draw_filled_circle_mut(&mut img, (cx, cy - 16), 11, body_color);
            // Eyes
            if is_blink_frame || pet.mood == Mood::Sleeping {
                draw_filled_rect_mut(&mut img, Rect::at(cx - 6, cy - 17).of_size(3, 2), Rgba([255, 255, 255, 255]));
                draw_filled_rect_mut(&mut img, Rect::at(cx + 3, cy - 17).of_size(3, 2), Rgba([255, 255, 255, 255]));
            } else {
                draw_filled_circle_mut(&mut img, (cx - 4, cy - 17), 3, Rgba([255, 255, 255, 255]));
                draw_filled_circle_mut(&mut img, (cx + 4, cy - 17), 3, Rgba([255, 255, 255, 255]));
                draw_filled_circle_mut(&mut img, (cx - 4, cy - 17), 2, eye_color);
                draw_filled_circle_mut(&mut img, (cx + 4, cy - 17), 2, eye_color);
            }
            // Beak
            draw_filled_circle_mut(&mut img, (cx, cy - 12), 3, nose_color);
            // Flippers
            let flap = if pet.action == Action::Dancing { if blink % 4 < 2 { -3 } else { 3 } } else { 0 };
            draw_filled_circle_mut(&mut img, (cx - 16, cy - 2 + flap), 5, body_color);
            draw_filled_circle_mut(&mut img, (cx + 16, cy - 2 - flap), 5, body_color);
            // Feet
            draw_filled_circle_mut(&mut img, (cx - 6, cy + 16), 4, nose_color);
            draw_filled_circle_mut(&mut img, (cx + 6, cy + 16), 4, nose_color);
        }
        Species::Ghost => {
            // Floaty body — bobs up and down
            let bob = if blink % 6 < 3 { -2 } else { 2 };
            let gy = cy + bob;
            // Body — rounded top, wavy bottom
            draw_filled_circle_mut(&mut img, (cx, gy - 6), 18, body_color);
            draw_filled_rect_mut(&mut img, Rect::at(cx - 18, gy - 6).of_size(36, 20), body_color);
            // Wavy bottom
            for i in 0..4 {
                let wave = if (i + blink as i32 / 3) % 2 == 0 { 0 } else { 4 };
                draw_filled_circle_mut(&mut img, (cx - 14 + i * 10, gy + 14 + wave), 5, body_color);
            }
            // Eyes
            draw_filled_circle_mut(&mut img, (cx - 6, gy - 8), 4, eye_color);
            draw_filled_circle_mut(&mut img, (cx + 6, gy - 8), 4, eye_color);
            draw_filled_circle_mut(&mut img, (cx - 5, gy - 9), 2, Rgba([255, 255, 255, 200]));
            draw_filled_circle_mut(&mut img, (cx + 7, gy - 9), 2, Rgba([255, 255, 255, 200]));
            // Mouth
            if pet.mood == Mood::Happy || pet.mood == Mood::Excited {
                draw_filled_circle_mut(&mut img, (cx, gy - 2), 3, Rgba([40, 40, 60, 180]));
            }
        }
    }

    // Mood decorations
    match pet.mood {
        Mood::Happy | Mood::Excited => {
            // Little hearts or stars floating
            let sparkle_y = cy - 30 - (blink % 8) as i32;
            if pet.mood == Mood::Excited {
                draw_filled_circle_mut(&mut img, (cx + 20, sparkle_y), 2, Rgba([255, 215, 0, 255]));
                draw_filled_circle_mut(&mut img, (cx - 18, sparkle_y + 5), 2, Rgba([255, 215, 0, 255]));
                draw_filled_circle_mut(&mut img, (cx + 28, sparkle_y + 8), 1, Rgba([255, 215, 0, 255]));
            } else {
                draw_filled_circle_mut(&mut img, (cx + 22, sparkle_y), 3, Rgba([255, 100, 120, 200]));
            }
        }
        Mood::Sleeping => {
            // Zzz bubbles
            let f = font_bold();
            let z_x = cx + 20;
            let z_y = cy - 30 - (blink % 6) as i32;
            draw_text_mut(&mut img, Rgba([120, 120, 180, 200]), z_x, z_y, PxScale::from(14.0), &f, "z");
            draw_text_mut(&mut img, Rgba([100, 100, 160, 180]), z_x + 8, z_y - 8, PxScale::from(12.0), &f, "z");
            draw_text_mut(&mut img, Rgba([80, 80, 140, 150]), z_x + 14, z_y - 14, PxScale::from(10.0), &f, "z");
        }
        Mood::Hungry => {
            // Sweat drop
            draw_filled_circle_mut(&mut img, (cx + 14, cy - 26), 3, Rgba([100, 150, 255, 200]));
            draw_filled_circle_mut(&mut img, (cx + 14, cy - 30), 2, Rgba([100, 150, 255, 180]));
        }
        Mood::Sad => {
            // Tear drop
            draw_filled_circle_mut(&mut img, (cx - 8, cy - 10), 2, Rgba([100, 150, 255, 220]));
        }
        _ => {}
    }

    // Action-specific decorations
    if pet.action == Action::Typing {
        // Little laptop/keyboard in front
        draw_filled_rect_mut(&mut img, Rect::at(cx - 20, cy + 8).of_size(16, 10), Rgba([60, 60, 80, 255]));
        draw_filled_rect_mut(&mut img, Rect::at(cx - 19, cy + 9).of_size(14, 6), Rgba([80, 120, 180, 255]));
        // Blinking cursor
        if blink % 4 < 2 {
            draw_filled_rect_mut(&mut img, Rect::at(cx - 14, cy + 11).of_size(2, 3), Rgba([200, 255, 200, 255]));
        }
    }

    if pet.action == Action::Eating {
        // Food item
        draw_filled_circle_mut(&mut img, (cx - 24, cy + 6), 4, Rgba([100, 200, 100, 255]));
        draw_filled_circle_mut(&mut img, (cx - 24, cy + 3), 3, Rgba([80, 180, 80, 255]));
    }

    if pet.action == Action::Celebrating {
        // Confetti!
        for i in 0..6 {
            let conf_x = cx - 30 + (i * 17 + blink as i32 * 3) % 80;
            let conf_y = 5 + (i * 13 + blink as i32 * 2) % 30;
            let colors = [
                Rgba([255, 100, 100, 255]),
                Rgba([100, 255, 100, 255]),
                Rgba([100, 100, 255, 255]),
                Rgba([255, 255, 100, 255]),
                Rgba([255, 100, 255, 255]),
                Rgba([100, 255, 255, 255]),
            ];
            draw_filled_rect_mut(&mut img, Rect::at(conf_x, conf_y).of_size(3, 3), colors[i as usize % 6]);
        }
    }

    // Status text — right side
    let status = pet.status();
    let status_scale = PxScale::from(14.0);
    draw_text_mut(
        &mut img,
        Rgba([100, 100, 130, 255]),
        280, 8,
        status_scale,
        &f,
        &status,
    );

    // Name + level
    let name_str = format!("{}", pet.name);
    draw_text_mut(
        &mut img,
        Rgba([180, 180, 210, 255]),
        280, 26,
        PxScale::from(16.0),
        &fb,
        &name_str,
    );

    // Stat bars on the right
    let bar_x = 280_i32;
    let bar_w = 100_i32;
    let bar_h = 6;

    // Happiness (green)
    let hp = (pet.happiness as i32 * bar_w / 100).max(1);
    draw_text_mut(&mut img, Rgba([80, 80, 100, 255]), bar_x, 44, PxScale::from(10.0), &f, "♡");
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 46).of_size(bar_w as u32, bar_h as u32), Rgba([25, 25, 40, 255]));
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 46).of_size(hp as u32, bar_h as u32), Rgba([80, 200, 80, 255]));

    // Hunger (orange, fills up)
    let hunger = (pet.hunger as i32 * bar_w / 100).max(1);
    draw_text_mut(&mut img, Rgba([80, 80, 100, 255]), bar_x, 56, PxScale::from(10.0), &f, "◆");
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 58).of_size(bar_w as u32, bar_h as u32), Rgba([25, 25, 40, 255]));
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 58).of_size(hunger as u32, bar_h as u32), Rgba([200, 120, 40, 255]));

    // XP bar
    let xp_in_level = pet.xp % 100;
    let xp_fill = (xp_in_level as i32 * bar_w / 100).max(1);
    draw_text_mut(&mut img, Rgba([80, 80, 100, 255]), bar_x, 68, PxScale::from(10.0), &f, "★");
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 70).of_size(bar_w as u32, bar_h as u32), Rgba([25, 25, 40, 255]));
    draw_filled_rect_mut(&mut img, Rect::at(bar_x + 12, 70).of_size(xp_fill as u32, bar_h as u32), Rgba([120, 100, 220, 255]));

    // Info overlay text (timer, meeting, reviews) at top-right
    if let Some(info) = info {
        draw_text_mut(
            &mut img,
            Rgba([200, 200, 220, 255]),
            280, 82,
            PxScale::from(13.0),
            &f,
            info,
        );
    }

    // Separator on left edge
    draw_filled_rect_mut(
        &mut img,
        Rect::at(0, 8).of_size(1, height - 16),
        Rgba([40, 40, 55, 255]),
    );

    img
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a clean break point
        let end = s.char_indices()
            .take(max - 1)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max - 1);
        format!("{}..", &s[..end])
    }
}

fn shorten_device_name(name: &str) -> String {
    // Shorten common device names to fit the LCD segment
    let name = name.trim();
    if name.contains("MacBook Pro Speakers") {
        return "Speakers".into();
    }
    if name.contains("HyperX") {
        return "HyperX".into();
    }
    if name.contains("BenQ") {
        return "BenQ".into();
    }
    if name.contains("ZoomAudio") {
        return "Zoom".into();
    }
    // Truncate long names
    if name.len() > 12 {
        format!("{}...", &name[..10])
    } else {
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_valid_color() {
        let c = parse_hex("#FF8800");
        assert_eq!(c, Rgba([255, 136, 0, 255]));
    }

    #[test]
    fn parse_hex_no_hash() {
        let c = parse_hex("00FF00");
        assert_eq!(c, Rgba([0, 255, 0, 255]));
    }

    #[test]
    fn parse_hex_short_falls_back_to_white() {
        let c = parse_hex("#FFF");
        assert_eq!(c, Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn parse_hex_black() {
        let c = parse_hex("#000000");
        assert_eq!(c, Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_adds_dots() {
        let result = truncate("a very long string here", 10);
        assert!(result.len() <= 12); // 9 chars + ".."
        assert!(result.ends_with(".."));
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate("12345", 5), "12345");
    }

    #[test]
    fn shorten_device_name_macbook_speakers() {
        assert_eq!(shorten_device_name("MacBook Pro Speakers"), "Speakers");
    }

    #[test]
    fn shorten_device_name_hyperx() {
        assert_eq!(shorten_device_name("HyperX Cloud II Wireless"), "HyperX");
    }

    #[test]
    fn shorten_device_name_benq() {
        assert_eq!(shorten_device_name("BenQ EW3270U"), "BenQ");
    }

    #[test]
    fn shorten_device_name_zoom() {
        assert_eq!(shorten_device_name("ZoomAudioDevice"), "Zoom");
    }

    #[test]
    fn shorten_device_name_long_unknown() {
        let result = shorten_device_name("VeryLongDeviceName");
        assert!(result.ends_with("..."));
        assert!(result.len() <= 13);
    }

    #[test]
    fn shorten_device_name_short_passthrough() {
        assert_eq!(shorten_device_name("AirPods"), "AirPods");
    }
}
