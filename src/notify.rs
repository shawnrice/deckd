use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use log::{error, info};

use crate::dashboard::{Notification, SharedDashboard};
use crate::timer::SharedTimer;

const LISTEN_PORT: u16 = 9876;

/// Start a UDP listener on localhost:9876 for external notifications.
/// Claude Code hooks, scripts, etc. can send messages here.
///
/// Protocol: plain UTF-8 text. The message becomes an LCD notification.
///
/// Example from a Claude Code hook:
///   echo "Task complete: tests passed" | nc -u -w0 localhost 9876
///
/// Or from a shell script:
///   echo "Deploy finished" > /dev/udp/localhost/9876
pub fn start_listener(dashboard: SharedDashboard, timer: SharedTimer, reload_flag: Arc<AtomicBool>) {
    thread::spawn(move || {
        let socket = match UdpSocket::bind(format!("127.0.0.1:{}", LISTEN_PORT)) {
            Ok(s) => {
                info!("Notification listener started on localhost:{}", LISTEN_PORT);
                s
            }
            Err(e) => {
                error!("Could not bind notification listener on port {}: {}", LISTEN_PORT, e);
                return;
            }
        };

        let mut buf = [0u8; 512];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((n, _addr)) => {
                    if let Ok(msg) = std::str::from_utf8(&buf[..n]) {
                        let msg = msg.trim().to_string();
                        if msg.is_empty() {
                            continue;
                        }

                        if msg == "__reload" {
                            info!("Reload requested via UDP");
                            reload_flag.store(true, Ordering::Relaxed);
                        } else if let Some(cmd) = msg.strip_prefix("__timer:") {
                            info!("Timer command via UDP: {}", cmd);
                            if let Ok(mut t) = timer.lock() {
                                match cmd {
                                    "start_25" => t.start(25),
                                    "start_5" => t.start(5),
                                    "start_10" => t.start(10),
                                    "start_15" => t.start(15),
                                    "toggle" => {
                                        if t.is_running() { t.toggle_pause(); } else { t.start(25); }
                                    }
                                    "stop" => t.stop(),
                                    _ => {}
                                }
                            }
                        } else {
                            info!("Notification received: {}", msg);
                            if let Ok(mut s) = dashboard.lock() {
                                s.notifications.push(Notification {
                                    message: msg,
                                    created: Instant::now(),
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Notification listener error: {}", e);
                }
            }
        }
    });
}
