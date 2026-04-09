mod actions;
mod audio;
mod boot;
mod camera;
mod config;
mod dashboard;
mod deck;
mod lights;
mod gcal;
mod notify;
mod render;
mod soundboard;
mod sysmon;
mod tamagotchi;
mod uvc;
mod timer;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use log::{error, info, warn};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Subcommands — send to running daemon and exit
    if args.len() >= 2 {
        match args[1].as_str() {
            "notify" => {
                let msg = args[2..].join(" ");
                if msg.is_empty() {
                    eprintln!("Usage: deckd notify <message>");
                    std::process::exit(1);
                }
                send_udp(&msg);
                return;
            }
            "timer" => {
                let cmd = args.get(2).map(|s| s.as_str()).unwrap_or("toggle");
                // Timer commands go through notify with a special prefix
                send_udp(&format!("__timer:{}", cmd));
                return;
            }
            "sound" | "sounds" => {
                let name = args.get(2).map(|s| s.as_str()).unwrap_or("");
                if name.is_empty() || name == "list" || args[1] == "sounds" {
                    soundboard::list_sounds();
                    return;
                }
                soundboard::play_named_sync(name);
                return;
            }
            "cameras" => {
                setup_camera();
                return;
            }
            "feed" => {
                send_udp("__pet:feed");
                println!("Fed the pet!");
                return;
            }
            "auth" => {
                let service = args.get(2).map(|s| s.as_str()).unwrap_or("");
                match service {
                    "google" => gcal::authorize(),
                    _ => {
                        eprintln!("Usage: deckd auth google");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "reload" => {
                send_udp("__reload");
                println!("Reload signal sent.");
                return;
            }
            "start" => {
                let plist = plist_path();
                if !plist.exists() {
                    eprintln!("Service not installed. Run `deckd install` first.");
                    std::process::exit(1);
                }
                std::process::Command::new("launchctl")
                    .args(["start", LABEL])
                    .status()
                    .ok();
                println!("Started.");
                return;
            }
            "stop" => {
                // Unload to fully stop (KeepAlive would restart on plain stop)
                let plist = plist_path();
                if plist.exists() {
                    std::process::Command::new("launchctl")
                        .args(["unload", plist.to_str().unwrap()])
                        .status()
                        .ok();
                }
                // Also kill any running instance
                std::process::Command::new("pkill")
                    .args(["-f", "deckd"])
                    .status()
                    .ok();
                println!("Stopped.");
                return;
            }
            "dev" => {
                // Stop the service, run debug build, restart service on exit
                let plist = plist_path();
                let had_service = plist.exists();
                if had_service {
                    println!("Stopping service...");
                    std::process::Command::new("launchctl")
                        .args(["unload", plist.to_str().unwrap()])
                        .status()
                        .ok();
                    std::process::Command::new("pkill")
                        .args(["-f", "target/release/deckd"])
                        .status()
                        .ok();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                println!("Running in dev mode... (Ctrl-C to stop)");
                start_daemon();
                if had_service {
                    println!("Restarting service...");
                    std::process::Command::new("launchctl")
                        .args(["load", plist.to_str().unwrap()])
                        .status()
                        .ok();
                }
                return;
            }
            "install" => {
                install_service();
                return;
            }
            "uninstall" => {
                uninstall_service();
                return;
            }
            "help" | "--help" | "-h" => {
                println!("deckd — Stream Deck+ daemon\n");
                println!("Usage:");
                println!("  deckd                    Start the daemon");
                println!("  deckd install            Install as launchd service (auto-start)");
                println!("  deckd uninstall          Remove launchd service");
                println!("  deckd start              Start the launchd service");
                println!("  deckd stop               Stop the service and kill deckd");
                println!("  deckd dev                Stop service, run debug build, restart on exit");
                println!("  deckd notify <message>   Show notification on LCD");
                println!("  deckd timer [toggle|start_25|start_5|start_10|stop]");
                println!("  deckd sound <name>       Play a sound from assets/sounds/");
                println!("  deckd sounds             List available sounds");
                println!("  deckd cameras             List connected UVC cameras");
                println!("  deckd auth google        Authorize Google Calendar access");
                println!("  deckd reload             Reload config without restarting");
                println!("  deckd help               Show this help");
                return;
            }
            _ => {} // Unknown subcommand — fall through to daemon startup
        }
    }

    start_daemon();
}

const LABEL: &str = "com.deckd.daemon";

fn plist_path() -> std::path::PathBuf {
    dirs::home_dir()
        .expect("home dir")
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LABEL))
}

fn install_service() {
    let binary = std::env::current_exe().expect("current exe path");
    let plist = plist_path();

    // Build PATH that includes nix, fnm, homebrew
    let home = dirs::home_dir().expect("home dir");
    let path = format!(
        "/usr/local/bin:/usr/bin:/bin:/run/current-system/sw/bin:{}",
        home.join("Library/Application Support/fnm/aliases/default/bin").display()
    );

    let content = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/deckd.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/deckd.stderr.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>PATH</key>
        <string>{path}</string>
    </dict>
    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>"#,
        label = LABEL,
        binary = binary.display(),
        path = path,
    );

    // Create LaunchAgents dir if needed
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    std::fs::write(&plist, content).expect("write plist");
    println!("Wrote {}", plist.display());

    // Load it
    let status = std::process::Command::new("launchctl")
        .args(["load", plist.to_str().unwrap()])
        .status();

    match status {
        Ok(s) if s.success() => println!("Service installed and started."),
        Ok(s) => println!("launchctl load exited with: {}", s),
        Err(e) => println!("Failed to run launchctl: {}", e),
    }
}

