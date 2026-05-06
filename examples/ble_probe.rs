// Standalone probe for iterating on BLE reconnect strategy without
// restarting deckd. Runs alongside the daemon — CoreBluetooth allows
// multiple processes (and multiple CBCentralManagers per process) to
// scan and connect concurrently.
//
// Each subcommand is one experiment. Add more as we test theories.
//
//   cargo run --example ble_probe -- fresh-scan
//   cargo run --example ble_probe -- fresh-scan --duration 15
//   cargo run --example ble_probe -- service-filter
//
// The validation criterion: if Neewer Control Center can connect to the
// lights, the probe must also be able to reach them with whatever
// strategy it's testing. That's the bar for porting a strategy back to
// deckd.

use std::time::{Duration, Instant};

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral};
use uuid::Uuid;

const NEEWER_SERVICE: Uuid = Uuid::from_u128(0x69400001_b5a3_f393_e0a9_e50e24dcca99);
const NEEWER_WRITE_CHAR: Uuid = Uuid::from_u128(0x69400002_b5a3_f393_e0a9_e50e24dcca99);

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");

    match cmd {
        "fresh-scan" => {
            let duration = parse_duration(&args).unwrap_or(Duration::from_secs(5));
            experiment_fresh_scan(duration).await;
        }
        "service-filter" => {
            let duration = parse_duration(&args).unwrap_or(Duration::from_secs(5));
            experiment_service_filter(duration).await;
        }
        "connect-and-wait" => {
            let scan = parse_duration(&args).unwrap_or(Duration::from_secs(3));
            experiment_connect_and_wait(scan).await;
        }
        "retrieve" => {
            let uuids: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
            if uuids.is_empty() {
                eprintln!("retrieve takes one or more peripheral UUIDs as args");
                return;
            }
            experiment_retrieve(&uuids).await;
        }
        "native-connect" => {
            let uuids: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
            if uuids.is_empty() {
                eprintln!("native-connect takes one or more peripheral UUIDs as args");
                return;
            }
            experiment_native_connect(&uuids).await;
        }
        "native-toggle" => {
            let uuids: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
            if uuids.is_empty() {
                eprintln!("native-toggle takes one or more peripheral UUIDs as args");
                return;
            }
            experiment_native_toggle(&uuids).await;
        }
        _ => {
            eprintln!(
                "Usage: ble_probe <subcommand> [--duration <secs>]\n\n\
                 Subcommands:\n  \
                   fresh-scan       Fresh Manager, scan, list all peripherals.\n  \
                                    Tells us whether the lights are visible at all.\n  \
                   service-filter   Fresh Manager, scan with NEEWER_SERVICE filter, list.\n  \
                                    Tells us whether macOS is filtering them out by default.\n  \
                   connect-and-wait Fresh Manager, brief scan, then connect() with a long\n  \
                                    timeout against any NEEWER peripheral found. Tests whether\n  \
                                    CoreBluetooth's queue-on-next-advertisement bridges gaps.\n\n\
                 Run while deckd is failing (`not seen in rescan` in /tmp/deckd.stderr.log)\n\
                 to test against the same OS state."
            );
        }
    }
}

fn parse_duration(args: &[String]) -> Option<Duration> {
    let i = args.iter().position(|a| a == "--duration")?;
    let secs: u64 = args.get(i + 1)?.parse().ok()?;
    Some(Duration::from_secs(secs))
}

