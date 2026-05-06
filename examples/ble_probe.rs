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
