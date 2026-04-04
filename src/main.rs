mod actions;
mod audio;
mod boot;
mod camera;
mod config;
mod dashboard;
mod deck;
mod lights;
mod notify;
mod render;
mod soundboard;
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
            "reload" => {
                send_udp("__reload");
                println!("Reload signal sent.");
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
                println!("  deckd notify <message>   Show notification on LCD");
                println!("  deckd timer [toggle|start_25|start_5|start_10|stop]");
                println!("  deckd sound <name>       Play a sound from assets/sounds/");
                println!("  deckd sounds             List available sounds");
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
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
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
    let is_restart = std::fs::metadata(stamp_path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() < std::time::Duration::from_secs(30))
        .unwrap_or(false);
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
    dashboard::start_poller(Arc::clone(&dash_state));

    // Camera state
    let mut cam_state = camera::CameraState::new();

    // Timer
    let timer_state = timer::new_shared();

    // Start notification listener (localhost:9876 UDP)
    let reload_flag = Arc::new(AtomicBool::new(false));
    notify::start_listener(Arc::clone(&dash_state), Arc::clone(&timer_state), Arc::clone(&reload_flag));

    // Audio device cycler (caches device list, single subprocess per switch)
    let mut audio_cycler = audio::DeviceCycler::new(&cfg.output_devices, &cfg.input_devices);

    // CoreAudio listener — fires when devices connect/disconnect or default changes
    let devices_changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    audio::start_device_change_listener(Arc::clone(&devices_changed));

    let boot_start = Instant::now();

    // Render buttons immediately — interactive before BLE scan finishes
    let mut current_page = cfg.start_page();
    let buttons = cfg.active_buttons(&current_page);
    render::render_buttons(&mut deck, buttons);
    // LCD stays showing boot animation until BLE scan completes (or dashboard takes over)

    // Track when we last refreshed the LCD dashboard
    let mut last_lcd_refresh = Instant::now();
    let lcd_refresh_interval = std::time::Duration::from_secs(5);

    // Main loop
    let mut read_errors = 0u32;
    let mut touch_state = deck::TouchState::new();
    let mut ble_pending = ble_rx;
    while !shutdown.load(Ordering::Relaxed) {
        // Check if BLE scan completed in background
        if let Some(ref rx) = ble_pending {
            if let Ok(ble_lights) = rx.try_recv() {
                info!("BLE scan complete: {} light(s)", ble_lights.len());
                all_lights.extend(ble_lights);
                ble_pending = None;
                boot::render_boot_complete(&mut deck, all_lights.len());
                last_lcd_refresh = Instant::now() - lcd_refresh_interval;
            } else {
                // Still scanning — animate boot progress on LCD
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

        let poll_result = deck::poll_and_dispatch(&deck, &cfg, &current_page, &mut read_errors, &mut touch_state);
        if poll_result.is_err() {
            break;
        }
        if let Ok(Some(result)) = poll_result {
            match result {
                deck::InputResult::SwitchPage(new_page) => {
                    info!("Switching to page: {}", new_page);
                    current_page = new_page;
                    render_page(&mut deck, &cfg, &current_page);
                    // Force LCD refresh on page switch
                    last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                }
                deck::InputResult::NeewerCommand(cmd) => {
                    if all_lights.is_empty() {
                        warn!("Light command '{}' but no lights connected", cmd);
                        continue;
                    }
                    handle_light_command(&mut all_lights, &cmd, &rt);
                }
                deck::InputResult::CameraCommand(cmd) => {
                    handle_camera_command(&mut cam_state, &cmd);
                }
                deck::InputResult::AudioCommand(cmd, amount) => {
                    handle_audio_command(&dash_state, &cmd, amount, &mut audio_cycler);
                    // Instant LCD refresh
                    if current_page == "main" {
                        if let Ok(dash) = dash_state.lock() {
                            let encoders = cfg.active_encoders(&current_page);
                            render::render_lcd_dashboard(&mut deck, encoders, &dash, &timer_state);
                        }
                        last_lcd_refresh = Instant::now();
                    }
                }
                deck::InputResult::ActionFired(ref action) => {
                    if current_page == "main" {
                        apply_optimistic_update(&dash_state, action);
                        if let Ok(dash) = dash_state.lock() {
                            let encoders = cfg.active_encoders(&current_page);
                            render::render_lcd_dashboard(&mut deck, encoders, &dash, &timer_state);
                        }
                        last_lcd_refresh = Instant::now();
                    }
                }
                deck::InputResult::LcdDoubleTap(segment) => {
                    handle_lcd_double_tap(segment);
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

        // React to audio device changes (headphones plugged in, etc.)
        if devices_changed.swap(false, Ordering::Relaxed) {
            info!("Audio device change detected, refreshing");
            audio_cycler.refresh(&cfg.output_devices, &cfg.input_devices);
            // Force dashboard audio re-poll and LCD refresh
            if let Ok(mut s) = dash_state.lock() {
                s.audio_suppress_until = std::time::Instant::now(); // allow poll
            }
            dashboard::refresh_audio(&dash_state);
            if current_page == "main" {
                if let Ok(dash) = dash_state.lock() {
                    let encoders = cfg.active_encoders(&current_page);
                    render::render_lcd_dashboard(&mut deck, encoders, &dash, &timer_state);
                }
                last_lcd_refresh = Instant::now();
            }
        }

        // Hot-reload config
        if reload_flag.swap(false, Ordering::Relaxed) {
            match config::load(&config_path) {
                Ok(new_cfg) => {
                    cfg = new_cfg;
                    info!("Config reloaded from {}", config_path.display());
                    audio_cycler.refresh(&cfg.output_devices, &cfg.input_devices);
                    render_page(&mut deck, &cfg, &current_page);
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

            if meeting_changed {
                if let Ok(mut s) = dash_state.lock() {
                    s.meeting_changed = false;
                    let in_meeting = s.in_meeting;
                    drop(s);

                    if in_meeting && current_page == "main" {
                        // Auto-enter meeting mode
                        info!("Auto-entering meeting mode");
                        // Set light presets
                        handle_light_command(&mut all_lights, "preset:70:5000:keylights", &rt);
                        handle_light_command(&mut all_lights, "preset:30:5000:desklights", &rt);
                        current_page = "meeting".into();
                        render_page(&mut deck, &cfg, &current_page);
                        last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                    } else if !in_meeting && current_page == "meeting" {
                        // Auto-exit meeting mode
                        info!("Auto-exiting meeting mode");
                        handle_light_command(&mut all_lights, "preset:50:4400:keylights", &rt);
                        handle_light_command(&mut all_lights, "preset:50:4400:desklights", &rt);
                        current_page = "main".into();
                        render_page(&mut deck, &cfg, &current_page);
                        last_lcd_refresh = Instant::now() - lcd_refresh_interval;
                    }
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

        let refresh_interval = if has_notification {
            std::time::Duration::from_millis(150)
        } else if timer_active {
            std::time::Duration::from_millis(500) // Update timer every 500ms
        } else {
            lcd_refresh_interval
        };

        if current_page == "main" && last_lcd_refresh.elapsed() >= refresh_interval {
            // Clean up stale notifications
            if let Ok(mut s) = dash_state.lock() {
                s.notifications.retain(|n| n.created.elapsed() < std::time::Duration::from_secs(6));
            }

            if let Ok(dash) = dash_state.lock() {
                let encoders = cfg.active_encoders(&current_page);
                render::render_lcd_dashboard(&mut deck, encoders, &dash, &timer_state);
            }
            last_lcd_refresh = Instant::now();
        }
    }

    info!("deckd shutting down");
    deck.set_brightness(0).ok();
    deck.clear_all_button_images().ok();
    deck.flush().ok();
}

fn render_page(deck: &mut elgato_streamdeck::StreamDeck, cfg: &config::Config, page: &str) {
    let buttons = cfg.active_buttons(page);
    let encoders = cfg.active_encoders(page);
    render::render_buttons(deck, buttons);
    render::render_lcd_strip(deck, encoders);
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

fn handle_lcd_double_tap(segment: u8) {
    let url = match segment {
        0 => None, // Volume — no URL
        1 => None, // Audio device — no URL
        2 => Some("https://github.com/shawnrice/deckd/pulls?q=is%3Apr+is%3Aopen+review-requested%3A%40me"),
        3 => Some("https://github.com/shawnrice/deckd/pulls?q=is%3Apr+is%3Aopen+author%3A%40me+review%3Aapproved"),
        _ => None,
    };
    if let Some(url) = url {
        info!("LCD double-tap segment {} → opening URL", segment);
        std::process::Command::new("open").arg(url).spawn().ok();
    }
}

fn apply_optimistic_update(state: &dashboard::SharedDashboard, action: &config::Action) {
    match action {
        config::Action::Shell { command } => {
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
        _ => {}
    }
}
