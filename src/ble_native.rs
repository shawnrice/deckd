// Native CoreBluetooth path via objc2-core-bluetooth.
//
// We bypass btleplug for these lights because btleplug 0.12 doesn't expose
// `retrievePeripheralsWithIdentifiers:` — the API Neewer Control Center
// uses to reach lights without depending on advertisement scan visibility.
// See examples/ble_probe.rs for the validation history.
//
// Architecture:
// - One shared BleManager per process (Arc'd). Owns the CBCentralManager
//   and a single delegate that fans events to per-peripheral channels.
// - Auto-reconnect lives in the delegate: when didDisconnectPeripheral
//   fires, we immediately call connect() again. CoreBluetooth queues that
//   request indefinitely, so we don't have to scan or back off — the link
//   re-establishes the moment the peripheral becomes connectable.
// - Writes are fire-and-forget against CBCharacteristicWriteType::WithoutResponse,
//   matching how the Neewer GATT characteristic operates. If the peripheral
//   isn't connected, CoreBluetooth drops the write silently — callers should
//   treat writes as best-effort and rely on the heartbeat to resync state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, info, warn};
use uuid::Uuid;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType,
    CBManagerState, CBPeripheral, CBPeripheralDelegate, CBPeripheralState, CBService,
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
    /// Fired when centralManagerDidUpdateState first reports a non-Unknown
    /// state. Subsequent state changes log only.
    state_ready: Mutex<Option<oneshot::Sender<CBManagerState>>>,
    /// Per-peripheral connect-completion. Inserted before connectPeripheral:
    /// and removed when didConnectPeripheral or didFailToConnectPeripheral
    /// fires. A peripheral being auto-reconnected after disconnect has no
    /// entry — the reconnect happens in the delegate without an awaiter.
    pending_connects: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
    pending_services: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
    pending_chars: Mutex<HashMap<Uuid, oneshot::Sender<ConnectResult>>>,
    /// Held weak so the delegate can call back into the manager (specifically
    /// to issue connect() on disconnect events) without an Arc cycle. Set
    /// after BleManager construction completes.
    central: Mutex<Option<Retained<CBCentralManager>>>,
    /// Names known when the peripheral was first scanned, keyed by id. Used
    /// for log clarity since CBPeripheral.name() returns nil while
    /// disconnected on macOS.
    names: Mutex<HashMap<Uuid, String>>,
    /// Set by `scan()` while a scan is in progress. didDiscoverPeripheral
    /// callbacks accumulate into this map, keyed by peripheral id. Cleared
    /// after each scan call collects results.
    scan_buf: Mutex<Option<HashMap<Uuid, ScannedAdvertisement>>>,
    /// Characteristic UUIDs to enable notifications on automatically when
    /// they appear via didDiscoverCharacteristics. Lets the manager handle
    /// "subscribe to NEEWER_NOTIFY_CHAR after every reconnect" without the
    /// caller having to coordinate with discovery timing.
    auto_subscribe: Mutex<Vec<Uuid>>,
    /// Latest notification bytes per peripheral, keyed by id. Updated on
    /// every didUpdateValueForCharacteristic. Callers use
    /// `BleManager::last_notify(peripheral)` to read the most recent.
    last_notify: Mutex<HashMap<Uuid, Vec<u8>>>,
}