fn uninstall_service() {
    let plist = plist_path();

    if !plist.exists() {
        println!("Service not installed.");
        return;
    }

    // Unload
    let _ = std::process::Command::new("launchctl")
        .args(["unload", plist.to_str().unwrap()])
        .status();

    std::fs::remove_file(&plist).expect("remove plist");
    println!("Service uninstalled.");
}

fn setup_camera() {
    let cameras = uvc::find_cameras();

    if cameras.is_empty() {
        println!("No UVC cameras found.");
        return;
    }

    let chosen = if cameras.len() == 1 {
        println!("Found camera: {} ({})", cameras[0].name, cameras[0].id_string());
        &cameras[0]
    } else {
        println!("Found {} cameras:\n", cameras.len());
        for (i, cam) in cameras.iter().enumerate() {
            println!("  [{}] {} ({})", i + 1, cam.name, cam.id_string());
        }
        print!("\nChoose camera [1-{}]: ", cameras.len());
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let choice: usize = match input.trim().parse::<usize>() {
            Ok(n) if n >= 1 && n <= cameras.len() => n - 1,
            _ => {
                println!("Invalid choice.");
                return;
            }
        };
        &cameras[choice]
    };

    let config_path = config::resolve_config_path();
    let id = chosen.id_string();

    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            let new_content = if content.contains("camera =") || content.contains("camera=") {
                // Replace existing camera line
                let mut result = String::new();
                for line in content.lines() {
                    if line.trim_start().starts_with("camera") && line.contains('=') {
                        result.push_str(&format!("camera = \"{}\"", id));
                    } else {
                        result.push_str(line);
                    }
                    result.push('\n');
                }
                result
            } else {
                // Insert after default_page or at the top of the file
                let mut result = String::new();
                let mut inserted = false;
                for line in content.lines() {
                    result.push_str(line);
                    result.push('\n');
                    if !inserted && (line.starts_with("default_page") || line.starts_with("github_repo")) {
                        result.push_str(&format!("camera = \"{}\"\n", id));
                        inserted = true;
                    }
                }
                if !inserted {
                    result.push_str(&format!("camera = \"{}\"\n", id));
                }
                result
            };

            if let Err(e) = std::fs::write(&config_path, new_content) {
                eprintln!("Failed to write config: {}", e);
                return;
            }
        }
        Err(_) => {
            eprintln!("Config file not found at {}", config_path.display());
            eprintln!("Add this to your config.toml:");
            eprintln!("  camera = \"{}\"", id);
            return;
        }
    }

    println!("Saved camera = \"{}\" to {}", id, config_path.display());
}

fn send_udp(msg: &str) {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").expect("bind");
    socket.send_to(msg.as_bytes(), "127.0.0.1:9876").expect("send");
}

