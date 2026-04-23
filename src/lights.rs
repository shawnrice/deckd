use std::io::Write as IoWrite;
use std::net::UdpSocket;
use std::time::Duration;

use log::{info, warn};

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
    Ble {
        peripheral: btleplug::platform::Peripheral,
    },
    Serial {
        port: Box<dyn serialport::SerialPort>,
    },
    Udp {
        socket: UdpSocket,
        broadcast_addr: String,
        handshake: Vec<u8>,
        last_heartbeat: std::time::Instant,
    },
}

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

    pub fn set_power(&mut self, on: bool, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.ensure_gl1_alive();
        self.state.on = on;

        match &mut self.transport {
            Transport::Ble { peripheral } => {
                let cmd = cmd_power(on); // 78 81 works for all BLE lights
                rt.block_on(ble_write(peripheral, &cmd))
            }
            Transport::Serial { port } => {
                // PL81: use brightness 0/100 since power command is unreliable
                let cmd = if self.is_pl81 {
                    let brt = if on { 100 } else { 0 };
                    pl81_cmd_cct(brt, kelvin_to_pl81_temp(self.state.color_temp_k))
                } else {
                    cmd_power(on)
                };
                serial_write(port, &cmd)
            }
            Transport::Udp { socket, broadcast_addr, .. } => {
                let cmd = gl1_cmd_power(on);
                udp_write(socket, broadcast_addr, &cmd)
            }
        }
    }

    pub fn toggle_power(&mut self, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        let on = !self.state.on;
        self.set_power(on, rt)
    }

    pub fn adjust_brightness(&mut self, delta: i8, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.state.adjust_brightness(delta);
        self.send_cct(rt)
    }

    pub fn adjust_temp(&mut self, delta: i16, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.state.adjust_temp(delta);
        self.send_cct(rt)
    }

    pub fn reset_temp(&mut self, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.state.reset_temp();
        self.send_cct(rt)
    }

    pub fn set_preset(&mut self, brightness: u8, temp_k: u16, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.state.brightness = brightness.clamp(0, 100);
        self.state.color_temp_k = temp_k.clamp(2900, 7000);
        self.state.color_temp_raw = ((temp_k.clamp(3200, 5600) - 3200) as f32 / 100.0) as u8 + 0x20;
        info!("[{}] preset: brightness={}, temp={}K", self.name, brightness, temp_k);
        // Turn on if off
        if !self.state.on {
            self.set_power(true, rt)?;
        }
        self.send_cct(rt)
    }

    fn send_cct(&mut self, rt: &tokio::runtime::Runtime) -> Result<(), String> {
        self.ensure_gl1_alive();
        info!(
            "[{}] brightness={}, temp={}K",
            self.name, self.state.brightness, self.state.color_temp_k
        );

        if self.is_pl81 {
            // PL81 PRO: 0x3A serial protocol with 16-bit checksum
            let temp_byte = kelvin_to_pl81_temp(self.state.color_temp_k);
            let cmd = pl81_cmd_cct(self.state.brightness, temp_byte);
            match &mut self.transport {
                Transport::Serial { port } => serial_write(port, &cmd),
                _ => Ok(()),
            }
        } else if self.is_gl1 {
            // GL1 PRO: separate brightness + temperature commands (0x82, 0x83)
            let brt_cmd = cmd_long_cct_brightness(self.state.brightness);
            let temp_cmd = cmd_long_cct_temp(self.state.color_temp_raw);
            match &mut self.transport {
                Transport::Ble { peripheral } => {
                    rt.block_on(ble_write(peripheral, &brt_cmd))?;
                    rt.block_on(ble_write(peripheral, &temp_cmd))
                }
                Transport::Udp { socket, broadcast_addr, .. } => {
                    let cmd = gl1_cmd_cct(self.state.brightness, self.state.color_temp_k);
                    udp_write(socket, broadcast_addr, &cmd)
                }
                _ => Ok(()),
            }
        } else {
            // Standard Neewer BLE CCT
            let cmd = cmd_cct(self.state.brightness, self.state.color_temp_raw);
            match &mut self.transport {
                Transport::Ble { peripheral } => rt.block_on(ble_write(peripheral, &cmd)),
                Transport::Serial { port } => serial_write(port, &cmd),
                Transport::Udp { socket, broadcast_addr, .. } => udp_write(socket, broadcast_addr, &cmd),
            }
        }
    }
}

// ── BLE transport ───────────────────────────────────────────────

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::Manager;
use uuid::Uuid;

const NEEWER_SERVICE: Uuid = Uuid::from_u128(0x69400001_b5a3_f393_e0a9_e50e24dcca99);
const NEEWER_WRITE_CHAR: Uuid = Uuid::from_u128(0x69400002_b5a3_f393_e0a9_e50e24dcca99);