async fn experiment_fresh_scan(duration: Duration) {
    println!("=== experiment: fresh-scan duration={:?} ===", duration);
    let t0 = Instant::now();

    let manager = match Manager::new().await {
        Ok(m) => {
            println!("[+{:>6.1?}] Manager::new ok", t0.elapsed());
            m
        }
        Err(e) => {
            println!("[+{:>6.1?}] Manager::new failed: {}", t0.elapsed(), e);
            return;
        }
    };

    let adapter = match manager.adapters().await.map(|v| v.into_iter().next()) {
        Ok(Some(a)) => {
            println!("[+{:>6.1?}] adapter ok", t0.elapsed());
            a
        }
        Ok(None) => {
            println!("[+{:>6.1?}] no adapter", t0.elapsed());
            return;
        }
        Err(e) => {
            println!("[+{:>6.1?}] adapters failed: {}", t0.elapsed(), e);
            return;
        }
    };

    if let Err(e) = adapter.start_scan(ScanFilter::default()).await {
        println!("[+{:>6.1?}] start_scan failed: {}", t0.elapsed(), e);
        return;
    }
    println!("[+{:>6.1?}] start_scan ok, scanning {:?}...", t0.elapsed(), duration);
    tokio::time::sleep(duration).await;
    let _ = adapter.stop_scan().await;
    println!("[+{:>6.1?}] stop_scan", t0.elapsed());

    let peripherals = match adapter.peripherals().await {
        Ok(p) => p,
        Err(e) => {
            println!("[+{:>6.1?}] peripherals() failed: {}", t0.elapsed(), e);
            return;
        }
    };
    println!("[+{:>6.1?}] peripherals(): {} entries", t0.elapsed(), peripherals.len());

    for p in &peripherals {
        let props = p.properties().await.ok().flatten();
        let name = props
            .as_ref()
            .and_then(|pr| pr.local_name.clone())
            .unwrap_or_else(|| "<no name>".into());
        let services = props
            .as_ref()
            .map(|pr| pr.services.len())
            .unwrap_or(0);
        let mfr = props
            .as_ref()
            .map(|pr| {
                pr.manufacturer_data
                    .iter()
                    .map(|(id, data)| format!("{:#06x}={}", id, hex(data)))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "-".into());
        let is_neewer = name.to_uppercase().starts_with("NEEWER");
        println!(
            "  {} id={} name={:?} adv_services={} mfr={}",
            if is_neewer { "*" } else { " " },
            p.id(),
            name,
            services,
            mfr
        );
    }

    let neewer: Vec<_> = futures_filter(&peripherals, |p| async move {
        let n = p
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|pr| pr.local_name)
            .unwrap_or_default();
        n.to_uppercase().starts_with("NEEWER")
    })
    .await;
    println!(
        "[+{:>6.1?}] Found {} NEEWER peripheral(s) in scan results",
        t0.elapsed(),
        neewer.len()
    );
}

async fn experiment_service_filter(duration: Duration) {
    println!("=== experiment: service-filter duration={:?} ===", duration);
    let t0 = Instant::now();

    let manager = Manager::new().await.expect("manager");
    let adapter = manager
        .adapters()
        .await
        .expect("adapters")
        .into_iter()
        .next()
        .expect("adapter");
    println!("[+{:>6.1?}] manager+adapter ok", t0.elapsed());

    let filter = ScanFilter {
        services: vec![NEEWER_SERVICE],
    };
    if let Err(e) = adapter.start_scan(filter).await {
        println!("[+{:>6.1?}] start_scan failed: {}", t0.elapsed(), e);
        return;
    }
    println!(
        "[+{:>6.1?}] start_scan ok with NEEWER_SERVICE filter, scanning {:?}...",
        t0.elapsed(),
        duration
    );
    tokio::time::sleep(duration).await;
    let _ = adapter.stop_scan().await;

    let peripherals = adapter.peripherals().await.unwrap_or_default();
    println!(
        "[+{:>6.1?}] {} peripherals visible after filtered scan",
        t0.elapsed(),
        peripherals.len()
    );
    for p in &peripherals {
        let name = p
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|pr| pr.local_name)
            .unwrap_or_else(|| "<no name>".into());
        println!("  id={} name={:?}", p.id(), name);
    }
}