fn start_daemon() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config_path = config::resolve_config_path();
    let mut cfg = match config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config from {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };

    info!("deckd starting");
    info!("Config loaded from {}", config_path.display());

    let shutdown = Arc::new(AtomicBool::new(false));

    // Handle SIGINT/SIGTERM for clean shutdown
    {
        signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))
            .expect("Failed to register SIGINT handler");
        signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown))
            .expect("Failed to register SIGTERM handler");
    }

    // SIGHUP triggers config reload (same flag as UDP __reload)
    let reload_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload_flag))
        .expect("Failed to register SIGHUP handler");

    let mut deck = match deck::connect() {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to connect to Stream Deck: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "Connected to {} (serial: {})",
        deck.product().unwrap_or_else(|_| "unknown".into()),
        deck.serial_number().unwrap_or_else(|_| "unknown".into()),
    );

    deck.set_brightness(cfg.brightness.unwrap_or(80))
        .unwrap_or_else(|e| error!("Failed to set brightness: {}", e));

    // ── Boot + discovery ──────────────────────────────────────────
    // Check if we're restarting after a sleep/wake (stamp file < 30s old)
    let stamp_path = "/tmp/deckd.last_run";
    let stamp_age = std::fs::metadata(stamp_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok());
    let is_restart = stamp_age.map(|age| age < std::time::Duration::from_secs(30)).unwrap_or(false);
    info!("Stamp age: {:?}, is_restart: {}", stamp_age, is_restart);
    // Touch the stamp file
    std::fs::write(stamp_path, "").ok();

    let lights_enabled = std::env::var("DECKD_LIGHTS").map(|v| v != "0").unwrap_or(true);

    // Serial discovery is instant — do it now
    let mut all_lights = if lights_enabled {
        boot::render_boot_frame(&mut deck, 0.3, "scanning USB...");
        lights::discover_serial()
    } else {
        Vec::new()
    };

    // BLE scan runs in background — lights will be added when ready
    let ble_rx = if lights_enabled && !is_restart {
        boot::render_boot_frame(&mut deck, 0.5, "scanning BLE...");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let ble_lights = lights::discover_ble(&rt);
            tx.send(ble_lights).ok();
        });
        Some(rx)
    } else {
        None
    };

    // Initialize tokio runtime for BLE operations (for ongoing commands)
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    // Start dashboard background poller
    let dash_state = dashboard::new_shared();
    dashboard::start_poller(Arc::clone(&dash_state), cfg.github_repo.clone(), cfg.monitoring.clone());

    // Camera state
    let mut cam_state = camera::CameraState::new(cfg.camera.clone());

    // Timer
    let timer_state = timer::new_shared();

    // Tamagotchi pet
    let pet_name = cfg.pet_name.as_deref().unwrap_or("deckchi");
    let pet_state = tamagotchi::new_shared(pet_name);

    // Start notification listener (localhost:9876 UDP)
    notify::start_listener(Arc::clone(&dash_state), Arc::clone(&timer_state), Arc::clone(&pet_state), Arc::clone(&reload_flag));

    // Audio device cycler (caches device list, single subprocess per switch)
    let mut audio_cycler = audio::DeviceCycler::new(&cfg.output_devices, &cfg.input_devices);

    // CoreAudio listener — fires when devices connect/disconnect or default changes
    let devices_changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    audio::start_device_change_listener(Arc::clone(&devices_changed));

    let boot_start = Instant::now();

    // Render buttons immediately — interactive before BLE scan finishes
    let mut page_stack: Vec<String> = vec![cfg.start_page()];
    let buttons = cfg.resolved_buttons(&page_stack);
    render::render_buttons(&mut deck, &buttons);

    // Startup sound (skip on quick restarts to avoid noise on USB hub resets)
    if !is_restart {
        soundboard::play_named("success");
    }
    // LCD stays showing boot animation until BLE scan completes (or dashboard takes over)

    // Track when we last refreshed the LCD dashboard
    let mut last_lcd_refresh = Instant::now();
    let lcd_refresh_interval = std::time::Duration::from_secs(5);

    // Page list for swipe navigation
    let swipe_pages = ["main", "tools", "monitor", "ship"];

    // Track timer expiry to play sound once
    let mut timer_was_expired = false;

    // Track config file mtime for auto-reload
    let mut last_config_mtime = std::fs::metadata(&config_path)
        .and_then(|m| m.modified())
        .ok();
    let mut last_mtime_check = Instant::now();

    // Main loop
    let mut read_errors = 0u32;
    let mut touch_state = deck::TouchState::new();
    let mut ble_pending = ble_rx;
    let mut is_booting = ble_pending.is_some();
    // Sleep/wake detection: compare wall-clock delta across iterations.
    // If wall clock jumps forward much more than the loop period, we slept.
    let mut last_wake_check = std::time::SystemTime::now();
    while !shutdown.load(Ordering::Relaxed) {
        // Detect wake from sleep: wall-clock gap >> loop iteration time
        if let Ok(elapsed) = std::time::SystemTime::now().duration_since(last_wake_check) {
            if elapsed > std::time::Duration::from_secs(10) {
                info!("Detected wake from sleep (gap: {:?}), refreshing deck state", elapsed);
                // Re-render current page buttons — the Stream Deck firmware
                // blanks button images during macOS sleep.
                render_page(&mut deck, &cfg, &page_stack);
                // Re-apply stateful button overrides for the current page
                let cp = current_page(&page_stack);
                if cp == "keylights" {
                    render::render_light_toggle_button(&mut deck, lights::keylights_on(&all_lights));
                } else if cp == "desklights" {
                    render::render_light_toggle_button(&mut deck, lights::desklights_on(&all_lights));
                } else if cp == "meeting" {
                    let muted = dash_state.lock().map(|s| s.mic_muted).unwrap_or(false);
                    render::render_mic_button(&mut deck, muted);
                } else if cp == "cam_settings" {
                    cam_state.sync_from_device();
                    render::render_camera_state_buttons(&mut deck, &cam_state);
                }
                // Force LCD refresh next tick
                last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                // Restore brightness (in case firmware dimmed it)
                deck.set_brightness(cfg.brightness.unwrap_or(80)).ok();
            }
        }
        last_wake_check = std::time::SystemTime::now();

        // Check if BLE scan completed in background
        if let Some(ref rx) = ble_pending {
            if let Ok(ble_lights) = rx.try_recv() {
                // Dedup: skip lights already connected (by name)
                let existing_names: Vec<String> = all_lights.iter().map(|l| l.name.clone()).collect();
                let new_lights: Vec<_> = ble_lights
                    .into_iter()
                    .filter(|l| !existing_names.contains(&l.name))
                    .collect();
                let new_count = new_lights.len();
                all_lights.extend(new_lights);
                ble_pending = None;

                if is_booting {
                    info!("BLE scan complete: {} light(s)", all_lights.len());
                    boot::render_boot_complete(&mut deck, all_lights.len());
                    is_booting = false;
                } else {
                    info!("BLE rescan complete: {} new light(s) ({} total)", new_count, all_lights.len());
                }
                last_lcd_refresh = Instant::now() - lcd_refresh_interval;
            } else if is_booting {
                // Still scanning at boot — animate boot progress on LCD
                let elapsed = boot_start.elapsed().as_secs_f32();
                let progress = (0.3 + elapsed / 8.0).min(0.95);
                let dots = match (elapsed as u32) % 4 {
                    0 => "scanning BLE",
                    1 => "scanning BLE.",
                    2 => "scanning BLE..",
                    _ => "scanning BLE...",
                };
                boot::render_boot_frame(&mut deck, progress, dots);
            }
        }

        let poll_result = deck::poll_and_dispatch(&deck, &cfg, &page_stack, &mut read_errors, &mut touch_state);
        if poll_result.is_err() {
            break;
        }
        if let Ok(Some(result)) = poll_result {
            match result {
                deck::InputResult::SwitchPage(new_page) => {
                    info!("Switching to page: {}", new_page);
                    push_page(&mut page_stack, new_page);
                    render_page(&mut deck, &cfg, &page_stack);
                    let cp = current_page(&page_stack);
                    // Sync camera state when entering camera settings page
                    if cp == "cam_settings" {
                        cam_state.sync_from_device();
                        render::render_camera_state_buttons(&mut deck, &cam_state);
                    }
                    // Render stateful light toggle on entry
                    if cp == "keylights" {
                        render::render_light_toggle_button(&mut deck, lights::keylights_on(&all_lights));
                    } else if cp == "desklights" {
                        render::render_light_toggle_button(&mut deck, lights::desklights_on(&all_lights));
                    }
                    // Render stateful mic button on meeting page entry
                    if cp == "meeting" {
                        let muted = dash_state.lock().map(|s| s.mic_muted).unwrap_or(false);
                        render::render_mic_button(&mut deck, muted);
                    }
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                }
                deck::InputResult::NeewerCommand(cmd) => {
                    if all_lights.is_empty() {
                        warn!("Light command '{}' but no lights connected", cmd);
                        continue;
                    }
                    handle_light_command(&mut all_lights, &cmd, &rt);
                    let cp = current_page(&page_stack);
                    if cp == "keylights" {
                        render::render_light_toggle_button(&mut deck, lights::keylights_on(&all_lights));
                    } else if cp == "desklights" {
                        render::render_light_toggle_button(&mut deck, lights::desklights_on(&all_lights));
                    }
                }
                deck::InputResult::BleScan => {
                    if ble_pending.is_some() {
                        info!("BLE scan already in progress, ignoring");
                    } else {
                        info!("Starting BLE rescan...");
                        let (tx, rx) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .expect("tokio runtime");
                            let ble_lights = lights::discover_ble(&rt);
                            tx.send(ble_lights).ok();
                        });
                        ble_pending = Some(rx);
                    }
                }
                deck::InputResult::CameraCommand(cmd) => {
                    handle_camera_command(&mut cam_state, &cmd);
                    if current_page(&page_stack) == "cam_settings" {
                        render::render_camera_state_buttons(&mut deck, &cam_state);
                    }
                }
                deck::InputResult::AudioCommand(cmd, amount) => {
                    handle_audio_command(&dash_state, &cmd, amount, &mut audio_cycler);
                    let cp = current_page(&page_stack);
                    if cmd == "mic_toggle" && cp == "meeting" {
                        let muted = dash_state.lock().map(|s| s.mic_muted).unwrap_or(false);
                        render::render_mic_button(&mut deck, muted);
                    }
                    // Instant LCD refresh on base page
                    if cp == cfg.start_page() {
                        if let Ok(dash) = dash_state.lock() {
                            render::render_lcd_dashboard(&mut deck, &dash, &timer_state, &pet_state);
                        }
                        last_lcd_refresh = Instant::now();
                    }
                }
                deck::InputResult::ActionFired(ref action) => {
                    let cp = current_page(&page_stack);
                    if cp == cfg.start_page() {
                        apply_optimistic_update(&dash_state, action);
                        if let Ok(dash) = dash_state.lock() {
                            render::render_lcd_dashboard(&mut deck, &dash, &timer_state, &pet_state);
                        }
                        last_lcd_refresh = Instant::now();
                    }
                }
                deck::InputResult::LcdDoubleTap(segment) => {
                    let cp = current_page(&page_stack);
                    let is_pet_pg = cfg.pet_pages.iter().any(|p| p == cp);
                    if is_pet_pg {
                        if let Ok(mut p) = pet_state.lock() {
                            p.pet();
                        }
                    } else if cp == cfg.start_page() {
                        handle_lcd_double_tap(segment, &dash_state, cfg.github_repo.as_deref());
                    }
                }
                deck::InputResult::SwipePage(direction) => {
                    let cp = current_page(&page_stack);
                    let current_idx = swipe_pages.iter().position(|&p| p == cp);
                    let new_page = match current_idx {
                        Some(idx) => {
                            let len = swipe_pages.len() as i32;
                            let new_idx = ((idx as i32 + direction as i32) % len + len) % len;
                            swipe_pages[new_idx as usize].to_string()
                        }
                        None => cfg.start_page(),
                    };
                    if new_page != cp {
                        info!("Swipe → page: {}", new_page);
                        // Swipe resets the stack — lands on a root page
                        page_stack = vec![new_page];
                        render_page(&mut deck, &cfg, &page_stack);
                        last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                    }
                }
                deck::InputResult::TimerCommand(cmd) => {
                    if let Ok(mut t) = timer_state.lock() {
                        match cmd.as_str() {
                            "start_25" => t.start(25),
                            "start_5" => t.start(5),
                            "start_10" => t.start(10),
                            "start_15" => t.start(15),
                            "toggle" => {
                                if t.is_running() {
                                    t.toggle_pause();
                                } else {
                                    t.start(25);
                                }
                            }
                            "stop" => t.stop(),
                            _ => {}
                        }
                    }
                    // Force LCD refresh to show timer
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                }
            }
        }

        // React to audio device changes (headphones plugged in, Zoom starting, etc.)
        if devices_changed.swap(false, Ordering::Relaxed) {
            info!("Audio device change detected, refreshing");
            audio_cycler.refresh(&cfg.output_devices, &cfg.input_devices);
            if let Ok(mut s) = dash_state.lock() {
                s.audio_suppress_until = std::time::Instant::now();
            }
            dashboard::refresh_audio(&dash_state);
            // Also check meeting state immediately — Zoom creates ZoomAudioDevice
            dashboard::check_meeting(&dash_state);
            if current_page(&page_stack) == cfg.start_page() {
                if let Ok(dash) = dash_state.lock() {
                    render::render_lcd_dashboard(&mut deck, &dash, &timer_state, &pet_state);
                }
                last_lcd_refresh = Instant::now();
            }
        }

        // Tick the pet
        if let Ok(mut p) = pet_state.lock() {
            p.tick();
        }

        // Check config file mtime every 2 seconds for auto-reload
        if last_mtime_check.elapsed() >= std::time::Duration::from_secs(2) {
            last_mtime_check = Instant::now();
            let current_mtime = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            if current_mtime != last_config_mtime && current_mtime.is_some() {
                info!("Config file changed on disk, triggering reload");
                last_config_mtime = current_mtime;
                reload_flag.store(true, Ordering::Relaxed);
            }
        }

        // Hot-reload config
        if reload_flag.swap(false, Ordering::Relaxed) {
            match config::load(&config_path) {
                Ok(new_cfg) => {
                    cfg = new_cfg;
                    info!("Config reloaded from {}", config_path.display());
                    audio_cycler.refresh(&cfg.output_devices, &cfg.input_devices);
                    render_page(&mut deck, &cfg, &page_stack);
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;

                    if let Ok(mut s) = dash_state.lock() {
                        s.notifications.push(dashboard::Notification {
                            message: "Config reloaded".into(),
                            created: Instant::now(),
                        });
                    }
                }
                Err(e) => {
                    error!("Failed to reload config: {}", e);
                    if let Ok(mut s) = dash_state.lock() {
                        s.notifications.push(dashboard::Notification {
                            message: format!("Reload failed: {}", e),
                            created: Instant::now(),
                        });
                    }
                }
            }
        }

        // Auto-switch pages on meeting start/end
        {
            let meeting_changed = dash_state.lock().ok()
                .map(|s| s.meeting_changed)
                .unwrap_or(false);

            if meeting_changed
                && let Ok(mut s) = dash_state.lock()
            {
                s.meeting_changed = false;
                let in_meeting = s.in_meeting;
                drop(s);

                let cp = current_page(&page_stack);
                if in_meeting && cp == cfg.start_page() {
                    // Auto-enter meeting mode — replaces stack
                    info!("Auto-entering meeting mode");
                    handle_light_command(&mut all_lights, "preset:70:5000:keylights", &rt);
                    handle_light_command(&mut all_lights, "preset:30:5000:desklights", &rt);
                    page_stack = vec![cfg.start_page(), "meeting".into()];
                    render_page(&mut deck, &cfg, &page_stack);
                    let muted = dash_state.lock().map(|s| s.mic_muted).unwrap_or(false);
                    render::render_mic_button(&mut deck, muted);
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                } else if !in_meeting && cp == "meeting" {
                    // Auto-exit meeting mode — back to base
                    info!("Auto-exiting meeting mode");
                    handle_light_command(&mut all_lights, "preset:50:4400:keylights", &rt);
                    handle_light_command(&mut all_lights, "preset:50:4400:desklights", &rt);
                    page_stack = vec![cfg.start_page()];
                    render_page(&mut deck, &cfg, &page_stack);
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                }
            }
        }

        // Periodically refresh the LCD strip with dashboard data
        // Refresh faster when a notification is active (for the pulse animation)
        let has_notification = dash_state.lock().ok()
            .map(|s| s.notifications.last()
                .map(|n| n.created.elapsed() < std::time::Duration::from_secs(5))
                .unwrap_or(false))
            .unwrap_or(false);

        let timer_active = timer_state.lock().ok()
            .map(|t| t.is_running())
            .unwrap_or(false);

        // Play sound when timer expires (once)
        if let Ok(t) = timer_state.lock() {
            let expired = t.is_expired();
            if expired && !timer_was_expired {
                soundboard::play_named("timer_done");
            }
            timer_was_expired = expired;
        }

        let cp = current_page(&page_stack);
        let base_page = cfg.start_page();
        let is_pet_page = cfg.pet_pages.iter().any(|p| p == cp);
        let overlay_positions = cfg.overlay_encoder_positions(&page_stack);
        let has_overlay = !overlay_positions.is_empty();
        let pet_animating = pet_state.lock().ok()
            .map(|p| p.action != crate::tamagotchi::Action::Idle && p.action != crate::tamagotchi::Action::Napping)
            .unwrap_or(false);
        // Pet may show via fallthrough on overlay pages (in base-layer segments 2-3)
        let pet_visible = is_pet_page
            || (has_overlay && !overlay_positions.contains("2") && !overlay_positions.contains("3"));

        let refresh_interval = if has_notification {
            std::time::Duration::from_millis(150)
        } else if timer_active {
            std::time::Duration::from_millis(500)
        } else if pet_animating && (pet_visible || cp == base_page) {
            std::time::Duration::from_millis(400) // Smooth pet animation
        } else if is_pet_page {
            std::time::Duration::from_millis(400)
        } else {
            lcd_refresh_interval
        };

        if ble_pending.is_some() && is_booting {
            // Boot scan still running — boot animation owns the LCD, skip dashboard rendering
        } else if last_lcd_refresh.elapsed() >= refresh_interval {
            // Clean up stale notifications + feed pet from GitHub events
            if let Ok(mut s) = dash_state.lock() {
                if let Ok(mut p) = pet_state.lock() {
                    for notif in &s.notifications {
                        if notif.message.contains("ready to merge") {
                            p.ship_pr();
                        } else if notif.message.contains("approved") {
                            p.feed();
                        } else if notif.message.contains("Review completed") {
                            p.complete_review();
                        }
                    }
                    p.pending_reviews(s.review_requests);
                }
                s.notifications.retain(|n| n.created.elapsed() < std::time::Duration::from_secs(6));
            }

            if cp == base_page {
                // Base page: full dashboard
                if let Ok(dash) = dash_state.lock() {
                    render::render_lcd_dashboard(&mut deck, &dash, &timer_state, &pet_state);
                }
            } else if cp == "monitor" {
                if let Ok(dash) = dash_state.lock() {
                    render::render_monitor_lcd(&mut deck, &dash);
                }
            } else if is_pet_page {
                // Explicit full-width pet page
                if let Ok(p) = pet_state.lock() {
                    render::render_pet_lcd(&mut deck, &p, 800, 100);
                }
            } else {
                // Overlay page: static labels for overlay positions, dashboard for the rest
                let resolved = cfg.resolved_encoders(&page_stack);
                render::render_overlay_encoder_labels(&mut deck, &resolved, &overlay_positions);
                if let Ok(dash) = dash_state.lock() {
                    render::render_lcd_dashboard_segments(
                        &mut deck, &dash, &timer_state, &pet_state, &overlay_positions,
                    );
                }
            }
            last_lcd_refresh = Instant::now();
        }
    }

    info!("deckd shutting down");
    std::fs::write(stamp_path, "").ok(); // Update stamp so next start detects quick restart
    if let Ok(p) = pet_state.lock() {
        p.save();
    }
    deck.set_brightness(0).ok();
    deck.clear_all_button_images().ok();
    deck.flush().ok();
}

