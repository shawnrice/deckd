use std::mem;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use coreaudio_sys::*;
use log::info;

// ── Device queries ──────────────────────────────────────────────

fn default_device(input: bool) -> Option<AudioDeviceID> {
    let selector = if input {
        kAudioHardwarePropertyDefaultInputDevice
    } else {
        kAudioHardwarePropertyDefaultOutputDevice
    };

    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut device_id: AudioDeviceID = 0;
    let mut size = mem::size_of::<AudioDeviceID>() as u32;

    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut device_id as *mut _ as *mut _,
        )
    };

    if status != 0 || device_id == 0 {
        None
    } else {
        Some(device_id)
    }
}

// ── Volume ──────────────────────────────────────────────────────

/// Get output volume as 0-100 integer
pub fn get_output_volume() -> Option<u32> {
    let device = default_device(false)?;
    let vol = get_volume_scalar(device, kAudioDevicePropertyScopeOutput)?;
    Some((vol * 100.0).round() as u32)
}

/// Set output volume (0-100)
pub fn set_output_volume(percent: u32) {
    if let Some(device) = default_device(false) {
        let scalar = (percent as f32 / 100.0).clamp(0.0, 1.0);
        set_volume_scalar(device, kAudioDevicePropertyScopeOutput, scalar);
    }
}

/// Adjust output volume by delta (e.g. +5, -5)
pub fn adjust_output_volume(delta: i32) -> u32 {
    let current = get_output_volume().unwrap_or(50) as i32;
    let new_vol = (current + delta).clamp(0, 100) as u32;
    set_output_volume(new_vol);
    new_vol
}

fn get_volume_scalar(device: AudioDeviceID, scope: AudioObjectPropertyScope) -> Option<f32> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyVolumeScalar,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };

    // Check if main element has volume (some devices use per-channel)
    let has_main = unsafe {
        AudioObjectHasProperty(device, &address)
    };

    if has_main != 0 {
        let mut volume: f32 = 0.0;
        let mut size = mem::size_of::<f32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut volume as *mut _ as *mut _,
            )
        };
        if status == 0 {
            return Some(volume);
        }
    }

    // Try channel 1 as fallback
    let ch_address = AudioObjectPropertyAddress {
        mElement: 1,
        ..address
    };
    let mut volume: f32 = 0.0;
    let mut size = mem::size_of::<f32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &ch_address,
            0,
            std::ptr::null(),
            &mut size,
            &mut volume as *mut _ as *mut _,
        )
    };
    if status == 0 {
        Some(volume)
    } else {
        None
    }
}

fn set_volume_scalar(device: AudioDeviceID, scope: AudioObjectPropertyScope, volume: f32) {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyVolumeScalar,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };

    let has_main = unsafe { AudioObjectHasProperty(device, &address) };

    if has_main != 0 {
        unsafe {
            AudioObjectSetPropertyData(
                device,
                &address,
                0,
                std::ptr::null(),
                mem::size_of::<f32>() as u32,
                &volume as *const _ as *const _,
            );
        }
    } else {
        // Set per-channel
        for ch in 1..=2u32 {
            let ch_address = AudioObjectPropertyAddress {
                mElement: ch,
                ..address
            };
            unsafe {
                AudioObjectSetPropertyData(
                    device,
                    &ch_address,
                    0,
                    std::ptr::null(),
                    mem::size_of::<f32>() as u32,
                    &volume as *const _ as *const _,
                );
            }
        }
    }
}

// ── Mute ────────────────────────────────────────────────────────

/// Get input mute state
#[allow(dead_code)]
pub fn is_input_muted() -> bool {
    let device = match default_device(true) {
        Some(d) => d,
        None => return false,
    };
    get_mute(device, kAudioDevicePropertyScopeInput).unwrap_or(false)
}

/// Toggle input mute, returns new mute state
pub fn toggle_input_mute() -> bool {
    let device = match default_device(true) {
        Some(d) => d,
        None => return false,
    };
    let currently_muted = get_mute(device, kAudioDevicePropertyScopeInput).unwrap_or(false);
    set_mute(device, kAudioDevicePropertyScopeInput, !currently_muted);
    !currently_muted
}

fn get_mute(device: AudioDeviceID, scope: AudioObjectPropertyScope) -> Option<bool> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyMute,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };

    let has = unsafe { AudioObjectHasProperty(device, &address) };
    if has == 0 {
        return None;
    }

    let mut muted: u32 = 0;
    let mut size = mem::size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut muted as *mut _ as *mut _,
        )
    };
    if status == 0 {
        Some(muted != 0)
    } else {
        None
    }
}

fn set_mute(device: AudioDeviceID, scope: AudioObjectPropertyScope, mute: bool) {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyMute,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    };

    let value: u32 = if mute { 1 } else { 0 };
    unsafe {
        AudioObjectSetPropertyData(
            device,
            &address,
            0,
            std::ptr::null(),
            mem::size_of::<u32>() as u32,
            &value as *const _ as *const _,
        );
    }
}

// ── Device switching (via SwitchAudioSource — fast enough) ───────

use crate::config::AudioDevice;

const EXCLUDED_DEVICES: &[&str] = &["ZoomAudioDevice"];

/// A device in the cycle list: uid for switching, display name for LCD
#[derive(Debug, Clone)]
struct CycleEntry {
    uid: String,
    display: String,
}

