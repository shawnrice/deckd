use std::io::Write as IoWrite;
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{info, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ble_native::{BleManager, BlePeripheral};

/// CoreBluetooth characteristic UUID Neewer lights accept GATT writes on.
/// Same value the original btleplug-based actor used.
const NEEWER_WRITE_CHAR: Uuid = Uuid::from_u128(0x69400002_b5a3_f393_e0a9_e50e24dcca99);

/// Notification characteristic. The light sends back state updates here
/// after writes (e.g., power on/off transitions, current channel).
/// Format per NeewerLite-Python and the Home Assistant integration:
///   data[0] == 0x01 → channel/mode status, data[3] is current channel
///   data[0] == 0x02 → power status, data[3]==1 ON, data[3]==2 STANDBY
const NEEWER_NOTIFY_CHAR: Uuid = Uuid::from_u128(0x69400003_b5a3_f393_e0a9_e50e24dcca99);

/// Decoded snapshot of what the light most recently reported via the
/// notify characteristic. None means we haven't received any notifications
/// for this light yet (just connected, or not subscribed). Power state is
/// the only field most lights actually emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyState {
    /// Reported power state. Some(true) = ON, Some(false) = STANDBY.
    Power(bool),
    /// Reported channel/mode index (light-specific meaning).
    Channel(u8),
    /// Bytes we received but couldn't classify. Logged but not actioned.
    Unknown,
}

fn parse_notify(bytes: &[u8]) -> NotifyState {
    if bytes.len() < 4 {
        return NotifyState::Unknown;
    }
    match bytes[0] {
        0x01 => NotifyState::Channel(bytes[3]),
        0x02 => NotifyState::Power(bytes[3] == 1),
        _ => NotifyState::Unknown,
    }
}

// ── Protocol (shared across all transports) ─────────────────────

const CMD_PREFIX: u8 = 0x78;
const TAG_POWER: u8 = 0x81;
const TAG_CCT: u8 = 0x87;

// GL1 UDP protocol
#[allow(dead_code)]
const GL1_PORT: u16 = 5052;

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u16, |acc, &b| acc + b as u16) as u8
}

fn cmd_power(on: bool) -> Vec<u8> {
    let state = if on { 0x01 } else { 0x02 };
    let payload = [CMD_PREFIX, TAG_POWER, 0x01, state];
    let chk = checksum(&payload);
    vec![CMD_PREFIX, TAG_POWER, 0x01, state, chk]
}

fn cmd_cct(brightness: u8, color_temp: u8) -> Vec<u8> {
    let payload = [CMD_PREFIX, TAG_CCT, 0x02, brightness, color_temp];
    let chk = checksum(&payload);
    vec![CMD_PREFIX, TAG_CCT, 0x02, brightness, color_temp, chk]
}

// Extended CCT: separate brightness and temperature commands (GL1 PRO, newer lights)
const TAG_LONG_CCT_BRT: u8 = 0x82;
const TAG_LONG_CCT_TEMP: u8 = 0x83;

// ── PL81 PRO USB serial protocol ───────────────────────────────
// Packet: [0x3A] [tag] [payload_len] [payload...] [checksum_hi] [checksum_lo]
// Checksum: 16-bit big-endian sum of ALL preceding bytes
// Source: https://github.com/m-rk/neewer-usb-control

const PL81_PREFIX: u8 = 0x3A;

fn pl81_checksum(bytes: &[u8]) -> [u8; 2] {
    let sum: u16 = bytes.iter().fold(0u16, |acc, &b| acc.wrapping_add(b as u16));
    [(sum >> 8) as u8, (sum & 0xFF) as u8]
}

fn pl81_cmd_cct(brightness: u8, color_temp_byte: u8) -> Vec<u8> {
    let payload = [PL81_PREFIX, 0x02, 0x03, 0x01, brightness, color_temp_byte];
    let cs = pl81_checksum(&payload);
    vec![PL81_PREFIX, 0x02, 0x03, 0x01, brightness, color_temp_byte, cs[0], cs[1]]
}

#[allow(dead_code)]
fn pl81_cmd_power(on: bool) -> Vec<u8> {
    let state = if on { 0x01 } else { 0x02 };
    let payload = [PL81_PREFIX, 0x06, 0x01, state];
    let cs = pl81_checksum(&payload);
    vec![PL81_PREFIX, 0x06, 0x01, state, cs[0], cs[1]]
}

