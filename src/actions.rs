use std::process::Command;

use log::{error, info};

use crate::config::Action;

/// Execute an action. Returns Some(page) if a page switch was requested.
pub fn execute(action: &Action) {
    match action {
        Action::Shell { command } => {
            info!("Executing: {}", command);
            let result = Command::new("sh")
                .arg("-c")
                .arg(command)
                .spawn();

            if let Err(e) = result {
                error!("Failed to execute command '{}': {}", command, e);
            }
        }

        Action::Open { path } => {
            info!("Opening: {}", path);
            let result = Command::new("open")
                .arg(path)
                .spawn();

            if let Err(e) = result {
                error!("Failed to open '{}': {}", path, e);
            }
        }

        Action::Url { url } => {
            info!("Opening URL: {}", url);
            let result = Command::new("open")
                .arg(url)
                .spawn();

            if let Err(e) = result {
                error!("Failed to open URL '{}': {}", url, e);
            }
        }

        Action::Keystroke { keys } => {
            info!("Sending keystroke: {}", keys);
            let script = format!(
                r#"tell application "System Events" to keystroke "{}""#,
                keys
            );
            let result = Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .spawn();

            if let Err(e) = result {
                error!("Failed to send keystroke '{}': {}", keys, e);
            }
        }

        // Page switching is handled by the caller
        Action::SwitchPage { .. } => {}

        Action::Neewer { command: cmd, .. } => {
            info!("Neewer command: {}", cmd);
        }

        Action::Camera { command: cmd } => {
            info!("Camera command: {}", cmd);
        }

        Action::Audio { command: cmd } => {
            info!("Audio command: {}", cmd);
        }

        Action::Sound { name } => {
            info!("Sound: {}", name);
        }

        Action::Timer { command: cmd } => {
            info!("Timer command: {}", cmd);
        }

        Action::LightPreset { brightness, temp_k, .. } => {
            info!("Light preset: brightness={}, temp={}K", brightness, temp_k);
        }

        Action::Multi { actions } => {
            for action in actions {
                execute(action);
            }
        }
    }
}