/// Push a page onto the layer stack. If the page is already in the stack,
/// truncate to it (prevents cycles, like browser history).
fn push_page(stack: &mut Vec<String>, page: String) {
    if let Some(idx) = stack.iter().position(|p| p == &page) {
        stack.truncate(idx + 1);
    } else {
        stack.push(page);
        if stack.len() > 8 {
            stack.drain(..stack.len() - 8);
        }
    }
}

fn current_page(stack: &[String]) -> &str {
    stack.last().map(|s| s.as_str()).unwrap_or("main")
}

fn render_page(deck: &mut elgato_streamdeck::StreamDeck, cfg: &config::Config, stack: &[String]) {
    let buttons = cfg.resolved_buttons(stack);
    render::render_buttons(deck, &buttons);
    // LCD is handled entirely by the main loop's refresh cycle
}

fn handle_light_command(
    all_lights: &mut [lights::Light],
    cmd_str: &str,
    rt: &tokio::runtime::Runtime,
) {
    // Parse preset commands: "preset:brightness:temp_k[:group]"
    // Parse regular commands: "command[:group]"
    let parts: Vec<&str> = cmd_str.split(':').collect();

    let (cmd, group, preset_brt, preset_temp) = if parts[0] == "preset" && parts.len() >= 3 {
        let brt: u8 = parts[1].parse().unwrap_or(50);
        let temp: u16 = parts[2].parse().unwrap_or(4400);
        let grp = parts.get(3).copied();
        ("preset", grp, brt, temp)
    } else if parts.len() == 2 {
        (parts[0], Some(parts[1]), 0, 0)
    } else {
        (parts[0], None, 0, 0)
    };

    for light in all_lights.iter_mut() {
        let matches = match group {
            Some("keylights") => light.is_gl1,
            Some("desklights") => light.is_pl81,
            Some("ble") => light.is_gl1,
            Some("serial") => light.is_pl81,
            None => true,
            Some(_) => true,
        };

        if !matches {
            continue;
        }

        let result = match cmd {
            "on" => light.set_power(true, rt),
            "off" => light.set_power(false, rt),
            "toggle" => light.toggle_power(rt),
            "brightness_up" => light.adjust_brightness(5, rt),
            "brightness_down" => light.adjust_brightness(-5, rt),
            "temp_warmer" => light.adjust_temp(-100, rt),
            "temp_cooler" => light.adjust_temp(100, rt),
            "temp_reset" => light.reset_temp(rt),
            "preset" => light.set_preset(preset_brt, preset_temp, rt),
            other => {
                warn!("Unknown light command: {}", other);
                Ok(())
            }
        };
        if let Err(e) = result {
            error!("Light '{}' command '{}' failed: {}", light.name, cmd, e);
        }
    }
}