/// Convert Kelvin to PL81 temp byte (0x00=2900K to 0x12=7000K, 19 steps)
fn kelvin_to_pl81_temp(k: u16) -> u8 {
    let k = k.clamp(2900, 7000);
    ((k - 2900) as f32 * 18.0 / 4100.0).round() as u8
}

fn cmd_long_cct_brightness(brightness: u8) -> Vec<u8> {
    let payload = [CMD_PREFIX, TAG_LONG_CCT_BRT, 0x01, brightness];
    let chk = checksum(&payload);
    vec![CMD_PREFIX, TAG_LONG_CCT_BRT, 0x01, brightness, chk]
}

fn cmd_long_cct_temp(color_temp: u8) -> Vec<u8> {
    let payload = [CMD_PREFIX, TAG_LONG_CCT_TEMP, 0x01, color_temp];
    let chk = checksum(&payload);
    vec![CMD_PREFIX, TAG_LONG_CCT_TEMP, 0x01, color_temp, chk]
}

// GL1 uses a different command format
fn gl1_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u16, |acc, &b| acc + b as u16) as u8
}

fn gl1_cmd_power(on: bool) -> Vec<u8> {
    if on {
        vec![0x80, 0x05, 0x02, 0x01, 0x01, 0x89]
    } else {
        vec![0x80, 0x05, 0x02, 0x01, 0x00, 0x88]
    }
}

fn gl1_cmd_cct(brightness: u8, color_temp_k: u16) -> Vec<u8> {
    // GL1 temp format: first two digits of kelvin value (e.g. 33 for 3300K, 56 for 5600K)
    let temp_byte = (color_temp_k / 100) as u8;
    let payload = [0x80, 0x05, 0x03, 0x02, brightness, temp_byte];
    let chk = gl1_checksum(&payload);
    vec![0x80, 0x05, 0x03, 0x02, brightness, temp_byte, chk]
}

#[allow(dead_code)]
fn gl1_handshake(local_ip: &str) -> Vec<u8> {
    // IP is sent as ASCII hex representation of each byte of the IP string
    let ip_as_ascii_hex: Vec<u8> = local_ip
        .bytes()
        .flat_map(|b| {
            let hi = b >> 4;
            let lo = b & 0x0f;
            let to_hex = |n: u8| if n < 10 { b'0' + n } else { b'a' + n - 10 };
            vec![to_hex(hi), to_hex(lo)]
        })
        .collect();
    let mut cmd = vec![0x80, 0x02, 0x10, 0x00, 0x00, 0x0d];
    cmd.extend_from_slice(&ip_as_ascii_hex);
    cmd.push(0x2e);
    cmd
}

// ── Light state ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LightState {
    pub on: bool,
    pub brightness: u8,       // 0-100
    pub color_temp_raw: u8,   // 0x20-0x38 for BLE (3200K-5600K)
    pub color_temp_k: u16,    // Kelvin for GL1
}

impl LightState {
    fn new() -> Self {
        Self {
            on: true,
            brightness: 50,
            color_temp_raw: 0x2C,
            color_temp_k: 4400,
        }
    }

    pub fn adjust_brightness(&mut self, delta: i8) {
        self.brightness = (self.brightness as i16 + delta as i16).clamp(0, 100) as u8;
    }

    pub fn adjust_temp(&mut self, delta: i16) {
        let raw_step: i16 = if delta > 0 { 1 } else { -1 };
        self.color_temp_raw = (self.color_temp_raw as i16 + raw_step).clamp(0x20, 0x38) as u8;
        self.color_temp_k = (self.color_temp_k as i16 + delta).clamp(2900, 7000) as u16;
    }

    pub fn reset_temp(&mut self) {
        self.color_temp_raw = 0x2C;
        self.color_temp_k = 4400;
    }
}

// ── Transport trait ─────────────────────────────────────────────

pub struct Light {
    pub name: String,
    pub is_gl1: bool,  // GL1 PRO uses long CCT (0x82/0x83) over BLE
    pub is_pl81: bool, // PL81 PRO uses 0x3A serial protocol
    transport: Transport,
    pub state: LightState,
}

/// Effective on-state for a light: prefer firmware-reported telemetry (BLE
/// notify) when present, otherwise fall back to the last-commanded state.
/// Telemetry catches cases where the light's physical button was pressed,
/// where a BLE write was silently dropped, or where the firmware refused
/// the command.
fn effective_on(light: &Light) -> bool {
    match light.actual_notify_state() {
        Some(NotifyState::Power(on)) => on,
        _ => light.state.on,
    }
}