async fn experiment_connect_and_wait(scan: Duration) {
    println!("=== experiment: connect-and-wait scan={:?} ===", scan);
    let t0 = Instant::now();

    let manager = Manager::new().await.expect("manager");
    let adapter = manager
        .adapters()
        .await
        .expect("adapters")
        .into_iter()
        .next()
        .expect("adapter");
    println!("[+{:>6.1?}] manager+adapter ok", t0.elapsed());

    adapter
        .start_scan(ScanFilter::default())
        .await
        .expect("start_scan");
    println!("[+{:>6.1?}] scanning {:?}...", t0.elapsed(), scan);
    tokio::time::sleep(scan).await;
    let _ = adapter.stop_scan().await;

    let peripherals = adapter.peripherals().await.unwrap_or_default();
    println!("[+{:>6.1?}] {} peripherals", t0.elapsed(), peripherals.len());

    for p in peripherals {
        let name = p
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|pr| pr.local_name)
            .unwrap_or_default();
        if !name.to_uppercase().starts_with("NEEWER") {
            continue;
        }
        println!("[+{:>6.1?}] attempting connect to {} ({})", t0.elapsed(), name, p.id());
        let conn = tokio::time::timeout(Duration::from_secs(30), p.connect()).await;
        match conn {
            Ok(Ok(())) => {
                println!("[+{:>6.1?}] connect ok", t0.elapsed());
                if let Err(e) = p.discover_services().await {
                    println!("[+{:>6.1?}] discover_services failed: {}", t0.elapsed(), e);
                } else {
                    let chars = p.characteristics();
                    let has_write = chars.iter().any(|c| c.uuid == NEEWER_WRITE_CHAR);
                    println!(
                        "[+{:>6.1?}] discover_services ok ({} chars, write_char={})",
                        t0.elapsed(),
                        chars.len(),
                        has_write
                    );
                }
                let _ = p.disconnect().await;
                println!("[+{:>6.1?}] disconnected", t0.elapsed());
            }
            Ok(Err(e)) => println!("[+{:>6.1?}] connect failed: {}", t0.elapsed(), e),
            Err(_) => println!("[+{:>6.1?}] connect timed out after 30s", t0.elapsed()),
        }
    }
}

// ── Native CoreBluetooth path via objc2-core-bluetooth ─────────────
//
// The btleplug-based experiments above all fail when the lights stop
// advertising — the only way to reach them is the API Neewer Control
// Center uses: retrievePeripheralsWithIdentifiers: + connectPeripheral:
// which doesn't depend on scan-time advertisement visibility.

mod native {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
    use objc2_core_bluetooth::{
        CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType,
        CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService,
    };
    use objc2_foundation::{NSArray, NSData, NSError, NSObject, NSObjectProtocol, NSUUID};
    use tokio::sync::oneshot;

    /// Convert an `NSUUID` into a Rust `Uuid` for use as a HashMap key.
    /// Going through bytes avoids relying on NSUUID's Hash/Eq.
    fn nsuuid_to_uuid(ns: &NSUUID) -> Uuid {
        let mut bytes = [0u8; 16];
        unsafe {
            let _: () = msg_send![ns, getUUIDBytes: &mut bytes as *mut [u8; 16] as *mut u8];
        }
        Uuid::from_bytes(bytes)
    }

    fn uuid_to_nsuuid(uuid: Uuid) -> Retained<NSUUID> {
        let bytes = uuid.into_bytes();
        unsafe {
            let alloc = NSUUID::alloc();
            msg_send![alloc, initWithUUIDBytes: bytes.as_ptr()]
        }
    }

    type ConnectResult = Result<(), String>;

    pub struct DelegateState {
        // Fired once when centralManagerDidUpdateState first reports a
        // non-Unknown state. Subsequent state changes (toggle, etc.) are
        // logged but don't refire.
        pub state_ready: Mutex<Option<oneshot::Sender<CBManagerState>>>,
        // Per-peripheral connect-completion channel. Inserted before
        // calling connectPeripheral:options: and removed when either
        // didConnectPeripheral: or didFailToConnectPeripheral:error:
        // fires for that id.
        pub pending_connects: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
        // Per-peripheral service-discovery completion channel.
        pub pending_services: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
        // Per-peripheral characteristic-discovery completion. Keyed by
        // peripheral id; multiple service discoveries on the same
        // peripheral would race, but we only do one at a time per light.
        pub pending_chars: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[ivars = DelegateState]
        pub struct BleDelegate;

        unsafe impl NSObjectProtocol for BleDelegate {}

        unsafe impl CBCentralManagerDelegate for BleDelegate {
            #[unsafe(method(centralManagerDidUpdateState:))]
            fn central_manager_did_update_state(&self, central: &CBCentralManager) {
                let state = unsafe { central.state() };
                println!("  delegate: state -> {:?}", state);
                if let Some(tx) = self.ivars().state_ready.lock().unwrap().take() {
                    let _ = tx.send(state);
                }
            }

