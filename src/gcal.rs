use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use log::info;
use serde_json::Value;

// OAuth client credentials — using gcalcli's public client ID
// These are intentionally public (installed app flow, not secret)
const CLIENT_ID: &str = "232867676714.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "3d871f6bae7e25e4450f22e5d3e2bcc4";
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.readonly";
const REDIRECT_PORT: u16 = 8085;
const REDIRECT_URI: &str = "http://localhost:8085";

fn token_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config/deckd/google_token.json")
}

// ── Token management ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

impl TokenInfo {
    fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= self.expires_at.saturating_sub(60) // refresh 60s before expiry
    }

    fn save(&self) -> Result<(), String> {
        let json = serde_json::json!({
            "access_token": self.access_token,
            "refresh_token": self.refresh_token,
            "expires_at": self.expires_at,
        });
        let path = token_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap())
            .map_err(|e| format!("Failed to save token: {}", e))?;

        // chmod 600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
        }

        Ok(())
    }

    fn load() -> Option<TokenInfo> {
        let content = std::fs::read_to_string(token_path()).ok()?;
        let v: Value = serde_json::from_str(&content).ok()?;
        Some(TokenInfo {
            access_token: v["access_token"].as_str()?.to_string(),
            refresh_token: v["refresh_token"].as_str()?.to_string(),
            expires_at: v["expires_at"].as_u64()?,
        })
    }
}

// ── OAuth browser flow ──────────────────────────────────────────

/// Run the OAuth authorization flow. Opens browser, waits for callback.
/// Called by `deckd auth google`.
pub fn authorize() {
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
        client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        CLIENT_ID,
        urlenc(REDIRECT_URI),
        urlenc(SCOPES),
    );

    println!("Opening browser for Google Calendar authorization...");
    std::process::Command::new("open")
        .arg(&auth_url)
        .spawn()
        .expect("Failed to open browser");

    println!("Waiting for authorization callback on localhost:{}...", REDIRECT_PORT);

    // Start a temporary HTTP server to receive the OAuth callback
    let listener = TcpListener::bind(format!("127.0.0.1:{}", REDIRECT_PORT))
        .expect("Failed to bind callback server");

    let (mut stream, _) = listener.accept().expect("No callback received");

    let mut reader = std::io::BufReader::new(&stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok();

    // Extract the authorization code from: GET /?code=XXXX&scope=... HTTP/1.1
    let code = request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| {
            path.split('?')
                .nth(1)?
                .split('&')
                .find(|p| p.starts_with("code="))
                .map(|p| p.trim_start_matches("code=").to_string())
        });

    // Send a response to the browser
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h2>deckd authorized!</h2><p>You can close this tab.</p></body></html>";
    let stream_ref: &mut std::net::TcpStream = &mut stream;
    stream_ref.write_all(response.as_bytes()).ok();
    stream_ref.flush().ok();
    drop(stream);

    let code = match code {
        Some(c) => c,
        None => {
            eprintln!("Failed to extract authorization code from callback");
            std::process::exit(1);
        }
    };

    println!("Got authorization code, exchanging for token...");

    // Exchange code for tokens
    let body = format!(
        "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
        urlenc(&code), CLIENT_ID, CLIENT_SECRET, urlenc(REDIRECT_URI),
    );

    let resp = ureq::post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(&body);

    match resp {
        Ok(mut resp) => {
            let json: Value = resp.body_mut().read_json::<Value>().unwrap_or_default();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let token = TokenInfo {
                access_token: json["access_token"].as_str().unwrap_or("").to_string(),
                refresh_token: json["refresh_token"].as_str().unwrap_or("").to_string(),
                expires_at: now + json["expires_in"].as_u64().unwrap_or(3600),
            };

            if token.refresh_token.is_empty() {
                eprintln!("No refresh token received. Try revoking access and re-authorizing:");
                eprintln!("  https://myaccount.google.com/permissions");
                std::process::exit(1);
            }

            token.save().expect("Failed to save token");
            println!("Token saved to {}", token_path().display());
            println!("Google Calendar integration is ready!");
        }
        Err(e) => {
            eprintln!("Token exchange failed: {}", e);
            std::process::exit(1);
        }
    }
}