#[allow(dead_code)]
pub fn any_on(lights: &[Light]) -> bool {
    lights.iter().any(|l| l.state.on)
}

pub fn keylights_on(lights: &[Light]) -> bool {
    lights.iter().filter(|l| l.is_gl1).any(|l| l.state.on)
}

pub fn desklights_on(lights: &[Light]) -> bool {
    lights.iter().filter(|l| l.is_pl81).any(|l| l.state.on)
}

#[allow(dead_code)]
enum Transport {
    /// BLE peripheral wrapped by the native CoreBluetooth path. The shared
    /// `BleManager` owns the central manager and handles auto-reconnect via
    /// its delegate; this transport just carries a handle to the peripheral
    /// and a clone of the manager Arc so writes can be issued.
    Ble {
        peripheral: BlePeripheral,
        manager: Arc<BleManager>,
        last_heartbeat: Instant,
    },
    Serial {
        /// `None` once a write has failed and the handle was dropped; the next
        /// write reopens it from `path`. Kept live the rest of the time.
        port: Option<Box<dyn serialport::SerialPort>>,
        path: String,
        /// Last time we attempted a reopen. Throttles open() attempts so an
        /// unplugged light can't turn a held-down button into an open() storm.
        last_reopen: Instant,
    },
    Udp {
        socket: UdpSocket,
        broadcast_addr: String,
        handshake: Vec<u8>,
        last_heartbeat: Instant,
    },
}

/// How long we'll wait for the initial connect at boot. CoreBluetooth queues
/// the request indefinitely once issued, so this only bounds the *await*;
/// the request keeps trying in the background even if we time out here.
const BLE_INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

impl Light {
    /// Re-send GL1 handshake if it's been more than 10 seconds since the last one
    fn ensure_gl1_alive(&mut self) {
        if let Transport::Udp { socket, broadcast_addr, handshake, last_heartbeat } = &mut self.transport
            && last_heartbeat.elapsed() > Duration::from_secs(10)
        {
            for _ in 0..2 {
                socket.send_to(handshake, broadcast_addr.as_str()).ok();
            }
            *last_heartbeat = std::time::Instant::now();
        }
    }

    pub fn set_power(&mut self, on: bool) -> Result<(), String> {
        self.ensure_gl1_alive();
        self.state.on = on;

        match &mut self.transport {
            Transport::Ble { peripheral, manager, .. } => {
                let cmd = cmd_power(on);
                manager.write(peripheral, NEEWER_WRITE_CHAR, &cmd)
            }
            Transport::Serial { port, path, last_reopen } => {
                // PL81: use brightness 0/100 since power command is unreliable
                let cmd = if self.is_pl81 {
                    let brt = if on { 100 } else { 0 };
                    pl81_cmd_cct(brt, kelvin_to_pl81_temp(self.state.color_temp_k))
                } else {
                    cmd_power(on)
                };
                serial_write_healing(port, path, last_reopen, &cmd)
            }
            Transport::Udp { socket, broadcast_addr, .. } => {
                let cmd = gl1_cmd_power(on);
                udp_write(socket, broadcast_addr, &cmd)
            }
        }
    }

    pub fn toggle_power(&mut self) -> Result<(), String> {
        let on = !effective_on(self);
        self.set_power(on)
    }

    pub fn adjust_brightness(&mut self, delta: i8) -> Result<(), String> {
        self.state.adjust_brightness(delta);
        self.send_cct()
    }

    pub fn adjust_temp(&mut self, delta: i16) -> Result<(), String> {
        self.state.adjust_temp(delta);
        self.send_cct()
    }

    pub fn reset_temp(&mut self) -> Result<(), String> {
        self.state.reset_temp();
        self.send_cct()
    }

    pub fn set_preset(&mut self, brightness: u8, temp_k: u16) -> Result<(), String> {
        self.state.brightness = brightness.clamp(0, 100);
        self.state.color_temp_k = temp_k.clamp(2900, 7000);
        self.state.color_temp_raw = ((temp_k.clamp(3200, 5600) - 3200) as f32 / 100.0) as u8 + 0x20;
        info!("[{}] preset: brightness={}, temp={}K", self.name, brightness, temp_k);
        if !self.state.on {
            self.set_power(true)?;
        }
        self.send_cct()
    }