            #[unsafe(method(centralManager:didConnectPeripheral:))]
            fn central_did_connect(
                &self,
                _central: &CBCentralManager,
                peripheral: &CBPeripheral,
            ) {
                let id = unsafe { peripheral.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                println!("  delegate: didConnectPeripheral {}", uuid);
                if let Some(tx) = self.ivars().pending_connects.lock().unwrap().remove(&uuid) {
                    let _ = tx.send(Ok(()));
                }
            }

            #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
            fn central_did_fail_to_connect(
                &self,
                _central: &CBCentralManager,
                peripheral: &CBPeripheral,
                error: Option<&NSError>,
            ) {
                let id = unsafe { peripheral.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                let msg = error
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "unknown".into());
                println!("  delegate: didFailToConnectPeripheral {} err={}", uuid, msg);
                if let Some(tx) = self.ivars().pending_connects.lock().unwrap().remove(&uuid) {
                    let _ = tx.send(Err(msg));
                }
            }

            #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
            fn central_did_disconnect(
                &self,
                _central: &CBCentralManager,
                peripheral: &CBPeripheral,
                error: Option<&NSError>,
            ) {
                let id = unsafe { peripheral.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                let msg = error
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "ok".into());
                println!("  delegate: didDisconnectPeripheral {} ({})", uuid, msg);
            }
        }

        unsafe impl CBPeripheralDelegate for BleDelegate {
            #[unsafe(method(peripheral:didDiscoverServices:))]
            fn peripheral_did_discover_services(
                &self,
                peripheral: &CBPeripheral,
                error: Option<&NSError>,
            ) {
                let id = unsafe { peripheral.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                let result = match error {
                    Some(e) => Err(e.localizedDescription().to_string()),
                    None => Ok(()),
                };
                println!("  delegate: didDiscoverServices {} -> {:?}", uuid, result);
                if let Some(tx) = self.ivars().pending_services.lock().unwrap().remove(&uuid) {
                    let _ = tx.send(result);
                }
            }

            #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
            fn peripheral_did_discover_chars(
                &self,
                peripheral: &CBPeripheral,
                _service: &CBService,
                error: Option<&NSError>,
            ) {
                let id = unsafe { peripheral.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                let result = match error {
                    Some(e) => Err(e.localizedDescription().to_string()),
                    None => Ok(()),
                };
                println!("  delegate: didDiscoverChars {} -> {:?}", uuid, result);
                if let Some(tx) = self.ivars().pending_chars.lock().unwrap().remove(&uuid) {
                    let _ = tx.send(result);
                }
            }
        }
    );

    impl BleDelegate {
        pub fn new() -> Retained<Self> {
            let ivars = DelegateState {
                state_ready: Mutex::new(None),
                pending_connects: Mutex::new(HashMap::new()),
                pending_services: Mutex::new(HashMap::new()),
                pending_chars: Mutex::new(HashMap::new()),
            };
            let alloc = Self::alloc().set_ivars(ivars);
            unsafe { msg_send![super(alloc), init] }
        }
    }

    /// Owns the CBCentralManager and the delegate. Starts a serial
    /// dispatch queue dedicated to BLE callbacks so they don't depend on
    /// any NSRunLoop existing on the calling thread.
    pub struct BleManager {
        pub central: Retained<CBCentralManager>,
        pub delegate: Retained<BleDelegate>,
    }

    impl BleManager {
        /// Create a CBCentralManager and wait for it to reach poweredOn
        /// (or fail with whatever non-poweredOn state it lands in).
        pub async fn new() -> Result<Self, String> {
            let delegate = BleDelegate::new();
            let (tx, rx) = oneshot::channel();
            *delegate.ivars().state_ready.lock().unwrap() = Some(tx);

            // Serial dispatch queue for delegate callbacks. dispatch2 wraps
            // libdispatch; we hand the underlying handle to CBCentralManager.
            let queue = dispatch2::DispatchQueue::new(
                "com.deckd.ble_probe",
                dispatch2::DispatchQueueAttr::SERIAL,
            );

            let central: Retained<CBCentralManager> = unsafe {
                let proto: &ProtocolObject<dyn CBCentralManagerDelegate> =
                    ProtocolObject::from_ref(&*delegate);
                let alloc = CBCentralManager::alloc();
                msg_send![
                    alloc,
                    initWithDelegate: proto,
                    queue: &*queue,
                ]
            };

            let state = tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .map_err(|_| "timed out waiting for state callback".to_string())?
                .map_err(|_| "state channel dropped".to_string())?;

            if state != CBManagerState::PoweredOn {
                return Err(format!("central state {:?}, not PoweredOn", state));
            }

            Ok(Self { central, delegate })
        }

