use ab_glyph::{FontRef, PxScale};
use elgato_streamdeck::StreamDeck;
use elgato_streamdeck::images::ImageRect;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use log::error;

const FONT_BOLD_DATA: &[u8] = include_bytes!("../assets/font-bold.ttf");
const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

fn font_bold() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_BOLD_DATA).expect("font")
}

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_DATA).expect("font")
}

const STRIP_W: u32 = 800;
const STRIP_H: u32 = 100;

const BG: Rgba<u8> = Rgba([8, 8, 16, 255]);
const BAR_BG: Rgba<u8> = Rgba([30, 30, 45, 255]);
const BAR_FG: Rgba<u8> = Rgba([76, 201, 240, 255]); // #4cc9f0
const TEXT_DIM: Rgba<u8> = Rgba([100, 100, 130, 255]);
const TEXT_BRIGHT: Rgba<u8> = Rgba([220, 220, 240, 255]);

/// Render a boot frame: progress bar + status text + "deckd" title
pub fn render_boot_frame(deck: &mut StreamDeck, progress: f32, status: &str) {
    let mut img = RgbaImage::from_pixel(STRIP_W, STRIP_H, BG);

    let fb = font_bold();
    let f = font();

    // "deckd" title — top left
    let title_scale = PxScale::from(28.0);
    draw_text_mut(&mut img, TEXT_BRIGHT, 20, 8, title_scale, &fb, "deckd");

    // Version — next to title, dimmer
    let ver_scale = PxScale::from(16.0);
    draw_text_mut(&mut img, TEXT_DIM, 110, 16, ver_scale, &f, "v0.1.0");

    // Status text — top right area
    let status_scale = PxScale::from(18.0);
    let status_x = (STRIP_W as i32 - status.len() as i32 * 10 - 20).max(200);
    draw_text_mut(&mut img, TEXT_DIM, status_x, 14, status_scale, &f, status);

    // Progress bar background
    let bar_x = 20_i32;
    let bar_y = 55_i32;
    let bar_w = (STRIP_W - 40) as u32;
    let bar_h = 20_u32;
    draw_filled_rect_mut(&mut img, Rect::at(bar_x, bar_y).of_size(bar_w, bar_h), BAR_BG);

    // Progress bar fill
    let fill_w = ((bar_w as f32 * progress.clamp(0.0, 1.0)) as u32).max(1);
    draw_filled_rect_mut(&mut img, Rect::at(bar_x, bar_y).of_size(fill_w, bar_h), BAR_FG);

    // Glow effect — brighter leading edge
    if fill_w > 2 {
        let glow = Rgba([120, 220, 255, 255]);
        let glow_x = bar_x + fill_w as i32 - 3;
        draw_filled_rect_mut(&mut img, Rect::at(glow_x, bar_y).of_size(3, bar_h), glow);
    }

    // Percentage text under bar
    let pct = format!("{}%", (progress * 100.0) as u32);
    let pct_scale = PxScale::from(14.0);
    draw_text_mut(&mut img, TEXT_DIM, bar_x, bar_y + bar_h as i32 + 4, pct_scale, &f, &pct);

    // Dots animation based on progress (trailing dots)
    let dots = match ((progress * 20.0) as u32) % 4 {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    };
    let dots_x = bar_x + 40;
    draw_text_mut(&mut img, TEXT_DIM, dots_x, bar_y + bar_h as i32 + 4, pct_scale, &f, dots);

    // Write to LCD
    let dyn_img = DynamicImage::ImageRgba8(img);
    match ImageRect::from_image(dyn_img) {
        Ok(rect) => {
            if let Err(e) = deck.write_lcd(0, 0, &rect) {
                error!("Boot LCD write failed: {}", e);
            }
        }
        Err(e) => error!("Boot LCD convert failed: {}", e),
    }
}

/// Render the boot-complete frame
pub fn render_boot_complete(deck: &mut StreamDeck, light_count: usize) {
    let status = format!("{} lights connected", light_count);
    render_boot_frame(deck, 1.0, &status);
    std::thread::sleep(std::time::Duration::from_millis(500));
}