    /// Manual recovery kick. Native auto-reconnect handles drops on its
    /// own; this just nudges connect() in case the user pressed rescan
    /// because something looked wrong. Idempotent if already connected.
    pub fn force_reconnect(&mut self) {
        if let Transport::Ble { peripheral, manager, .. } = &self.transport {
            // Reissue connect synchronously-fire-and-forget. CoreBluetooth
            // queues, so this won't block. The delegate's didConnect will
            // resync state via the heartbeat.
            let _ = manager.kick_reconnect(peripheral);
        }
    }

    /// Read what the light most recently reported via its notify
    /// characteristic, decoded. None for non-BLE lights or BLE lights that
    /// haven't sent a notification yet. Power(true) means the light's
    /// firmware reported it's ON; Power(false) means STANDBY.
    pub fn actual_notify_state(&self) -> Option<NotifyState> {
        let Transport::Ble { peripheral, manager, .. } = &self.transport else {
            return None;
        };
        manager.last_notify(peripheral).map(|b| parse_notify(&b))
    }


    fn send_cct(&mut self) -> Result<(), String> {
        self.ensure_gl1_alive();
        info!(
            "[{}] brightness={}, temp={}K",
            self.name, self.state.brightness, self.state.color_temp_k
        );

        if self.is_pl81 {
            let temp_byte = kelvin_to_pl81_temp(self.state.color_temp_k);
            let cmd = pl81_cmd_cct(self.state.brightness, temp_byte);
            match &mut self.transport {
                Transport::Serial { port, path, last_reopen } => {
                    serial_write_healing(port, path, last_reopen, &cmd)
                }
                _ => Ok(()),
            }
        } else if self.is_gl1 {
            match &mut self.transport {
                Transport::Ble { peripheral, manager, .. } => {
                    // GL1 PRO: brightness and temp go in two separate writes
                    // (TAG_LONG_CCT_BRT then TAG_LONG_CCT_TEMP).
                    let brt = cmd_long_cct_brightness(self.state.brightness);
                    let temp = cmd_long_cct_temp(self.state.color_temp_raw);
                    manager.write(peripheral, NEEWER_WRITE_CHAR, &brt)?;
                    manager.write(peripheral, NEEWER_WRITE_CHAR, &temp)
                }
                Transport::Udp { socket, broadcast_addr, .. } => {
                    let cmd = gl1_cmd_cct(self.state.brightness, self.state.color_temp_k);
                    udp_write(socket, broadcast_addr, &cmd)
                }
                _ => Ok(()),
            }
        } else {
            match &mut self.transport {
                Transport::Ble { peripheral, manager, .. } => {
                    let cmd = cmd_cct(self.state.brightness, self.state.color_temp_raw);
                    manager.write(peripheral, NEEWER_WRITE_CHAR, &cmd)
                }
                Transport::Serial { port, path, last_reopen } => {
                    let cmd = cmd_cct(self.state.brightness, self.state.color_temp_raw);
                    serial_write_healing(port, path, last_reopen, &cmd)
                }
                Transport::Udp { socket, broadcast_addr, .. } => {
                    let cmd = cmd_cct(self.state.brightness, self.state.color_temp_raw);
                    udp_write(socket, broadcast_addr, &cmd)
                }
            }
        }
    }
}

/// Force-reconnect every BLE light. Used by the rescan button and by
/// macOS wake-from-sleep recovery. With the native path, auto-reconnect
/// already runs on every disconnect; this just nudges connect() in case
/// the user pressed rescan because something looked wrong.
pub fn force_reconnect_all(lights: &mut [Light]) {
    let mut count = 0usize;
    for l in lights.iter_mut() {
        if matches!(l.transport, Transport::Ble { .. }) {
            l.force_reconnect();
            count += 1;
        }
    }
    info!("Rescan: kicking {} BLE light(s)", count);
}


// ── Serial transport ────────────────────────────────────────────

const SERIAL_BAUD: u32 = 115200;
const SERIAL_TIMEOUT: Duration = Duration::from_millis(100);

/// Minimum gap between reopen attempts for one serial light. The PL81's USB
/// serial fd dies (EPIPE) on a hub blip or USB selective-suspend without the
/// Stream Deck necessarily dropping, and deckd has no other recovery path for
/// it. This throttle keeps a genuinely-unplugged light from turning every
/// queued command into an open() storm.
const SERIAL_REOPEN_COOLDOWN: Duration = Duration::from_secs(3);

