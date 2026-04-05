//! Minimal UVC (USB Video Class) camera control.
//! Sends USB control transfers directly via rusb — no libuvc, no Node.js.

use std::time::Duration;

use log::{info, warn};
use rusb::{DeviceHandle, GlobalContext};

// ── UVC Constants ───────────────────────────────────────────────

// Request types
const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;

// bmRequestType
const REQ_TYPE_SET: u8 = 0x21; // host→device, class, interface
const REQ_TYPE_GET: u8 = 0xA1; // device→host, class, interface

// Camera Terminal control selectors (UVC 1.1 spec, Table A-12)
const CT_AE_MODE: u8 = 0x02;
const CT_FOCUS_ABSOLUTE: u8 = 0x06;
const CT_FOCUS_AUTO: u8 = 0x08;
const CT_ZOOM_ABSOLUTE: u8 = 0x0B;
const CT_PANTILT_ABSOLUTE: u8 = 0x0D;

// Processing Unit control selectors (UVC 1.1 spec, Table A-14)
const PU_BRIGHTNESS: u8 = 0x02;
const PU_CONTRAST: u8 = 0x03;

const TIMEOUT: Duration = Duration::from_millis(1000);

// ── Camera handle ───────────────────────────────────────────────

pub struct Camera {
    handle: DeviceHandle<GlobalContext>,
    interface: u8,
    camera_terminal_id: u8,
    processing_unit_id: u8,
}

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
        let mut camera_terminal_id = 1u8; // default fallback
        let mut processing_unit_id = 2u8; // default fallback

        for iface in config.interfaces() {
            for desc in iface.descriptors() {
                if desc.class_code() == 14 && desc.sub_class_code() == 1 {
                    interface_num = desc.interface_number();

                    // Parse UVC class-specific descriptors to find unit/terminal IDs
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
                            // CS_INTERFACE
                            match desc_subtype {
                                0x02 => {
                                    // INPUT_TERMINAL
                                    if len >= 8 && extra[pos + 4] == 0x01 && extra[pos + 5] == 0x02 {
                                        // ITT_CAMERA
                                        camera_terminal_id = extra[pos + 3];
                                    }
                                }
                                0x05 => {
                                    // PROCESSING_UNIT
                                    if len >= 8 {
                                        processing_unit_id = extra[pos + 3];
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
            "UVC camera opened: {:04x}:{:04x} interface={} CT={} PU={}",
            dev_desc.vendor_id(), dev_desc.product_id(), interface_num, camera_terminal_id, processing_unit_id
        );

        Ok(Camera {
            handle,
            interface: interface_num,
            camera_terminal_id,
            processing_unit_id,
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

    // ── Processing Unit controls ─────────────────────────────────

    pub fn set_brightness(&self, value: i32) -> Result<(), String> {
        self.set_pu_control(PU_BRIGHTNESS, &(value as i16).to_le_bytes())
    }

    pub fn get_brightness(&self) -> Result<i32, String> {
        let mut buf = [0u8; 2];
        self.get_pu_control(PU_BRIGHTNESS, &mut buf)?;
        Ok(i16::from_le_bytes(buf) as i32)
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
