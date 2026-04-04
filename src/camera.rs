use std::process::Command;

use log::info;

/// Camera state tracking
#[derive(Debug, Clone)]
pub struct CameraState {
    pub zoom: i32,      // 100-400 (100 = 1x, 400 = 4x)
    pub pan: i32,       // -72000 to 72000
    pub tilt: i32,      // -72000 to 72000
    #[allow(dead_code)]
    pub brightness: i32,
    #[allow(dead_code)]
    pub contrast: i32,
    pub auto_focus: bool,
}

impl CameraState {
    pub fn new() -> Self {
        Self {
            zoom: 100,
            pan: 0,
            tilt: 0,
            brightness: 128,
            contrast: 128,
            auto_focus: true,
        }
    }

    /// Read current state from camera
    #[allow(dead_code)]
    pub fn read_from_device() -> Self {
        let zoom = uvcc_get("absolute_zoom").unwrap_or(100);
        let pan_tilt = uvcc_get_pair("absolute_pan_tilt").unwrap_or((0, 0));
        let brightness = uvcc_get("brightness").unwrap_or(128);
        let contrast = uvcc_get("contrast").unwrap_or(128);
        let auto_focus = uvcc_get("auto_focus").unwrap_or(1) == 1;

        Self {
            zoom,
            pan: pan_tilt.0,
            tilt: pan_tilt.1,
            brightness,
            contrast,
            auto_focus,
        }
    }
}

// ── uvcc CLI wrapper ──────────────────────────────────────────────

fn uvcc_set(control: &str, value: i32) -> Result<(), String> {
    let output = Command::new("uvcc")
        .args(["set", control, &value.to_string()])
        .output()
        .map_err(|e| format!("uvcc set {}: {}", control, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("uvcc set {} failed: {}", control, stderr));
    }
    Ok(())
}

fn uvcc_set_pair(control: &str, a: i32, b: i32) -> Result<(), String> {
    let output = Command::new("uvcc")
        .args(["set", control, &a.to_string(), &b.to_string()])
        .output()
        .map_err(|e| format!("uvcc set {}: {}", control, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("uvcc set {} failed: {}", control, stderr));
    }
    Ok(())
}

#[allow(dead_code)]
fn uvcc_get(control: &str) -> Option<i32> {
    Command::new("uvcc")
        .args(["get", control])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
}

#[allow(dead_code)]
fn uvcc_get_pair(control: &str) -> Option<(i32, i32)> {
    let output = Command::new("uvcc")
        .args(["get", control])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())?;

    // Parse JSON array like [0, -57600]
    let trimmed = output.trim();
    let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() == 2 {
        let a = parts[0].trim().parse().ok()?;
        let b = parts[1].trim().parse().ok()?;
        Some((a, b))
    } else {
        None
    }
}

// ── Public camera commands ────────────────────────────────────────

pub fn zoom_in(state: &mut CameraState) -> Result<(), String> {
    state.zoom = (state.zoom + 20).min(400);
    info!("Camera zoom: {}x", state.zoom as f32 / 100.0);
    uvcc_set("absolute_zoom", state.zoom)
}

pub fn zoom_out(state: &mut CameraState) -> Result<(), String> {
    state.zoom = (state.zoom - 20).max(100);
    info!("Camera zoom: {}x", state.zoom as f32 / 100.0);
    uvcc_set("absolute_zoom", state.zoom)
}

pub fn zoom_reset(state: &mut CameraState) -> Result<(), String> {
    state.zoom = 100;
    state.pan = 0;
    state.tilt = 0;
    info!("Camera reset to 1x, centered");
    uvcc_set("absolute_zoom", 100)?;
    uvcc_set_pair("absolute_pan_tilt", 0, 0)
}

pub fn pan_left(state: &mut CameraState) -> Result<(), String> {
    state.pan = (state.pan - 3600).max(-72000);
    uvcc_set_pair("absolute_pan_tilt", state.pan, state.tilt)
}

pub fn pan_right(state: &mut CameraState) -> Result<(), String> {
    state.pan = (state.pan + 3600).min(72000);
    uvcc_set_pair("absolute_pan_tilt", state.pan, state.tilt)
}

pub fn tilt_up(state: &mut CameraState) -> Result<(), String> {
    state.tilt = (state.tilt + 3600).min(72000);
    uvcc_set_pair("absolute_pan_tilt", state.pan, state.tilt)
}

pub fn tilt_down(state: &mut CameraState) -> Result<(), String> {
    state.tilt = (state.tilt - 3600).max(-72000);
    uvcc_set_pair("absolute_pan_tilt", state.pan, state.tilt)
}

pub fn toggle_autofocus(state: &mut CameraState) -> Result<(), String> {
    state.auto_focus = !state.auto_focus;
    let val = if state.auto_focus { 1 } else { 0 };
    info!("Camera autofocus: {}", if state.auto_focus { "on" } else { "off" });
    uvcc_set("auto_focus", val)
}