        /// Look up known peripherals by UUID without scanning. Mirrors
        /// `[CBCentralManager retrievePeripheralsWithIdentifiers:]`.
        pub fn retrieve(&self, uuids: &[Uuid]) -> Vec<Retained<CBPeripheral>> {
            let nsuuids: Vec<Retained<NSUUID>> = uuids.iter().copied().map(uuid_to_nsuuid).collect();
            let nsuuid_refs: Vec<&NSUUID> = nsuuids.iter().map(|u| &**u).collect();
            let array = NSArray::from_slice(&nsuuid_refs);
            let result: Retained<NSArray<CBPeripheral>> =
                unsafe { self.central.retrievePeripheralsWithIdentifiers(&array) };
            result.iter().collect()
        }

        /// Issue connectPeripheral: and wait for the corresponding
        /// didConnectPeripheral or didFailToConnect callback. CoreBluetooth
        /// queues this until the peripheral next becomes connectable;
        /// timeout caps how long we'll wait.
        pub async fn connect(
            &self,
            peripheral: &CBPeripheral,
            timeout: Duration,
        ) -> Result<(), String> {
            let id = unsafe { peripheral.identifier() };
            let uuid = nsuuid_to_uuid(&id);
            let (tx, rx) = oneshot::channel();
            self.delegate
                .ivars()
                .pending_connects
                .lock()
                .unwrap()
                .insert(uuid, tx);
            unsafe {
                self.central.connectPeripheral_options(peripheral, None);
            }
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err("connect channel dropped".into()),
                Err(_) => {
                    // Cancel the pending connect on timeout.
                    self.delegate
                        .ivars()
                        .pending_connects
                        .lock()
                        .unwrap()
                        .remove(&uuid);
                    unsafe { self.central.cancelPeripheralConnection(peripheral) };
                    Err(format!("connect timed out after {:?}", timeout))
                }
            }
        }

        pub fn disconnect(&self, peripheral: &CBPeripheral) {
            unsafe { self.central.cancelPeripheralConnection(peripheral) };
        }

        /// Run service discovery and (per service we care about) characteristic
        /// discovery. Returns once both rounds have fired delegate callbacks.
        pub async fn discover_services(
            &self,
            peripheral: &CBPeripheral,
            timeout: Duration,
        ) -> Result<(), String> {
            // The peripheral needs a delegate set so the CBPeripheralDelegate
            // callbacks land somewhere. We reuse the central's delegate.
            let proto: &ProtocolObject<dyn CBPeripheralDelegate> =
                ProtocolObject::from_ref(&*self.delegate);
            unsafe { peripheral.setDelegate(Some(proto)) };

            let id = unsafe { peripheral.identifier() };
            let uuid = nsuuid_to_uuid(&id);

            let (tx, rx) = oneshot::channel();
            self.delegate
                .ivars()
                .pending_services
                .lock()
                .unwrap()
                .insert(uuid, tx);
            unsafe { peripheral.discoverServices(None) };
            tokio::time::timeout(timeout, rx)
                .await
                .map_err(|_| format!("discoverServices timed out after {:?}", timeout))?
                .map_err(|_| "service channel dropped".to_string())??;

            // Discover characteristics on each service. The probe only cares
            // about the Neewer service; if it's missing we report it.
            let services_opt: Option<Retained<NSArray<CBService>>> =
                unsafe { peripheral.services() };
            let services = services_opt.ok_or("no services after discovery")?;
            for service in services.iter() {
                let (tx, rx) = oneshot::channel();
                self.delegate
                    .ivars()
                    .pending_chars
                    .lock()
                    .unwrap()
                    .insert(uuid, tx);
                unsafe { peripheral.discoverCharacteristics_forService(None, &service) };
                tokio::time::timeout(timeout, rx)
                    .await
                    .map_err(|_| format!("discoverCharacteristics timed out after {:?}", timeout))?
                    .map_err(|_| "char channel dropped".to_string())??;
            }
            Ok(())
        }

