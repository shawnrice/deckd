//! Minimal UVC (USB Video Class) camera control.
//! Sends USB control transfers directly via rusb — no libuvc, no Node.js.

use std::time::Duration;

use log::info;
use rusb::{DeviceHandle, GlobalContext};

// ── UVC Constants (spec Table A-12, A-14) ───────────────────────
#[allow(dead_code)]
mod consts {
    // Request types
    pub const SET_CUR: u8 = 0x01;
    pub const GET_CUR: u8 = 0x81;
    pub const GET_MIN: u8 = 0x82;
    pub const GET_MAX: u8 = 0x83;
    pub const REQ_TYPE_SET: u8 = 0x21;
    pub const REQ_TYPE_GET: u8 = 0xA1;

    // Camera Terminal selectors (CT)
    pub const CT_AE_MODE: u8 = 0x02;
    pub const CT_EXPOSURE_TIME_ABS: u8 = 0x04;
    pub const CT_FOCUS_ABSOLUTE: u8 = 0x06;
    pub const CT_FOCUS_AUTO: u8 = 0x08;
    pub const CT_ZOOM_ABSOLUTE: u8 = 0x0B;
    pub const CT_PANTILT_ABSOLUTE: u8 = 0x0D;

    // Processing Unit selectors (PU)
    pub const PU_BACKLIGHT_COMP: u8 = 0x01;
    pub const PU_BRIGHTNESS: u8 = 0x02;
    pub const PU_CONTRAST: u8 = 0x03;
    pub const PU_GAIN: u8 = 0x04;
    pub const PU_POWER_LINE_FREQ: u8 = 0x05;
    pub const PU_SATURATION: u8 = 0x07;
    pub const PU_SHARPNESS: u8 = 0x08;
    pub const PU_WHITE_BALANCE_TEMP: u8 = 0x0A;
    pub const PU_WHITE_BALANCE_AUTO: u8 = 0x0B;

    // Logitech Extension Unit: Video Pipe V3 (BRIO / MX Brio)
    // GUID: 49E40215-F434-47fe-B158-0E885023E51B
    pub const LOGI_XU_COLOR_BOOST: u8 = 0x01;
    pub const LOGI_XU_RIGHTLIGHT: u8 = 0x04;  // HDR / low-light adaptation
    pub const LOGI_XU_FOV: u8 = 0x05;         // 0x00=90°, 0x01=78°, 0x02=65°
    pub const LOGI_XU_FW_ZOOM: u8 = 0x06;
    pub const LOGI_XU_DUAL_ISO: u8 = 0x07;
}
use consts::*;

const TIMEOUT: Duration = Duration::from_millis(1000);

pub struct CameraInfo {
    pub vid: u16,
    pub pid: u16,
    pub name: String,
}

impl CameraInfo {
    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.vid, self.pid)
    }
}

/// Find all UVC cameras connected to the system
pub fn find_cameras() -> Vec<CameraInfo> {
    let devices = match rusb::devices() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut cameras = Vec::new();
    for device in devices.iter() {
        let config = match device.active_config_descriptor() {
            Ok(c) => c,
            Err(_) => continue,
        };

        let is_uvc = config.interfaces().any(|iface| {
            iface
                .descriptors()
                .any(|desc| desc.class_code() == 14 && desc.sub_class_code() == 1)
        });

        if !is_uvc {
            continue;
        }

        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        let handle = device.open().ok();
        let product = handle
            .as_ref()
            .and_then(|h| h.read_product_string_ascii(&desc).ok())
            .unwrap_or_else(|| "(unknown)".into());
        let manufacturer = handle
            .as_ref()
            .and_then(|h| h.read_manufacturer_string_ascii(&desc).ok())
            .unwrap_or_default();

        let name = if manufacturer.is_empty() {
            product
        } else {
            format!("{} {}", manufacturer, product)
        };

        cameras.push(CameraInfo {
            vid: desc.vendor_id(),
            pid: desc.product_id(),
            name,
        });
    }

    cameras
}

// ── Camera handle ───────────────────────────────────────────────