#[derive(Clone)]
pub struct ScannedAdvertisement {
    pub uuid: Uuid,
    pub name: Option<String>,
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
            info!("BLE: central state -> {:?}", state);
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
            let name = self.ivars().names.lock().unwrap().get(&uuid).cloned()
                .unwrap_or_else(|| uuid.to_string());
            info!("BLE: connected {}", name);
            // The peripheral needs its delegate set so service/char
            // discovery callbacks land here. Setting it on every connect
            // is idempotent and survives reconnect cycles.
            let proto: &ProtocolObject<dyn CBPeripheralDelegate> =
                ProtocolObject::from_ref(self);
            unsafe { peripheral.setDelegate(Some(proto)) };
            // Kick off service discovery so writes have a characteristic
            // to target. didDiscoverServices fires the per-peripheral
            // pending_services oneshot if any caller is awaiting; reconnects
            // discover services again but with no awaiter (services may
            // change across sessions per CoreBluetooth contract).
            unsafe { peripheral.discoverServices(None) };
            if let Some(tx) = self.ivars().pending_connects.lock().unwrap().remove(&uuid) {
                let _ = tx.send(Ok(()));
            }
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn central_did_discover(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            _adv: &objc2_foundation::NSDictionary,
            _rssi: &objc2_foundation::NSNumber,
        ) {
            let id = unsafe { peripheral.identifier() };
            let uuid = nsuuid_to_uuid(&id);
            let name = unsafe { peripheral.name() }.map(|s| s.to_string());
            let mut buf = self.ivars().scan_buf.lock().unwrap();
            if let Some(map) = buf.as_mut() {
                map.entry(uuid).or_insert(ScannedAdvertisement {
                    uuid,
                    name: name.clone(),
                });
            }
            // Cache the name for future log lines, since CBPeripheral.name()
            // can return nil for disconnected peripherals later.
            if let Some(n) = name {
                self.ivars().names.lock().unwrap().insert(uuid, n);
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
            let msg = error.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "unknown".into());
            warn!("BLE: didFailToConnectPeripheral {} err={}", uuid, msg);
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
            let name = self.ivars().names.lock().unwrap().get(&uuid).cloned()
                .unwrap_or_else(|| uuid.to_string());
            let msg = error.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "ok".into());
            info!("BLE: disconnected {} ({}) — re-issuing connect", name, msg);
            // Auto-reconnect: queue another connect immediately. CoreBluetooth
            // holds this until the peripheral becomes connectable again, so
            // we don't need a scan, backoff, or retry loop. This is the
            // pattern keefo/NeewerLite uses and what the Neewer Control
            // Center binary appears to do.
            if let Some(central) = self.ivars().central.lock().unwrap().as_ref() {
                unsafe { central.connectPeripheral_options(peripheral, None) };
            }
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
            debug!("BLE: didDiscoverServices {} -> {:?}", uuid, result);
            // After services come back, kick characteristic discovery on
            // each so writes have a target. We don't await this here; the
            // discovery is fire-and-forget for the auto-reconnect path.
            if result.is_ok() {
                let services_opt: Option<Retained<NSArray<CBService>>> = unsafe { peripheral.services() };
                if let Some(services) = services_opt {
                    for service in services.iter() {
                        unsafe { peripheral.discoverCharacteristics_forService(None, &service) };
                    }
                }
            }
            if let Some(tx) = self.ivars().pending_services.lock().unwrap().remove(&uuid) {
                let _ = tx.send(result);
            }
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn peripheral_did_discover_chars(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            error: Option<&NSError>,
        ) {
            let id = unsafe { peripheral.identifier() };
            let uuid = nsuuid_to_uuid(&id);
            let result = match error {
                Some(e) => Err(e.localizedDescription().to_string()),
                None => Ok(()),
            };
            debug!("BLE: didDiscoverChars {} -> {:?}", uuid, result);
            // Auto-subscribe to any characteristic in our watch list.
            // Lets writers (deckd::lights) ask for notifications once at
            // boot and have them re-attached on every reconnect without
            // additional plumbing.
            if result.is_ok() {
                let watched: Vec<Uuid> = self.ivars().auto_subscribe.lock().unwrap().clone();
                if !watched.is_empty() {
                    if let Some(chars) = unsafe { service.characteristics() } {
                        for c in chars.iter() {
                            let cb_uuid = unsafe { c.UUID() };
                            let s = unsafe { cb_uuid.UUIDString() }.to_string();
                            for target in &watched {
                                if uuid_str_eq(&s, *target) {
                                    info!("BLE: subscribing notify on {} for {}", target, uuid);
                                    unsafe {
                                        peripheral.setNotifyValue_forCharacteristic(true, &c)
                                    };
                                }
                            }
                        }
                    }
                }
            }
            if let Some(tx) = self.ivars().pending_chars.lock().unwrap().remove(&uuid) {
                let _ = tx.send(result);
            }
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn peripheral_did_update_value(
            &self,
            peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            let id = unsafe { peripheral.identifier() };
            let uuid = nsuuid_to_uuid(&id);
            if let Some(e) = error {
                warn!("BLE: notify error for {}: {}", uuid, e.localizedDescription());
                return;
            }
            let value_opt = unsafe { characteristic.value() };
            let Some(value) = value_opt else { return };
            // NSData → Vec<u8>. The bytes API on objc2-foundation's NSData
            // returns &[u8] tied to the NSData's lifetime; copy out.
            let bytes: Vec<u8> = value.to_vec();
            debug!("BLE: notify from {}: {:02x?}", uuid, bytes);
            self.ivars()
                .last_notify
                .lock()
                .unwrap()
                .insert(uuid, bytes);
        }
    }
);