fn handle_camera_command(state: &mut camera::CameraState, cmd: &str) {
    let result = match cmd {
        "zoom_in" => camera::zoom_in(state),
        "zoom_out" => camera::zoom_out(state),
        "zoom_reset" => camera::zoom_reset(state),
        "pan_left" => camera::pan_left(state),
        "pan_right" => camera::pan_right(state),
        "tilt_up" => camera::tilt_up(state),
        "tilt_down" => camera::tilt_down(state),
        "autofocus" => camera::toggle_autofocus(state),
        "brightness_up" => camera::adjust_brightness(state, 10),
        "brightness_down" => camera::adjust_brightness(state, -10),
        "contrast_up" => camera::adjust_contrast(state, 10),
        "contrast_down" => camera::adjust_contrast(state, -10),
        "saturation_up" => camera::adjust_saturation(state, 10),
        "saturation_down" => camera::adjust_saturation(state, -10),
        "sharpness_up" => camera::adjust_sharpness(state, 10),
        "sharpness_down" => camera::adjust_sharpness(state, -10),
        "wb_up" => camera::adjust_white_balance(state, 200),
        "wb_down" => camera::adjust_white_balance(state, -200),
        "wb_auto" => camera::toggle_auto_white_balance(state),
        "ae_auto" | "auto_exposure" => camera::toggle_auto_exposure(state),
        "fov_wide" => camera::set_fov_wide(state),
        "fov_medium" => camera::set_fov_medium(state),
        "fov_narrow" => camera::set_fov_narrow(state),
        "fov_cycle" => camera::cycle_fov(state),
        "rightlight" => camera::toggle_rightlight(state),
        other => {
            warn!("Unknown camera command: {}", other);
            Ok(())
        }
    };
    if let Err(e) = result {
        error!("Camera command '{}' failed: {}", cmd, e);
    }
}

