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
            // macOS `open -u` passes ASCII URLs through untouched, but if the
            // URL contains any non-ASCII byte (e.g. a raw emoji in &title=🌊),
            // Launch Services decides the URL is "unencoded" and re-encodes
            // everything — turning existing %3C into %253C. Pre-encoding any
            // non-ASCII bytes to their UTF-8 %XX form makes the URL fully
            // ASCII, so `open -u` has nothing to re-encode.
            let encoded = encode_non_ascii(url);
            let result = Command::new("open")
                .arg("-u")
                .arg(&encoded)
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

        Action::BleScan => {
            info!("BLE rescan requested");
        }

        Action::Multi { actions } => {
            for action in actions {
                execute(action);
            }
        }
    }
}

/// Percent-encode any non-ASCII bytes in a URL, leaving ASCII (including
/// already-encoded `%XX` sequences) untouched. See the comment in the Url
/// action above for why this is needed.
fn encode_non_ascii(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        if b.is_ascii() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_url_unchanged() {
        let url = "https://example.com/?body=%3C!--%20x%20--%3E";
        assert_eq!(encode_non_ascii(url), url);
    }

    #[test]
    fn emoji_gets_percent_encoded() {
        // 🌊 is U+1F30A, UTF-8: F0 9F 8C 8A
        let url = "https://example.com/?title=🌊";
        assert_eq!(encode_non_ascii(url), "https://example.com/?title=%F0%9F%8C%8A");
    }

    #[test]
    fn mixed_already_encoded_and_emoji() {
        // Existing %3C / %20 are preserved, emoji gets encoded.
        let url = "https://example.com/?body=%3C%20%3E&title=🚢";
        assert_eq!(
            encode_non_ascii(url),
            "https://example.com/?body=%3C%20%3E&title=%F0%9F%9A%A2"
        );
    }
}