impl BleDelegate {
    fn new() -> Retained<Self> {
        let ivars = DelegateState {
            state_ready: Mutex::new(None),
            pending_connects: Mutex::new(HashMap::new()),
            pending_services: Mutex::new(HashMap::new()),
            pending_chars: Mutex::new(HashMap::new()),
            central: Mutex::new(None),
            names: Mutex::new(HashMap::new()),
            scan_buf: Mutex::new(None),
            auto_subscribe: Mutex::new(Vec::new()),
            last_notify: Mutex::new(HashMap::new()),
        };
        let alloc = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(alloc), init] }
    }
}

/// One per process. Holds the CBCentralManager and delegate; lives until the
/// daemon exits. Cloning gives shared access (Arc internally).
pub struct BleManager {
    central: Retained<CBCentralManager>,
    delegate: Retained<BleDelegate>,
    // Keep the dispatch queue alive for the lifetime of the manager so the
    // CBCentralManager's callbacks have a place to land.
    _queue: dispatch2::DispatchRetained<dispatch2::DispatchQueue>,
}

// CBCentralManager / CBPeripheral are Apple's reference types — internally
// thread-safe for the operations we do (delegate callbacks happen on our
// serial dispatch queue, message sends from arbitrary Rust threads are fine).
unsafe impl Send for BleManager {}
unsafe impl Sync for BleManager {}

impl BleManager {
    /// Create the central, wait for it to reach poweredOn, return.
    /// Errors if the central lands in any non-poweredOn state.
    pub async fn new() -> Result<Arc<Self>, String> {
        let delegate = BleDelegate::new();
        let (tx, rx) = oneshot::channel();
        *delegate.ivars().state_ready.lock().unwrap() = Some(tx);

        let queue = dispatch2::DispatchQueue::new(
            "com.deckd.ble",
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

        // Park the central for the delegate's auto-reconnect path before we
        // wait for state — if poweredOn fires synchronously the delegate
        // shouldn't see an empty central slot.
        *delegate.ivars().central.lock().unwrap() = Some(central.clone());

        let state = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .map_err(|_| "timed out waiting for state callback".to_string())?
            .map_err(|_| "state channel dropped".to_string())?;

        if state != CBManagerState::PoweredOn {
            return Err(format!("central state {:?}, not PoweredOn", state));
        }

        Ok(Arc::new(Self {
            central,
            delegate,
            _queue: queue,
        }))
    }

    /// Look up known peripherals by UUID without scanning. Mirrors
    /// `[CBCentralManager retrievePeripheralsWithIdentifiers:]`.
    /// Returns whatever subset CoreBluetooth recognises — UUIDs not known
    /// to the system bluetoothd (e.g. a paired device that was unpaired)
    /// are silently dropped from the result.
    pub fn retrieve(&self, uuids: &[Uuid]) -> Vec<BlePeripheral> {
        let nsuuids: Vec<Retained<NSUUID>> = uuids.iter().copied().map(uuid_to_nsuuid).collect();
        let nsuuid_refs: Vec<&NSUUID> = nsuuids.iter().map(|u| &**u).collect();
        let array = NSArray::from_slice(&nsuuid_refs);
        let result: Retained<NSArray<CBPeripheral>> =
            unsafe { self.central.retrievePeripheralsWithIdentifiers(&array) };
        result
            .iter()
            .map(|p| {
                let id = unsafe { p.identifier() };
                let uuid = nsuuid_to_uuid(&id);
                let name = unsafe { p.name() }
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| uuid.to_string());
                self.delegate
                    .ivars()
                    .names
                    .lock()
                    .unwrap()
                    .insert(uuid, name.clone());
                BlePeripheral {
                    uuid,
                    name,
                    cb: p,
                }
            })
            .collect()
    }

    /// Issue connect and wait for the corresponding didConnectPeripheral or
    /// didFailToConnect. Internally CoreBluetooth queues a request that
    /// fires the moment the peripheral is connectable; `timeout` caps how
    /// long we'll block before giving up and cancelling the queued request.
    pub async fn connect(
        &self,
        peripheral: &BlePeripheral,
        timeout: Duration,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.delegate
            .ivars()
            .pending_connects
            .lock()
            .unwrap()
            .insert(peripheral.uuid, tx);
        unsafe {
            self.central.connectPeripheral_options(&peripheral.cb, None);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("connect channel dropped".into()),
            Err(_) => {
                self.delegate
                    .ivars()
                    .pending_connects
                    .lock()
                    .unwrap()
                    .remove(&peripheral.uuid);
                unsafe { self.central.cancelPeripheralConnection(&peripheral.cb) };
                Err(format!("connect timed out after {:?}", timeout))
            }
        }
    }