/// An `Instant` far enough in the past that the first write is allowed to
/// reopen immediately. Avoids `Instant - Duration` underflow panics on
/// platforms where the monotonic clock starts near zero.
fn stale_instant() -> Instant {
    Instant::now()
        .checked_sub(SERIAL_REOPEN_COOLDOWN)
        .unwrap_or_else(Instant::now)
}

/// Write to a serial light, self-healing a dead handle. On the happy path this
/// is just `write_all`. If the handle is gone or the write fails, we drop the
/// handle and — at most once per `SERIAL_REOPEN_COOLDOWN` — reopen the port
/// from `path` and retry the write exactly once. Bounded work per call, so a
/// wedged port can never spin: failed reopens just return `Err` until the
/// cooldown lapses again on the next user action.
fn serial_write_healing(
    port: &mut Option<Box<dyn serialport::SerialPort>>,
    path: &str,
    last_reopen: &mut Instant,
    cmd: &[u8],
) -> Result<(), String> {
    // Fast path: live handle and the write lands.
    if let Some(p) = port.as_mut()
        && p.write_all(cmd).is_ok()
    {
        return Ok(());
    }

    // The handle is missing or the write just failed. Drop the dead one and
    // consider reopening — but back off if we tried too recently.
    *port = None;
    if last_reopen.elapsed() < SERIAL_REOPEN_COOLDOWN {
        return Err(format!("Serial {} unavailable (reopen on cooldown)", path));
    }
    *last_reopen = Instant::now();

    let mut fresh = serialport::new(path, SERIAL_BAUD)
        .timeout(SERIAL_TIMEOUT)
        .open()
        .map_err(|e| format!("Serial reopen {}: {}", path, e))?;
    fresh
        .write_all(cmd)
        .map_err(|e| format!("Serial write after reopen {}: {}", path, e))?;
    *port = Some(fresh);
    warn!("Serial: reopened {} after write failure", path);
    Ok(())
}

fn discover_serial_lights() -> Vec<Light> {
    let ports = serialport::available_ports().unwrap_or_default();
    let mut lights = Vec::new();

    for port in ports {
        // CH340 chips used by Neewer PL81 have vendor ID 0x1A86 (6790)
        let is_neewer = match &port.port_type {
            serialport::SerialPortType::UsbPort(usb) => usb.vid == 0x1A86,
            _ => false,
        };
        // Only use cu.* ports (not tty.*)
        if port.port_name.contains("/dev/tty.") {
            continue;
        }

        if !is_neewer {
            continue;
        }

        let label = match &port.port_type {
            serialport::SerialPortType::UsbPort(usb) => {
                usb.product.clone().unwrap_or_else(|| "Neewer USB".into())
            }
            _ => "Neewer USB".into(),
        };

        info!("Serial: found {} at {}", label, port.port_name);
        match serialport::new(&port.port_name, SERIAL_BAUD)
            .timeout(SERIAL_TIMEOUT)
            .open()
        {
            Ok(serial_port) => {
                lights.push(Light {
                    name: format!("{} ({})", label, port.port_name),
                    is_gl1: false,
                    is_pl81: true,
                    transport: Transport::Serial {
                        port: Some(serial_port),
                        path: port.port_name.clone(),
                        last_reopen: stale_instant(),
                    },
                    state: LightState::new(),
                });
            }
            Err(e) => {
                warn!("Serial: could not open {}: {} (is Neewer Control Center running?)", port.port_name, e);
            }
        }
    }

    lights
}

// ── UDP transport (GL1) ─────────────────────────────────────────

fn udp_write(socket: &UdpSocket, addr: &str, cmd: &[u8]) -> Result<(), String> {
    socket
        .send_to(cmd, addr)
        .map_err(|e| format!("UDP send: {}", e))?;
    Ok(())
}

#[allow(dead_code)]
fn get_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

