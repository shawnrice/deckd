//! USB hotplug watcher for the Stream Deck. Spawns a background thread that
//! uses libusb (via rusb) to receive kernel-driven attach/detach events for
//! the deck's VID:PID, and forwards them to the main loop on an mpsc channel.
//!
//! This replaces the old "blindly reconnect on read failure" behavior at
//! startup: deckd can stay alive while the deck is unplugged and react
//! immediately when it comes back, without polling.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use elgato_streamdeck::info::{ELGATO_VENDOR_ID, PID_STREAMDECK_PLUS};
use log::{info, warn};
use rusb::{Device, GlobalContext, Hotplug, HotplugBuilder, UsbContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckEvent {
    Attached,
    Detached,
}

struct Watcher {
    tx: Sender<DeckEvent>,
}

impl Hotplug<GlobalContext> for Watcher {
    fn device_arrived(&mut self, _device: Device<GlobalContext>) {
        info!("USB hotplug: Stream Deck attached");
        let _ = self.tx.send(DeckEvent::Attached);
    }

    fn device_left(&mut self, _device: Device<GlobalContext>) {
        info!("USB hotplug: Stream Deck detached");
        let _ = self.tx.send(DeckEvent::Detached);
    }
}

/// Start watching for Stream Deck Plus USB attach/detach events.
///
/// Returns a receiver that yields `Attached` for each device currently on the
/// bus (because we register with `enumerate(true)`) and for every later
/// plug-in, plus `Detached` for each unplug. If libusb hotplug isn't supported
/// on this platform, returns `Err` and the caller should fall back to polling.
pub fn start() -> Result<Receiver<DeckEvent>, String> {
    if !rusb::has_hotplug() {
        return Err("libusb hotplug unsupported on this platform".into());
    }

    let (tx, rx) = channel();

    thread::Builder::new()
        .name("usb-hotplug".into())
        .spawn(move || {
            let ctx = GlobalContext::default();
            let registration = HotplugBuilder::new()
                .vendor_id(ELGATO_VENDOR_ID)
                .product_id(PID_STREAMDECK_PLUS)
                .enumerate(true)
                .register(ctx, Box::new(Watcher { tx }));

            let _reg = match registration {
                Ok(r) => r,
                Err(e) => {
                    warn!("USB hotplug registration failed: {}", e);
                    return;
                }
            };

            // Pump libusb events forever. handle_events with a timeout
            // (instead of None) lets the thread cleanly observe shutdown if
            // we ever need it; the timeout itself costs nothing.
            loop {
                if let Err(e) = ctx.handle_events(Some(Duration::from_secs(1))) {
                    warn!("USB hotplug handle_events error: {}", e);
                    thread::sleep(Duration::from_secs(1));
                }
            }
        })
        .map_err(|e| format!("failed to spawn usb-hotplug thread: {}", e))?;

    Ok(rx)
}