/// Build cycle list from configured devices + any dynamically-connected ones
fn build_cycle_list(preferred: &[AudioDevice], device_type: &str) -> Vec<CycleEntry> {
    let connected = list_connected_devices(device_type);

    // Start with preferred devices that are currently connected
    let mut cycle: Vec<CycleEntry> = preferred
        .iter()
        .filter(|p| connected.iter().any(|(uid, _)| *uid == p.uid))
        .map(|p| CycleEntry { uid: p.uid.clone(), display: p.name.clone() })
        .collect();

    // Add any connected device not already in the list and not excluded
    for (uid, name) in &connected {
        let excluded = EXCLUDED_DEVICES.iter().any(|e| name.contains(e));
        let already = cycle.iter().any(|c| c.uid == *uid);
        if !excluded && !already {
            cycle.push(CycleEntry { uid: uid.clone(), display: name.clone() });
        }
    }

    cycle
}

/// Stateful device cycler — caches the device list and tracks current index
pub struct DeviceCycler {
    output_devices: Vec<CycleEntry>,
    input_devices: Vec<CycleEntry>,
    output_idx: usize,
    input_idx: usize,
}

impl DeviceCycler {
    pub fn new(output_preferred: &[AudioDevice], input_preferred: &[AudioDevice]) -> Self {
        let output_devices = build_cycle_list(output_preferred, "output");
        let input_devices = build_cycle_list(input_preferred, "input");

        // Find current position
        let current_out = current_device_uid("output");
        let output_idx = output_devices.iter().position(|d| d.uid == current_out).unwrap_or(0);
        let current_in = current_device_uid("input");
        let input_idx = input_devices.iter().position(|d| d.uid == current_in).unwrap_or(0);

        Self { output_devices, input_devices, output_idx, input_idx }
    }

    /// Refresh the device lists (call periodically to pick up new devices)
    pub fn refresh(&mut self, output_preferred: &[AudioDevice], input_preferred: &[AudioDevice]) {
        self.output_devices = build_cycle_list(output_preferred, "output");
        self.input_devices = build_cycle_list(input_preferred, "input");
        // Clamp indices
        if !self.output_devices.is_empty() {
            self.output_idx = self.output_idx.min(self.output_devices.len() - 1);
        }
        if !self.input_devices.is_empty() {
            self.input_idx = self.input_idx.min(self.input_devices.len() - 1);
        }
    }

    /// Cycle output device. Single subprocess call (just the switch).
    pub fn cycle_output(&mut self, direction: i8) -> String {
        if self.output_devices.is_empty() {
            return "?".into();
        }
        let len = self.output_devices.len() as i32;
        let next_i = ((self.output_idx as i32 + direction as i32) % len + len) % len;
        self.output_idx = next_i as usize;
        let entry = &self.output_devices[self.output_idx];
        switch_device_uid(&entry.uid);
        entry.display.clone()
    }

    /// Cycle input device. Single subprocess call.
    pub fn cycle_input(&mut self, direction: i8) -> String {
        if self.input_devices.is_empty() {
            return "?".into();
        }
        let len = self.input_devices.len() as i32;
        let next_i = ((self.input_idx as i32 + direction as i32) % len + len) % len;
        self.input_idx = next_i as usize;
        let entry = &self.input_devices[self.input_idx];
        switch_device_uid(&entry.uid);
        entry.display.clone()
    }
}

/// List connected devices as (uid, name) pairs
fn list_connected_devices(device_type: &str) -> Vec<(String, String)> {
    Command::new("SwitchAudioSource")
        .args(["-a", "-t", device_type, "-f", "json"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter_map(|line| {
                    let uid = extract_json_field(line, "uid")?;
                    let name = extract_json_field(line, "name")?;
                    Some((uid, name))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn current_device_uid(device_type: &str) -> String {
    Command::new("SwitchAudioSource")
        .args(["-c", "-t", device_type, "-f", "json"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| extract_json_field(&s, "uid"))
        .unwrap_or_default()
}

fn switch_device_uid(uid: &str) {
    Command::new("SwitchAudioSource")
        .args(["-u", uid])
        .output()
        .ok();
}

fn extract_json_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\": \"", key);
    let start = json.find(&pattern)? + pattern.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

// ── CoreAudio device change listener ────────────────────────────

/// Register CoreAudio listeners that set `flag` to true when audio devices
/// change (connect/disconnect, default device switch). The main loop polls
/// this flag and refreshes the cycler + dashboard.
pub fn start_device_change_listener(flag: Arc<AtomicBool>) {
    let selectors = [
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioHardwarePropertyDevices,
    ];

    for selector in selectors {
        let address = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let flag_ptr = Arc::into_raw(Arc::clone(&flag)) as *mut std::ffi::c_void;

        unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject,
                &address,
                Some(audio_property_changed),
                flag_ptr,
            );
        }
    }

    info!("CoreAudio device change listener started");
}

unsafe extern "C" fn audio_property_changed(
    _id: AudioObjectID,
    _num_addresses: u32,
    _addresses: *const AudioObjectPropertyAddress,
    client_data: *mut std::ffi::c_void,
) -> OSStatus {
    // Set the flag — main loop will notice and refresh
    // SAFETY: client_data is an Arc<AtomicBool> pointer created via Arc::into_raw
    let flag = unsafe { Arc::from_raw(client_data as *const AtomicBool) };
    flag.store(true, Ordering::Relaxed);
    // Don't drop — we need to keep the Arc alive
    let _ = Arc::into_raw(flag);
    0
}