        /// Find a characteristic by UUID across all discovered services and
        /// write the given bytes. Uses WithoutResponse for compat with the
        /// Neewer write characteristic which doesn't ack.
        pub fn write(
            &self,
            peripheral: &CBPeripheral,
            char_uuid: Uuid,
            data: &[u8],
        ) -> Result<(), String> {
            let services_opt: Option<Retained<NSArray<CBService>>> =
                unsafe { peripheral.services() };
            let services = services_opt.ok_or("no services discovered")?;
            for service in services.iter() {
                let chars_opt: Option<Retained<NSArray<CBCharacteristic>>> =
                    unsafe { service.characteristics() };
                let Some(chars) = chars_opt else { continue };
                for c in chars.iter() {
                    let cb_uuid = unsafe { c.UUID() };
                    let s = unsafe { cb_uuid.UUIDString() }.to_string();
                    if uuid_str_eq(&s, char_uuid) {
                        let nsdata = NSData::from_vec(data.to_vec());
                        unsafe {
                            peripheral.writeValue_forCharacteristic_type(
                                &nsdata,
                                &c,
                                CBCharacteristicWriteType::WithoutResponse,
                            );
                        }
                        return Ok(());
                    }
                }
            }
            Err(format!("characteristic {} not found", char_uuid))
        }
    }