fn handle_audio_command(
    state: &dashboard::SharedDashboard,
    cmd: &str,
    amount: i8,
    cycler: &mut audio::DeviceCycler,
) {
    let abs_amount = (amount.unsigned_abs() as i32).clamp(1, 10);
    let delta = abs_amount + 1;
    let direction = if amount >= 0 { 1i8 } else { -1i8 };

    match cmd {
        "volume_up" => {
            let new_vol = audio::adjust_output_volume(delta);
            dashboard::nudge_volume(state, 0);
            if let Ok(mut s) = state.lock() {
                s.volume = new_vol.to_string();
            }
        }
        "volume_down" => {
            let new_vol = audio::adjust_output_volume(-delta);
            dashboard::nudge_volume(state, 0);
            if let Ok(mut s) = state.lock() {
                s.volume = new_vol.to_string();
            }
        }
        "mic_toggle" => {
            let now_muted = audio::toggle_input_mute();
            dashboard::set_mic_muted(state, now_muted);
        }
        "cycle_output" => {
            let new_device = cycler.cycle_output(direction);
            if let Ok(mut s) = state.lock() {
                s.audio_output = new_device;
                s.audio_suppress_until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
            }
        }
        "cycle_input" => {
            let new_device = cycler.cycle_input(direction);
            if let Ok(mut s) = state.lock() {
                s.audio_input = new_device.clone();
                s.input_flash = Some((new_device, std::time::Instant::now()));
                s.audio_suppress_until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
            }
        }
        other => {
            warn!("Unknown audio command: {}", other);
        }
    }
}