    /// Scan for advertising peripherals for `duration`, return what was
    /// seen. Used at first-time discovery to learn peripheral UUIDs that
    /// will be persisted and looked up via `retrieve()` on every subsequent
    /// boot. Once we have UUIDs we never need to scan again.
    pub async fn scan(&self, duration: Duration) -> Vec<ScannedAdvertisement> {
        // Initialize the buffer the delegate will populate.
        *self.delegate.ivars().scan_buf.lock().unwrap() = Some(HashMap::new());
        // Default ScanFilter (no filter) so we see all peripherals; we
        // filter for NEEWER on the receiving side.
        unsafe { self.central.scanForPeripheralsWithServices_options(None, None) };
        tokio::time::sleep(duration).await;
        unsafe { self.central.stopScan() };
        let buf = self.delegate.ivars().scan_buf.lock().unwrap().take();
        buf.map(|m| m.into_values().collect()).unwrap_or_default()
    }

    /// Register a characteristic UUID we want notifications for. The
    /// delegate auto-subscribes on every didDiscoverCharacteristics, so a
    /// reconnect doesn't require the caller to re-issue this — set it once
    /// after BleManager::new() and the manager keeps the subscription
    /// attached for the lifetime of the process.
    pub fn watch_characteristic(&self, char_uuid: Uuid) {
        self.delegate
            .ivars()
            .auto_subscribe
            .lock()
            .unwrap()
            .push(char_uuid);
    }

    /// Latest bytes received via notification on any watched characteristic
    /// for `peripheral`, or None if nothing has been received yet (or the
    /// peripheral has never been connected). Per the NEEWER protocol the
    /// caller decodes:
    ///   data[0] == 0x01 → channel/mode status, data[3] is current channel
    ///   data[0] == 0x02 → power status, data[3]==1 ON, data[3]==2 STANDBY
    pub fn last_notify(&self, peripheral: &BlePeripheral) -> Option<Vec<u8>> {
        self.delegate
            .ivars()
            .last_notify
            .lock()
            .unwrap()
            .get(&peripheral.uuid)
            .cloned()
    }

    /// Synchronous, fire-and-forget connect kick. Used by the rescan path
    /// where the user has explicitly asked us to reconnect — we don't await
    /// the result; the delegate's didConnect / didDisconnect callbacks
    /// drive the actual state transition.
    pub fn kick_reconnect(&self, peripheral: &BlePeripheral) -> Result<(), String> {
        unsafe {
            self.central.connectPeripheral_options(&peripheral.cb, None);
        }
        Ok(())
    }

    pub fn is_connected(&self, peripheral: &BlePeripheral) -> bool {
        let s = unsafe { peripheral.cb.state() };
        s == CBPeripheralState::Connected
    }

    /// Find a characteristic by UUID across all discovered services and
    /// write `data` to it without response. Returns Err if the peripheral
    /// is disconnected or services haven't been discovered yet — the caller
    /// is expected to drop the write and rely on the heartbeat to resync.
    pub fn write(
        &self,
        peripheral: &BlePeripheral,
        char_uuid: Uuid,
        data: &[u8],
    ) -> Result<(), String> {
        if !self.is_connected(peripheral) {
            return Err("not connected".into());
        }
        let services_opt: Option<Retained<NSArray<CBService>>> =
            unsafe { peripheral.cb.services() };
        let services = services_opt.ok_or("services not discovered yet")?;
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
                        peripheral.cb.writeValue_forCharacteristic_type(
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

/// Owned reference to a CBPeripheral plus our cached display name. Lives in
/// a Light's transport variant. Cloning is cheap (Retained<CBPeripheral>
/// is reference-counted) so we hand these around freely.
pub struct BlePeripheral {
    pub uuid: Uuid,
    pub name: String,
    cb: Retained<CBPeripheral>,
}

unsafe impl Send for BlePeripheral {}
unsafe impl Sync for BlePeripheral {}

impl Clone for BlePeripheral {
    fn clone(&self) -> Self {
        Self {
            uuid: self.uuid,
            name: self.name.clone(),
            cb: self.cb.clone(),
        }
    }
}

/// CBUUID prints in dashed-hex; compare lowercase to a Rust Uuid. Some
/// characteristics report 16-bit short UUIDs that match the suffix of the
/// canonical 128-bit form, so we accept either.
fn uuid_str_eq(s: &str, target: Uuid) -> bool {
    let target_str = target.hyphenated().to_string();
    let s = s.to_lowercase();
    s == target_str || target_str.ends_with(&s)
}