/// Timeout for individual BLE writes. CoreBluetooth can wedge for seconds or
/// forever after sleep/wake cycles; without a bound, a single write can freeze
/// the main event loop (see `rt.block_on(ble_write(..))` callers).
const BLE_WRITE_TIMEOUT: Duration = Duration::from_millis(500);

async fn ble_write(
    peripheral: &btleplug::platform::Peripheral,
    cmd: &[u8],
) -> Result<(), String> {
    let chars = peripheral.characteristics();
    let write_char = chars
        .iter()
        .find(|c| c.uuid == NEEWER_WRITE_CHAR)
        .ok_or("BLE write characteristic not found")?;

    let write_fut = peripheral.write(write_char, cmd, WriteType::WithoutResponse);
    match tokio::time::timeout(BLE_WRITE_TIMEOUT, write_fut).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("BLE write failed: {}", e)),
        Err(_) => Err(format!("BLE write timed out after {:?}", BLE_WRITE_TIMEOUT)),
    }
}

async fn discover_ble_lights() -> Result<Vec<Light>, String> {
    let manager = Manager::new()
        .await
        .map_err(|e| format!("BLE manager: {}", e))?;
    let adapters = manager
        .adapters()
        .await
        .map_err(|e| format!("BLE adapters: {}", e))?;
    let adapter = adapters.into_iter().next().ok_or("No BLE adapter")?;

    info!("BLE: scanning (5s)...");
    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|e| format!("BLE scan: {}", e))?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    adapter.stop_scan().await.ok();

    let peripherals = adapter
        .peripherals()
        .await
        .map_err(|e| format!("BLE peripherals: {}", e))?;

    let mut lights = Vec::new();
    for peripheral in peripherals {
        let props = match peripheral.properties().await.ok().flatten() {
            Some(p) => p,
            None => continue,
        };

        let name = props.local_name.unwrap_or_default();
        let name_upper = name.to_uppercase();

        let is_neewer = name_upper.starts_with("NEEWER")
            || name_upper.starts_with("NW-")
            || name_upper.starts_with("NW ")
            || props.services.contains(&NEEWER_SERVICE);

        if !is_neewer {
            continue;
        }

        info!("BLE: found {}", name);
        if !peripheral.is_connected().await.unwrap_or(false) {
            peripheral
                .connect()
                .await
                .map_err(|e| format!("BLE connect {}: {}", name, e))?;
        }
        peripheral.discover_services().await.ok();

        let has_service = peripheral
            .services()
            .iter()
            .any(|s| s.uuid == NEEWER_SERVICE);

        if !has_service {
            warn!("BLE: {} has no Neewer service, skipping", name);
            peripheral.disconnect().await.ok();
            continue;
        }

        let is_gl1 = name_upper.contains("GL1");
        info!("BLE: connected to {}{}", name, if is_gl1 { " (GL1 format)" } else { "" });
        lights.push(Light {
            name: name.clone(),
            is_gl1,
            is_pl81: false,
            transport: Transport::Ble { peripheral },
            state: LightState::new(),
        });
    }

    Ok(lights)
}

// ── Serial transport ────────────────────────────────────────────

fn serial_write(port: &mut Box<dyn serialport::SerialPort>, cmd: &[u8]) -> Result<(), String> {
    port.write_all(cmd)
        .map_err(|e| format!("Serial write: {}", e))
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
        match serialport::new(&port.port_name, 115200)
            .timeout(Duration::from_millis(100))
            .open()
        {
            Ok(serial_port) => {
                lights.push(Light {
                    name: format!("{} ({})", label, port.port_name),
                    is_gl1: false,
                    is_pl81: true,
                    transport: Transport::Serial {
                        port: serial_port,
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
    let lights = discover_serial_lights();
    for light in &lights {
        info!("  - {}", light.name);
    }
    lights
}

/// Hard ceiling on BLE discovery. The scan itself sleeps 5s, then each
/// peripheral may require connect + service discovery — budget is ~10s of
/// btleplug work even on a healthy adapter. We cap at 15s so a wedged
/// CoreBluetooth future (common after sleep/wake) can't strand the boot
/// state machine: on timeout we return an empty Vec, the spawned thread's
/// `tx.send(..)` unblocks, `ble_pending` clears, and the LCD can leave the
/// boot animation.
const BLE_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Discover BLE lights (takes ~5s for scanning, bounded by BLE_DISCOVERY_TIMEOUT)
/// Must be called from a thread with its own tokio runtime.
pub fn discover_ble(rt: &tokio::runtime::Runtime) -> Vec<Light> {
    let lights = rt.block_on(async {
        match tokio::time::timeout(BLE_DISCOVERY_TIMEOUT, discover_ble_lights()).await {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => {
                warn!("BLE discovery failed: {}", e);
                Vec::new()
            }
            Err(_) => {
                warn!("BLE discovery timed out after {:?}", BLE_DISCOVERY_TIMEOUT);
                Vec::new()
            }
        }
    });
    for light in &lights {
        info!("  - {}", light.name);
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