fn handle_lcd_double_tap(segment: u8, dash_state: &dashboard::SharedDashboard, github_repo: Option<&str>) {
    match segment {
        0 => {
            // Toggle mic mute
            info!("LCD double-tap segment 0 → toggling mic mute");
            let now_muted = audio::toggle_input_mute();
            dashboard::set_mic_muted(dash_state, now_muted);
        }
        1 => {
            // Cycle output device (forward)
            info!("LCD double-tap segment 1 → cycling output device");
            // We can't access the cycler here, so use a shell command
            std::process::Command::new("SwitchAudioSource")
                .args(["-n"])
                .spawn()
                .ok();
        }
        2 | 3 => {
            // Check if now-playing is showing on segments 2-3
            let has_music = dash_state.lock().ok()
                .map(|s| {
                    s.now_playing_state == "playing"
                        || (s.now_playing_state == "paused" && !s.now_playing_title.is_empty())
                })
                .unwrap_or(false);

            if has_music {
                info!("LCD double-tap segment {} → opening Spotify", segment);
                std::process::Command::new("open")
                    .arg("-a")
                    .arg("Spotify")
                    .spawn()
                    .ok();
            } else if let Some(repo) = github_repo {
                let url = if segment == 2 {
                    format!("https://github.com/{}/pulls?q=is%3Apr+is%3Aopen+review-requested%3A%40me", repo)
                } else {
                    format!("https://github.com/{}/pulls?q=is%3Apr+is%3Aopen+author%3A%40me+review%3Aapproved", repo)
                };
                info!("LCD double-tap segment {} → opening URL", segment);
                std::process::Command::new("open").arg(url).spawn().ok();
            }
        }
        _ => {}
    }
}