// ── Token refresh ───────────────────────────────────────────────

fn refresh_token(token: &mut TokenInfo) -> Result<(), String> {
    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        CLIENT_ID, CLIENT_SECRET, urlenc(&token.refresh_token),
    );

    let mut resp = ureq::post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(&body)
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    let json: Value = resp.body_mut().read_json::<Value>().unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    token.access_token = json["access_token"]
        .as_str()
        .ok_or("No access_token in refresh response")?
        .to_string();
    token.expires_at = now + json["expires_in"].as_u64().unwrap_or(3600);

    token.save()?;
    info!("Google Calendar token refreshed");
    Ok(())
}

// ── Calendar query ──────────────────────────────────────────────

/// Fetch the next few calendar events. Returns (title, minutes_until_start) pairs.
pub fn next_events(max_results: u32) -> Result<Vec<(String, i32)>, String> {
    let mut token = TokenInfo::load()
        .ok_or("No Google Calendar token — run `deckd auth google`")?;

    if token.is_expired() {
        refresh_token(&mut token)?;
    }

    let now = chrono_now_rfc3339();
    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events?\
        timeMin={}&maxResults={}&singleEvents=true&orderBy=startTime",
        urlenc(&now),
        max_results,
    );

    let mut resp = ureq::get(&url)
        .header("Authorization", &format!("Bearer {}", token.access_token))
        .call()
        .map_err(|e| format!("Calendar API error: {}", e))?;

    let json: Value = resp.body_mut().read_json::<Value>().unwrap_or_default();

    let events = json["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let title = item["summary"].as_str()?.to_string();
                    // Get start time — either dateTime (timed event) or date (all-day)
                    let start_str = item["start"]["dateTime"]
                        .as_str()
                        .or_else(|| item["start"]["date"].as_str())?;
                    let mins = minutes_until(start_str);
                    Some((title, mins))
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(events)
}

/// Check if Google Calendar is configured
pub fn is_configured() -> bool {
    token_path().exists()
}

// ── Helpers ─────────────────────────────────────────────────────

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn chrono_now_rfc3339() -> String {
    // Get current time in RFC3339 format without pulling in the chrono crate
    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "2026-01-01T00:00:00Z".into());
    output.trim().to_string()
}

fn minutes_until(datetime_str: &str) -> i32 {
    // Parse ISO 8601 datetime and compute minutes from now
    // Handles both "2026-04-04T10:00:00-07:00" and "2026-04-04"
    let now_output = std::process::Command::new("date")
        .args(["+%s"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);

    let event_epoch = std::process::Command::new("date")
        .args(["-j", "-f", "%Y-%m-%dT%H:%M:%S%z", datetime_str, "+%s"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        // Try without timezone
        .or_else(|| {
            let trimmed = datetime_str.split('+').next()
                .or_else(|| datetime_str.rsplitn(2, '-').nth(1))?;
            std::process::Command::new("date")
                .args(["-j", "-f", "%Y-%m-%dT%H:%M:%S", trimmed, "+%s"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<i64>().ok())
        })
        .unwrap_or(0);

    ((event_epoch - now_output) / 60) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlenc_leaves_unreserved_chars() {
        assert_eq!(urlenc("hello"), "hello");
        assert_eq!(urlenc("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn urlenc_encodes_special_chars() {
        assert_eq!(urlenc("a b"), "a%20b");
        assert_eq!(urlenc("foo@bar"), "foo%40bar");
        assert_eq!(urlenc("a+b=c"), "a%2Bb%3Dc");
    }

    #[test]
    fn urlenc_encodes_url() {
        let encoded = urlenc("http://localhost:8085");
        assert!(encoded.contains("%3A"));
        assert!(encoded.contains("%2F"));
    }
}