// Logitech Video Pipe V3 GUID (little-endian mixed format as it appears in USB descriptors)
const LOGI_VIDEO_PIPE_V3_GUID: [u8; 16] = [
    0x15, 0x02, 0xE4, 0x49, 0x34, 0xF4, 0xFE, 0x47,
    0xB1, 0x58, 0x0E, 0x88, 0x50, 0x23, 0xE5, 0x1B,
];

pub struct Camera {
    handle: DeviceHandle<GlobalContext>,
    interface: u8,
    camera_terminal_id: u8,
    processing_unit_id: u8,
    logi_xu_id: Option<u8>, // Logitech Extension Unit ID (if present)
}

#[allow(dead_code)]
impl Camera {
    /// Open a UVC camera by vendor/product ID.
    /// Parses the UVC descriptor to find terminal/unit IDs.
    pub fn open(vendor_id: u16, product_id: u16) -> Result<Self, String> {
        let device = rusb::devices()
            .map_err(|e| format!("USB enumeration: {}", e))?
            .iter()
            .find(|d| {
                d.device_descriptor().ok().map_or(false, |desc| {
                    desc.vendor_id() == vendor_id && desc.product_id() == product_id
                })
            })
            .ok_or_else(|| format!("Camera {:04x}:{:04x} not found", vendor_id, product_id))?;

        Self::open_device(device)
    }

    /// Open the first UVC camera found on the system.
    pub fn open_any() -> Result<Self, String> {
        let device = rusb::devices()
            .map_err(|e| format!("USB enumeration: {}", e))?
            .iter()
            .find(|d| {
                let config = match d.active_config_descriptor() {
                    Ok(c) => c,
                    Err(_) => return false,
                };
                // Look for UVC Video Control interface (class 14, subclass 1)
                config.interfaces().any(|iface| {
                    iface.descriptors().any(|desc| {
                        desc.class_code() == 14 && desc.sub_class_code() == 1
                    })
                })
            })
            .ok_or("No UVC camera found")?;

        let desc = device.device_descriptor().map_err(|e| format!("USB descriptor: {}", e))?;
        info!("Auto-detected UVC camera: {:04x}:{:04x}", desc.vendor_id(), desc.product_id());

        Self::open_device(device)
    }

