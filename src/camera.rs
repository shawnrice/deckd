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

pub fn toggle_autofocus(state: &mut CameraState) -> Result<(), String> {
    state.auto_focus = !state.auto_focus;
    info!("Camera autofocus: {}", if state.auto_focus { "on" } else { "off" });
    with_camera(state.camera_id(), |c| c.set_focus_auto(state.auto_focus))
}