fn apply_optimistic_update(state: &dashboard::SharedDashboard, action: &config::Action) {
    if let config::Action::Shell { command } = action {
        if command.contains("output volume") && command.contains("+ 5") {
            dashboard::nudge_volume(state, 5);
        } else if command.contains("output volume") && command.contains("- 5") {
            dashboard::nudge_volume(state, -5);
        } else if command.contains("-m toggle -t input") {
            // Mic mute toggle — flip current state
            if let Ok(s) = state.lock() {
                let currently_muted = s.mic_muted;
                drop(s);
                dashboard::set_mic_muted(state, !currently_muted);
            }
        } else if command.contains("SwitchAudioSource -n") {
            dashboard::mark_audio_changed(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_page_simple_push() {
        let mut stack = vec!["main".into()];
        push_page(&mut stack, "tools".into());
        assert_eq!(stack, vec!["main", "tools"]);
    }

    #[test]
    fn push_page_truncates_on_cycle() {
        let mut stack = vec!["main".into(), "tools".into(), "keylights".into()];
        push_page(&mut stack, "main".into());
        assert_eq!(stack, vec!["main"]);
    }

    #[test]
    fn push_page_truncates_to_mid_stack() {
        let mut stack = vec!["main".into(), "tools".into(), "keylights".into()];
        push_page(&mut stack, "tools".into());
        assert_eq!(stack, vec!["main", "tools"]);
    }

    #[test]
    fn push_page_noop_for_current() {
        let mut stack = vec!["main".into(), "tools".into()];
        push_page(&mut stack, "tools".into());
        assert_eq!(stack, vec!["main", "tools"]);
    }

    #[test]
    fn push_page_depth_cap() {
        let mut stack: Vec<String> = (0..8).map(|i| format!("p{}", i)).collect();
        push_page(&mut stack, "p8".into());
        assert_eq!(stack.len(), 8);
        assert_eq!(stack.last().unwrap(), "p8");
        assert_eq!(stack.first().unwrap(), "p1"); // p0 was drained
    }

    #[test]
    fn current_page_returns_last() {
        let stack = vec!["main".into(), "tools".into()];
        assert_eq!(current_page(&stack), "tools");
    }

    #[test]
    fn current_page_empty_stack_defaults() {
        let stack: Vec<String> = vec![];
        assert_eq!(current_page(&stack), "main");
    }
}