    /// CBUUID prints in dashed-hex; compare lowercase to a Rust Uuid.
    fn uuid_str_eq(s: &str, target: Uuid) -> bool {
        let target_str = target.hyphenated().to_string();
        // CBUUIDs may be 16-bit short forms; compare suffix.
        let s = s.to_lowercase();
        s == target_str || target_str.ends_with(&s)
    }
}

async fn experiment_retrieve(uuid_strs: &[&str]) {
    println!("=== experiment: native retrieve ===");
    let t0 = Instant::now();

    let uuids: Vec<Uuid> = match uuid_strs.iter().map(|s| Uuid::parse_str(s)).collect() {
        Ok(v) => v,
        Err(e) => {
            println!("invalid uuid: {}", e);
            return;
        }
    };

    let mgr = match native::BleManager::new().await {
        Ok(m) => {
            println!("[+{:>6.1?}] BleManager::new ok", t0.elapsed());
            m
        }
        Err(e) => {
            println!("[+{:>6.1?}] BleManager::new failed: {}", t0.elapsed(), e);
            return;
        }
    };

    let peripherals = mgr.retrieve(&uuids);
    println!(
        "[+{:>6.1?}] retrievePeripheralsWithIdentifiers: returned {} peripheral(s)",
        t0.elapsed(),
        peripherals.len()
    );
    for p in &peripherals {
        let id = unsafe { p.identifier() };
        let name = unsafe { p.name() }
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<no name>".into());
        let state = unsafe { p.state() };
        println!("  id={:?} name={:?} state={:?}", id, name, state);
    }
}

async fn experiment_native_connect(uuid_strs: &[&str]) {
    println!("=== experiment: native retrieve + connect ===");
    let t0 = Instant::now();

    let uuids: Vec<Uuid> = match uuid_strs.iter().map(|s| Uuid::parse_str(s)).collect() {
        Ok(v) => v,
        Err(e) => {
            println!("invalid uuid: {}", e);
            return;
        }
    };

    let mgr = match native::BleManager::new().await {
        Ok(m) => {
            println!("[+{:>6.1?}] BleManager::new ok", t0.elapsed());
            m
        }
        Err(e) => {
            println!("[+{:>6.1?}] BleManager::new failed: {}", t0.elapsed(), e);
            return;
        }
    };

    let peripherals = mgr.retrieve(&uuids);
    println!(
        "[+{:>6.1?}] retrieve returned {} peripheral(s)",
        t0.elapsed(),
        peripherals.len()
    );

    for p in &peripherals {
        let id = unsafe { p.identifier() };
        println!("[+{:>6.1?}] connecting to {:?}...", t0.elapsed(), id);
        match mgr.connect(p, Duration::from_secs(60)).await {
            Ok(()) => {
                println!("[+{:>6.1?}] connected. disconnecting.", t0.elapsed());
                mgr.disconnect(p);
            }
            Err(e) => {
                println!("[+{:>6.1?}] connect failed: {}", t0.elapsed(), e);
            }
        }
    }
}

async fn experiment_native_toggle(uuid_strs: &[&str]) {
    println!("=== experiment: native retrieve + connect + toggle power ===");
    let t0 = Instant::now();

    let uuids: Vec<Uuid> = match uuid_strs.iter().map(|s| Uuid::parse_str(s)).collect() {
        Ok(v) => v,
        Err(e) => {
            println!("invalid uuid: {}", e);
            return;
        }
    };

    let mgr = match native::BleManager::new().await {
        Ok(m) => {
            println!("[+{:>6.1?}] BleManager::new ok", t0.elapsed());
            m
        }
        Err(e) => {
            println!("[+{:>6.1?}] BleManager::new failed: {}", t0.elapsed(), e);
            return;
        }
    };

    let peripherals = mgr.retrieve(&uuids);
    println!(
        "[+{:>6.1?}] retrieve returned {} peripheral(s)",
        t0.elapsed(),
        peripherals.len()
    );

    // BLE power command from src/lights.rs cmd_power(): the GL1 PROs talk
    // BLE on the deckd actor side, so we use the BLE-format bytes, NOT the
    // UDP-format gl1_cmd_power. Payload = [CMD_PREFIX, TAG_POWER, len, state],
    // checksum = sum of payload bytes mod 256, appended.
    // CMD_PREFIX=0x78, TAG_POWER=0x81. state: 0x01=on, 0x02=off.
    let cksum = |b: &[u8]| -> u8 { b.iter().fold(0u16, |a, x| a + *x as u16) as u8 };
    let mut power_off_buf = vec![0x78u8, 0x81, 0x01, 0x02];
    power_off_buf.push(cksum(&power_off_buf));
    let mut power_on_buf = vec![0x78u8, 0x81, 0x01, 0x01];
    power_on_buf.push(cksum(&power_on_buf));
    let power_off = power_off_buf.clone();
    let power_on = power_on_buf.clone();
    println!("  bytes: power_off={} power_on={}", hex(&power_off), hex(&power_on));

    for p in &peripherals {
        let id = unsafe { p.identifier() };
        println!("[+{:>6.1?}] connecting to {:?}", t0.elapsed(), id);
        if let Err(e) = mgr.connect(p, Duration::from_secs(60)).await {
            println!("[+{:>6.1?}] connect failed: {}", t0.elapsed(), e);
            continue;
        }
        println!("[+{:>6.1?}] connected. discovering services...", t0.elapsed());
        if let Err(e) = mgr.discover_services(p, Duration::from_secs(8)).await {
            println!("[+{:>6.1?}] discover failed: {}", t0.elapsed(), e);
            mgr.disconnect(p);
            continue;
        }
        println!(
            "[+{:>6.1?}] services discovered. writing power_off then power_on (1s apart)...",
            t0.elapsed()
        );
        if let Err(e) = mgr.write(p, NEEWER_WRITE_CHAR, &power_off) {
            println!("[+{:>6.1?}] write power_off failed: {}", t0.elapsed(), e);
        }
        println!("[+{:>6.1?}] sent power_off — light should be DARK for 4s", t0.elapsed());
        tokio::time::sleep(Duration::from_secs(4)).await;
        if let Err(e) = mgr.write(p, NEEWER_WRITE_CHAR, &power_on) {
            println!("[+{:>6.1?}] write power_on failed: {}", t0.elapsed(), e);
        }
        println!("[+{:>6.1?}] sent power_on", t0.elapsed());
        tokio::time::sleep(Duration::from_secs(1)).await;
        println!("[+{:>6.1?}] toggled. disconnecting.", t0.elapsed());
        mgr.disconnect(p);
    }
}

fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

async fn futures_filter<F, Fut>(items: &[Peripheral], mut f: F) -> Vec<Peripheral>
where
    F: FnMut(Peripheral) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let mut out = Vec::new();
    for p in items {
        if f(p.clone()).await {
            out.push(p.clone());
        }
    }
    out
}