    fn open_device(device: rusb::Device<GlobalContext>) -> Result<Self, String> {
        let handle = device.open().map_err(|e| format!("USB open: {}", e))?;

        // Find the UVC Video Control interface (class 14, subclass 1)
        let config = device
            .active_config_descriptor()
            .map_err(|e| format!("USB config: {}", e))?;

        let mut interface_num = 0u8;
        let mut camera_terminal_id = 1u8;
        let mut processing_unit_id = 2u8;
        let mut logi_xu_id: Option<u8> = None;

        for iface in config.interfaces() {
            for desc in iface.descriptors() {
                if desc.class_code() == 14 && desc.sub_class_code() == 1 {
                    interface_num = desc.interface_number();

                    let extra = desc.extra();
                    let mut pos = 0;
                    while pos + 3 < extra.len() {
                        let len = extra[pos] as usize;
                        if len < 3 || pos + len > extra.len() {
                            break;
                        }
                        let desc_type = extra[pos + 1];
                        let desc_subtype = extra[pos + 2];

                        if desc_type == 0x24 {
                            match desc_subtype {
                                0x02 => {
                                    // INPUT_TERMINAL (ITT_CAMERA)
                                    if len >= 8 && extra[pos + 4] == 0x01 && extra[pos + 5] == 0x02 {
                                        camera_terminal_id = extra[pos + 3];
                                    }
                                }
                                0x05 => {
                                    // PROCESSING_UNIT
                                    if len >= 8 {
                                        processing_unit_id = extra[pos + 3];
                                    }
                                }
                                0x06 => {
                                    // EXTENSION_UNIT — check GUID
                                    if len >= 24 {
                                        let unit_id = extra[pos + 3];
                                        let guid = &extra[pos + 4..pos + 20];
                                        if guid == LOGI_VIDEO_PIPE_V3_GUID {
                                            logi_xu_id = Some(unit_id);
                                            info!("Found Logitech Video Pipe V3 XU: unit={}", unit_id);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        pos += len;
                    }
                    break;
                }
            }
        }

        let dev_desc = device.device_descriptor().map_err(|e| format!("USB descriptor: {}", e))?;
        info!(
            "UVC camera opened: {:04x}:{:04x} interface={} CT={} PU={} XU={:?}",
            dev_desc.vendor_id(), dev_desc.product_id(), interface_num,
            camera_terminal_id, processing_unit_id, logi_xu_id
        );

        Ok(Camera {
            handle,
            interface: interface_num,
            camera_terminal_id,
            processing_unit_id,
            logi_xu_id,
        })
    }

    // ── Camera Terminal controls ─────────────────────────────────

    pub fn set_zoom(&self, value: i32) -> Result<(), String> {
        self.set_ct_control(CT_ZOOM_ABSOLUTE, &(value as u16).to_le_bytes())
    }

    pub fn get_zoom(&self) -> Result<i32, String> {
        let mut buf = [0u8; 2];
        self.get_ct_control(CT_ZOOM_ABSOLUTE, &mut buf)?;
        Ok(u16::from_le_bytes(buf) as i32)
    }

    pub fn set_pantilt(&self, pan: i32, tilt: i32) -> Result<(), String> {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&pan.to_le_bytes());
        buf[4..8].copy_from_slice(&tilt.to_le_bytes());
        self.set_ct_control(CT_PANTILT_ABSOLUTE, &buf)
    }

    pub fn set_focus_auto(&self, on: bool) -> Result<(), String> {
        self.set_ct_control(CT_FOCUS_AUTO, &[if on { 1 } else { 0 }])
    }

    pub fn set_exposure_auto(&self, on: bool) -> Result<(), String> {
        // AE mode: 1=manual, 2=auto, 4=shutter priority, 8=aperture priority
        self.set_ct_control(CT_AE_MODE, &[if on { 2 } else { 1 }])
    }

    pub fn set_exposure_time(&self, value: i32) -> Result<(), String> {
        self.set_ct_control(CT_EXPOSURE_TIME_ABS, &(value as u32).to_le_bytes())
    }

    // ── Processing Unit controls ─────────────────────────────────

    pub fn set_brightness(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_BRIGHTNESS, &(value as i16).to_le_bytes())
    }

    pub fn get_brightness(&self) -> Result<i32, String> {
        let mut buf = [0u8; 2];
        self.get_pu_control(PU_BRIGHTNESS, &mut buf)?;
        Ok(i16::from_le_bytes(buf) as i32)
    }

    pub fn set_contrast(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_CONTRAST, &(value as u16).to_le_bytes())
    }

    pub fn set_saturation(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_SATURATION, &(value as u16).to_le_bytes())
    }

    pub fn set_sharpness(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_SHARPNESS, &(value as u16).to_le_bytes())
    }