#[allow(dead_code)]
fn discover_gl1_lights() -> Vec<Light> {
    let local_ip = match get_local_ip() {
        Some(ip) => ip,
        None => {
            warn!("GL1: could not determine local IP");
            return Vec::new();
        }
    };

    // Derive broadcast address from local IP (assume /24)
    let parts: Vec<&str> = local_ip.split('.').collect();
    if parts.len() != 4 {
        warn!("GL1: unexpected IP format: {}", local_ip);
        return Vec::new();
    }
    let broadcast = format!("{}.{}.{}.255:{}", parts[0], parts[1], parts[2], GL1_PORT);

    info!("GL1: broadcasting to {} (from {})", broadcast, local_ip);

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            warn!("GL1: socket bind failed: {}", e);
            return Vec::new();
        }
    };
    socket.set_broadcast(true).ok();

    // Send handshake to broadcast — all GL1s on the subnet will accept commands
    let ip_hex: Vec<u8> = local_ip.bytes().collect();
    let mut handshake = vec![0x80, 0x02, 0x10, 0x00, 0x00, 0x0d];
    handshake.extend_from_slice(&ip_hex);
    handshake.push(0x2e);

    for _ in 0..3 {
        socket.send_to(&handshake, &broadcast).ok();
        std::thread::sleep(Duration::from_millis(100));
    }
    info!("GL1: handshake sent");

    // We treat all GL1s as one broadcast group
    vec![Light {
        name: "GL1 PRO (broadcast)".into(),
        is_gl1: true,
        is_pl81: false,
        transport: Transport::Udp {
            socket,
            broadcast_addr: broadcast,
            handshake,
            last_heartbeat: std::time::Instant::now(),
        },
        state: LightState::new(),
    }]
}

// ── Public discovery functions ──────────────────────────────────

/// Discover USB serial lights (instant, no scanning delay)
pub fn discover_serial() -> Vec<Light> {
    info!("Serial: scanning USB...");
    let lights = discover_serial_lights();
    info!("Serial: found {} USB light(s)", lights.len());
    for light in &lights {
        info!("  - {}", light.name);
    }
    lights
}

/// Drop any existing USB-serial lights and re-open the current set. Used
/// after a USB hub blip / Stream Deck re-attach to recover from `EPIPE`s on
/// stale serial handles. BLE lights are left untouched — their actors have
/// their own reconnect path.
pub fn refresh_serial(all_lights: &mut Vec<Light>) {
    all_lights.retain(|l| !matches!(l.transport, Transport::Serial { .. }));
    let fresh = discover_serial_lights();
    info!("Serial: refreshed {} USB light(s) after USB event", fresh.len());
    for l in &fresh {
        info!("  - {}", l.name);
    }
    all_lights.extend(fresh);
}

/// Same as `discover_serial` but skips ports whose label is already in
/// `existing_names`. Used by the rescan button so we don't double-open a
/// serial port that an existing `Light` still owns.
pub fn discover_serial_excluding(existing_names: &std::collections::HashSet<String>) -> Vec<Light> {
    info!("Serial: rescanning USB...");
    let lights: Vec<Light> = discover_serial_lights()
        .into_iter()
        .filter(|l| !existing_names.contains(&l.name))
        .collect();
    info!("Serial: found {} new USB light(s)", lights.len());
    for light in &lights {
        info!("  - {}", light.name);
    }
    lights
}

// ── BLE discovery + persistence ─────────────────────────────────

/// Persisted record of a BLE peripheral so subsequent boots can reach it
/// via `retrievePeripheralsWithIdentifiers:` without scanning. Saving the
/// name and is_gl1 flag here lets us bypass scan even on cold boot.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct StoredPeripheral {
    uuid: Uuid,
    name: String,
    is_gl1: bool,
}

fn ble_state_path() -> std::path::PathBuf {
    let mut p = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    p.push("deckd");
    p.push("ble_peripherals.json");
    p
}

