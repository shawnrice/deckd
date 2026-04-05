use log::info;

use crate::uvc::Camera;

/// Camera state tracking
#[derive(Debug, Clone)]
pub struct CameraState {
    pub zoom: i32,       // 100-400 (100 = 1x, 400 = 4x)
    pub pan: i32,        // -72000 to 72000
    pub tilt: i32,       // -72000 to 72000
    pub auto_focus: bool,
    device: Option<String>, // "vid:pid" or None for auto-detect
}

impl CameraState {
    pub fn new(camera_config: Option<String>) -> Self {
        Self {
            zoom: 100,
            pan: 0,
            tilt: 0,
            auto_focus: true,
            device: camera_config,
        }
    }

    fn camera_id(&self) -> Option<&str> {
        self.device.as_deref()
    }
}

/// Parse "046d:0944" into (vendor_id, product_id)
fn parse_vid_pid(s: &str) -> Option<(u16, u16)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let vid = u16::from_str_radix(parts[0], 16).ok()?;
    let pid = u16::from_str_radix(parts[1], 16).ok()?;
    Some((vid, pid))
}

fn with_camera<F>(camera_config: Option<&str>, f: F) -> Result<(), String>
where
    F: FnOnce(&Camera) -> Result<(), String>,
{
    let cam = match camera_config.and_then(parse_vid_pid) {
        Some((vid, pid)) => Camera::open(vid, pid)?,
        None => Camera::open_any()?,
    };
    f(&cam)
}

pub fn zoom_in(state: &mut CameraState) -> Result<(), String> {
    state.zoom = (state.zoom + 20).min(400);
    info!("Camera zoom: {}x", state.zoom as f32 / 100.0);
    with_camera(state.camera_id(), |c| c.set_zoom(state.zoom))
}

pub fn zoom_out(state: &mut CameraState) -> Result<(), String> {
    state.zoom = (state.zoom - 20).max(100);
    info!("Camera zoom: {}x", state.zoom as f32 / 100.0);
    with_camera(state.camera_id(), |c| c.set_zoom(state.zoom))
}

pub fn zoom_reset(state: &mut CameraState) -> Result<(), String> {
    state.zoom = 100;
    state.pan = 0;
    state.tilt = 0;
    info!("Camera reset to 1x, centered");
    with_camera(state.camera_id(), |c| {
        c.set_zoom(100)?;
        c.set_pantilt(0, 0)
    })
}

pub fn pan_left(state: &mut CameraState) -> Result<(), String> {
    state.pan = (state.pan - 3600).max(-72000);
    with_camera(state.camera_id(), |c| c.set_pantilt(state.pan, state.tilt))
}

pub fn pan_right(state: &mut CameraState) -> Result<(), String> {
    state.pan = (state.pan + 3600).min(72000);
    with_camera(state.camera_id(), |c| c.set_pantilt(state.pan, state.tilt))
}

pub fn tilt_up(state: &mut CameraState) -> Result<(), String> {
    state.tilt = (state.tilt + 3600).min(72000);
    with_camera(state.camera_id(), |c| c.set_pantilt(state.pan, state.tilt))
}

pub fn tilt_down(state: &mut CameraState) -> Result<(), String> {
    state.tilt = (state.tilt - 3600).max(-72000);
    with_camera(state.camera_id(), |c| c.set_pantilt(state.pan, state.tilt))
}

pub fn adjust_brightness(state: &mut CameraState, delta: i32) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let (cur, min, max) = c.get_control_range(false, 0x02)?;
        let new_val = (cur + delta).clamp(min, max);
        info!("Camera brightness: {} ({}..{})", new_val, min, max);
        c.set_brightness(new_val)
    })
}

pub fn adjust_contrast(state: &mut CameraState, delta: i32) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let (cur, min, max) = c.get_control_range(false, 0x03)?;
        let new_val = (cur + delta).clamp(min, max);
        info!("Camera contrast: {} ({}..{})", new_val, min, max);
        c.set_contrast(new_val)
    })
}

pub fn adjust_saturation(state: &mut CameraState, delta: i32) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let (cur, min, max) = c.get_control_range(false, 0x07)?;
        let new_val = (cur + delta).clamp(min, max);
        info!("Camera saturation: {} ({}..{})", new_val, min, max);
        c.set_saturation(new_val)
    })
}

pub fn adjust_sharpness(state: &mut CameraState, delta: i32) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let (cur, min, max) = c.get_control_range(false, 0x08)?;
        let new_val = (cur + delta).clamp(min, max);
        info!("Camera sharpness: {} ({}..{})", new_val, min, max);
        c.set_sharpness(new_val)
    })
}

pub fn adjust_white_balance(state: &mut CameraState, delta: i32) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        // Disable auto WB first
        c.set_white_balance_auto(false).ok();
        let (cur, min, max) = c.get_control_range(false, 0x0A)?;
        let new_val = (cur + delta).clamp(min, max);
        info!("Camera white balance: {}K ({}..{})", new_val, min, max);
        c.set_white_balance_temp(new_val)
    })
}

pub fn toggle_auto_white_balance(state: &mut CameraState) -> Result<(), String> {
    // Toggle — read current, flip
    with_camera(state.camera_id(), |c| {
        let (cur, _, _) = c.get_control_range(false, 0x0B)?;
        let new_val = if cur != 0 { false } else { true };
        info!("Camera auto WB: {}", if new_val { "on" } else { "off" });
        c.set_white_balance_auto(new_val)
    })
}

pub fn toggle_auto_exposure(state: &mut CameraState) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let (cur, _, _) = c.get_control_range(true, 0x02)?;
        let auto = cur == 2;
        info!("Camera auto exposure: {}", if !auto { "on" } else { "off" });
        c.set_exposure_auto(!auto)
    })
}

pub fn set_fov_wide(state: &mut CameraState) -> Result<(), String> {
    info!("Camera FOV: 90° wide");
    with_camera(state.camera_id(), |c| c.set_fov(0))
}

pub fn set_fov_medium(state: &mut CameraState) -> Result<(), String> {
    info!("Camera FOV: 78° medium");
    with_camera(state.camera_id(), |c| c.set_fov(1))
}

pub fn set_fov_narrow(state: &mut CameraState) -> Result<(), String> {
    info!("Camera FOV: 65° narrow");
    with_camera(state.camera_id(), |c| c.set_fov(2))
}

pub fn cycle_fov(state: &mut CameraState) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let current = c.get_fov().unwrap_or(0);
        let next = (current + 1) % 3;
        let label = match next {
            0 => "90° wide",
            1 => "78° medium",
            _ => "65° narrow",
        };
        info!("Camera FOV: {}", label);
        c.set_fov(next)
    })
}

pub fn toggle_rightlight(state: &mut CameraState) -> Result<(), String> {
    with_camera(state.camera_id(), |c| {
        let current = c.get_rightlight().unwrap_or(0);
        let next = if current != 0 { 0 } else { 1 };
        info!("Camera RightLight: {}", if next != 0 { "on" } else { "off" });
        c.set_rightlight(next)
    })
}

pub fn toggle_autofocus(state: &mut CameraState) -> Result<(), String> {
    state.auto_focus = !state.auto_focus;
    info!("Camera autofocus: {}", if state.auto_focus { "on" } else { "off" });
    with_camera(state.camera_id(), |c| c.set_focus_auto(state.auto_focus))
}