    pub fn set_gain(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_GAIN, &(value as u16).to_le_bytes())
    }

    pub fn set_white_balance_auto(&self, on: bool) -> Result<(), String> {
        self.set_pu_control(PU_WHITE_BALANCE_AUTO, &[if on { 1 } else { 0 }])
    }

    pub fn set_white_balance_temp(&self, kelvin: i32) -> Result<(), String> {
        self.set_pu_control(PU_WHITE_BALANCE_TEMP, &(kelvin as u16).to_le_bytes())
    }

    pub fn set_backlight_compensation(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_BACKLIGHT_COMP, &(value as u16).to_le_bytes())
    }

    /// Get a control's current, min, and max values (for discovering ranges)
    pub fn get_control_range(&self, is_ct: bool, selector: u8) -> Result<(i32, i32, i32), String> {
        let unit = if is_ct { self.camera_terminal_id } else { self.processing_unit_id };
        let mut cur_buf = [0u8; 2];
        let mut min_buf = [0u8; 2];
        let mut max_buf = [0u8; 2];

        self.control_get(selector, unit, &mut cur_buf)?;
        let w_value = (selector as u16) << 8;
        let w_index = (unit as u16) << 8 | self.interface as u16;

        self.handle
            .read_control(REQ_TYPE_GET, GET_MIN, w_value, w_index, &mut min_buf, TIMEOUT)
            .map_err(|e| format!("GET_MIN: {}", e))?;
        self.handle
            .read_control(REQ_TYPE_GET, GET_MAX, w_value, w_index, &mut max_buf, TIMEOUT)
            .map_err(|e| format!("GET_MAX: {}", e))?;

        Ok((
            i16::from_le_bytes(cur_buf) as i32,
            i16::from_le_bytes(min_buf) as i32,
            i16::from_le_bytes(max_buf) as i32,
        ))
    }

    // ── Logitech Extension Unit controls ────────────────────────

    /// Check if this camera has Logitech extension controls
    pub fn has_logitech_xu(&self) -> bool {
        self.logi_xu_id.is_some()
    }

    /// Set Field of View: 0=90° wide, 1=78° medium, 2=65° narrow
    pub fn set_fov(&self, fov: u8) -> Result<(), String> {
        let xu = self.logi_xu_id.ok_or("No Logitech XU on this camera")?;
        self.control_set(LOGI_XU_FOV, xu, &[fov.clamp(0, 2)])
    }

    pub fn get_fov(&self) -> Result<u8, String> {
        let xu = self.logi_xu_id.ok_or("No Logitech XU on this camera")?;
        let mut buf = [0u8; 1];
        self.control_get(LOGI_XU_FOV, xu, &mut buf)?;
        Ok(buf[0])
    }

    /// Set RightLight mode (HDR / adaptive exposure)
    pub fn set_rightlight(&self, mode: u8) -> Result<(), String> {
        let xu = self.logi_xu_id.ok_or("No Logitech XU on this camera")?;
        self.control_set(LOGI_XU_RIGHTLIGHT, xu, &[mode])
    }

    pub fn get_rightlight(&self) -> Result<u8, String> {
        let xu = self.logi_xu_id.ok_or("No Logitech XU on this camera")?;
        let mut buf = [0u8; 1];
        self.control_get(LOGI_XU_RIGHTLIGHT, xu, &mut buf)?;
        Ok(buf[0])
    }

    // ── Low-level control transfers ──────────────────────────────

    fn set_ct_control(&self, selector: u8, data: &[u8]) -> Result<(), String> {
        self.control_set(selector, self.camera_terminal_id, data)
    }

    fn get_ct_control(&self, selector: u8, buf: &mut [u8]) -> Result<usize, String> {
        self.control_get(selector, self.camera_terminal_id, buf)
    }

    fn set_pu_control(&self, selector: u8, data: &[u8]) -> Result<(), String> {
        self.control_set(selector, self.processing_unit_id, data)
    }

    fn get_pu_control(&self, selector: u8, buf: &mut [u8]) -> Result<usize, String> {
        self.control_get(selector, self.processing_unit_id, buf)
    }

    fn control_set(&self, selector: u8, unit_id: u8, data: &[u8]) -> Result<(), String> {
        let w_value = (selector as u16) << 8;
        let w_index = (unit_id as u16) << 8 | self.interface as u16;

        self.handle
            .write_control(REQ_TYPE_SET, SET_CUR, w_value, w_index, data, TIMEOUT)
            .map_err(|e| format!("UVC SET_CUR (selector={:#04x}): {}", selector, e))?;
        Ok(())
    }

    fn control_get(&self, selector: u8, unit_id: u8, buf: &mut [u8]) -> Result<usize, String> {
        let w_value = (selector as u16) << 8;
        let w_index = (unit_id as u16) << 8 | self.interface as u16;

        self.handle
            .read_control(REQ_TYPE_GET, GET_CUR, w_value, w_index, buf, TIMEOUT)
            .map_err(|e| format!("UVC GET_CUR (selector={:#04x}): {}", selector, e))
    }
}