fn load_stored_peripherals() -> Vec<StoredPeripheral> {
    let path = ble_state_path();
    let Ok(data) = std::fs::read_to_string(&path) else { return Vec::new() };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_stored_peripherals(peripherals: &[StoredPeripheral]) {
    let path = ble_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(data) = serde_json::to_string_pretty(peripherals) else { return };
    if let Err(e) = std::fs::write(&path, data) {
        warn!("Failed to save BLE peripherals to {:?}: {}", path, e);
    }
}

/// Discover BLE lights using the native CoreBluetooth path. Strategy:
/// - If a saved peripherals file exists, retrieve those by UUID without
///   scanning. This is the steady-state path: instant boot recovery
///   regardless of whether the lights are advertising right now.
/// - If no file exists (or `expected` says we want more lights than
///   we have), do a native scan, filter for NEEWER, persist the result.
///
/// `existing_names` is honored to avoid duplicates when called from the
/// rescan button mid-runtime. `expected` is the configured light count
/// from `expected_ble_lights`; when set, we'll scan if we don't have it.
pub fn discover_ble(
    rt: &tokio::runtime::Runtime,
    existing_names: std::collections::HashSet<String>,
    expected: Option<usize>,
) -> Vec<Light> {
    let manager = match rt.block_on(BleManager::new()) {
        Ok(m) => m,
        Err(e) => {
            warn!("BLE manager init failed: {}", e);
            return Vec::new();
        }
    };
    // Auto-subscribe to the Neewer notify characteristic on every peripheral
    // we (re)discover. The manager re-attaches the subscription on every
    // reconnect, so a drop+auto-reconnect doesn't lose state visibility.
    manager.watch_characteristic(NEEWER_NOTIFY_CHAR);

    let mut stored = load_stored_peripherals();
    let mut lights: Vec<Light> = Vec::new();

    // Step 1: hydrate Lights from any stored peripherals we don't already
    // have. Retrieve gives us fresh CBPeripheral handles by UUID — no scan.
    if !stored.is_empty() {
        let want_uuids: Vec<Uuid> = stored.iter().map(|s| s.uuid).collect();
        let peripherals = manager.retrieve(&want_uuids);
        info!(
            "BLE: retrieved {}/{} stored peripheral(s)",
            peripherals.len(),
            stored.len()
        );
        for p in peripherals {
            let stored_entry = match stored.iter().find(|s| s.uuid == p.uuid) {
                Some(s) => s.clone(),
                None => continue,
            };
            if existing_names.contains(&stored_entry.name) {
                continue;
            }
            // Kick off connect in the background — auto-reconnect via the
            // delegate keeps it trying. We don't await: writes against a
            // disconnected peripheral fail gracefully and the heartbeat
            // resyncs once the link is up.
            let mgr_clone = Arc::clone(&manager);
            let p_clone = p.clone();
            rt.spawn(async move {
                match mgr_clone.connect(&p_clone, BLE_INITIAL_CONNECT_TIMEOUT).await {
                    Ok(()) => info!("BLE: initial connect ok for {}", p_clone.name),
                    Err(e) => warn!(
                        "BLE: initial connect for {} returned {}; auto-reconnect will keep trying",
                        p_clone.name, e
                    ),
                }
            });
            info!("BLE: tracked stored peripheral {}", stored_entry.name);
            lights.push(Light {
                name: stored_entry.name.clone(),
                is_gl1: stored_entry.is_gl1,
                is_pl81: false,
                transport: Transport::Ble {
                    peripheral: p,
                    manager: Arc::clone(&manager),
                    last_heartbeat: Instant::now(),
                },
                state: LightState::new(),
            });
        }
    }

    // Step 2: if we still need more lights (first run, or new light), scan.
    let need_scan = match expected {
        Some(n) => lights.len() < n,
        None => stored.is_empty(),
    };
    if need_scan {
        info!("BLE: scanning for new peripherals...");
        let mgr_for_scan = Arc::clone(&manager);
        let advertisements = rt.block_on(async move {
            mgr_for_scan.scan(Duration::from_secs(10)).await
        });
        let neewer: Vec<_> = advertisements
            .into_iter()
            .filter(|a| {
                a.name
                    .as_ref()
                    .map(|n| n.to_uppercase().starts_with("NEEWER"))
                    .unwrap_or(false)
            })
            .collect();
        info!("BLE: scan saw {} NEEWER peripheral(s)", neewer.len());

        let known_uuids: std::collections::HashSet<Uuid> =
            stored.iter().map(|s| s.uuid).collect();
        let new_uuids: Vec<Uuid> = neewer
            .iter()
            .map(|a| a.uuid)
            .filter(|u| !known_uuids.contains(u))
            .collect();
        if !new_uuids.is_empty() {
            let new_peripherals = manager.retrieve(&new_uuids);
            for p in new_peripherals {
                let adv_name = neewer
                    .iter()
                    .find(|a| a.uuid == p.uuid)
                    .and_then(|a| a.name.clone())
                    .unwrap_or_else(|| p.name.clone());
                let is_gl1 = adv_name.to_uppercase().contains("GL1");
                let display_name = format!("{} [{}]", adv_name, p.uuid);
                if existing_names.contains(&display_name) {
                    continue;
                }
                stored.push(StoredPeripheral {
                    uuid: p.uuid,
                    name: display_name.clone(),
                    is_gl1,
                });
                let mgr_clone = Arc::clone(&manager);
                let p_clone = p.clone();
                rt.spawn(async move {
                    let _ = mgr_clone
                        .connect(&p_clone, BLE_INITIAL_CONNECT_TIMEOUT)
                        .await;
                });
                info!("BLE: discovered + tracked {}", display_name);
                lights.push(Light {
                    name: display_name,
                    is_gl1,
                    is_pl81: false,
                    transport: Transport::Ble {
                        peripheral: p,
                        manager: Arc::clone(&manager),
                        last_heartbeat: Instant::now(),
                    },
                    state: LightState::new(),
                });
            }
            save_stored_peripherals(&stored);
        }
    }

    lights
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_8bit_sum() {
        assert_eq!(checksum(&[0x78, 0x81, 0x01, 0x01]), 0xFB);
        assert_eq!(checksum(&[0x78, 0x81, 0x01, 0x02]), 0xFC);
        // Verify wrapping: sum > 255 truncates to low byte
        assert_eq!(checksum(&[0xFF, 0xFF]), 0xFE);
    }

    #[test]
    fn cmd_power_on() {
        assert_eq!(cmd_power(true), vec![0x78, 0x81, 0x01, 0x01, 0xFB]);
    }

    #[test]
    fn cmd_power_off() {
        assert_eq!(cmd_power(false), vec![0x78, 0x81, 0x01, 0x02, 0xFC]);
    }

    #[test]
    fn cmd_cct_checksum() {
        let pkt = cmd_cct(50, 0x2C);
        assert_eq!(pkt.len(), 6);
        assert_eq!(pkt[0], CMD_PREFIX);
        assert_eq!(pkt[1], TAG_CCT);
        assert_eq!(pkt[2], 0x02);
        assert_eq!(pkt[3], 50);
        assert_eq!(pkt[4], 0x2C);
        let expected_chk = checksum(&[CMD_PREFIX, TAG_CCT, 0x02, 50, 0x2C]);
        assert_eq!(pkt[5], expected_chk);
    }

    #[test]
    fn cmd_long_cct_brightness_bytes() {
        let pkt = cmd_long_cct_brightness(80);
        assert_eq!(&pkt[..4], &[0x78, 0x82, 0x01, 0x50]);
        let expected_chk = checksum(&[0x78, 0x82, 0x01, 0x50]);
        assert_eq!(pkt[4], expected_chk);
    }

    #[test]
    fn cmd_long_cct_temp_bytes() {
        let pkt = cmd_long_cct_temp(0x2C);
        assert_eq!(&pkt[..4], &[0x78, 0x83, 0x01, 0x2C]);
        let expected_chk = checksum(&[0x78, 0x83, 0x01, 0x2C]);
        assert_eq!(pkt[4], expected_chk);
    }

    #[test]
    fn pl81_checksum_is_big_endian_16bit_sum() {
        let cs = pl81_checksum(&[0x3A, 0x02, 0x03, 0x01, 100, 0x09]);
        let sum: u16 = [0x3Au16, 0x02, 0x03, 0x01, 100, 0x09].iter().sum();
        assert_eq!(cs, [(sum >> 8) as u8, (sum & 0xFF) as u8]);
    }

    #[test]
    fn pl81_cmd_cct_format() {
        let pkt = pl81_cmd_cct(100, 0x09);
        assert_eq!(pkt[0], PL81_PREFIX);
        assert_eq!(pkt[1], 0x02);
        assert_eq!(pkt[2], 0x03);
        assert_eq!(pkt[3], 0x01);
        assert_eq!(pkt[4], 100);
        assert_eq!(pkt[5], 0x09);
        let cs = pl81_checksum(&pkt[..6]);
        assert_eq!(&pkt[6..], &cs);
    }

    #[test]
    fn kelvin_to_pl81_temp_boundaries() {
        assert_eq!(kelvin_to_pl81_temp(2900), 0x00);
        assert_eq!(kelvin_to_pl81_temp(7000), 0x12);
        // Clamp below minimum
        assert_eq!(kelvin_to_pl81_temp(1000), 0x00);
        // Clamp above maximum
        assert_eq!(kelvin_to_pl81_temp(9000), 0x12);
    }

    #[test]
    fn kelvin_to_pl81_temp_midrange() {
        let mid = kelvin_to_pl81_temp(4400);
        // 4400K is (4400-2900)/4100 * 18 = 1500/4100 * 18 ≈ 6.59 → rounds to 7
        assert_eq!(mid, 7);
    }
}
