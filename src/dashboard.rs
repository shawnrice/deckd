use std::collections::HashSet;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, warn};

use crate::config::MonitoringConfig;
use crate::sysmon;

/// Shared dashboard state updated by the background poller
#[derive(Debug, Clone)]
pub struct DashboardState {
    // Audio
    pub volume: String,
    pub audio_output: String,
    pub audio_input: String,
    pub mic_muted: bool,

    // GitHub
    pub my_pr_count: u32,
    pub review_requests: u32,
    pub mergeable_count: u32,
    pub my_issue_count: u32,
    pub latest_pr_title: String,
    pub latest_pr_status: String,
    pub latest_pr_checks: String,

    // Calendar
    pub next_meeting: String,      // Meeting title or "—"
    pub next_meeting_mins: i32,    // Minutes until next meeting (-1 = none)

    // Now playing
    pub now_playing_title: String,
    pub now_playing_artist: String,
    pub now_playing_state: String,  // "playing", "paused", "none"
    pub now_playing_changed: Instant, // when state last changed

    // Meeting detection
    pub in_meeting: bool,
    pub meeting_changed: bool,

    // Timestamps
    pub audio_updated: Instant,
    pub github_updated: Instant,

    // Suppress background audio poll until this time (after optimistic updates)
    pub audio_suppress_until: Instant,

    // Flash input device name on LCD for a brief period after switching
    pub input_flash: Option<(String, Instant)>,

    // Notifications — set when values change, cleared after display
    pub notifications: Vec<Notification>,

    // CI failure tracking — PR numbers we've already notified about
    pub notified_ci_failures: HashSet<u32>,

    // System monitoring
    pub cpu_load: Option<f32>,
    pub memory_percent: Option<u8>,
    pub containers_running: Option<u32>,
    pub containers_unhealthy: Vec<String>,
    pub network_latency_ms: Option<u32>,
    pub uptime_hours: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub created: Instant,
}

/// How long a notification stays "active" on the LCD banner before it fades out.
pub const NOTIFICATION_DURATION: Duration = Duration::from_secs(5);

impl DashboardState {
    /// True when the most recent notification is still within the banner display window.
    pub fn notification_active(&self) -> bool {
        self.notifications
            .last()
            .is_some_and(|n| n.created.elapsed() < NOTIFICATION_DURATION)
    }

    pub fn new() -> Self {
        Self {
            volume: "?".into(),
            audio_output: "?".into(),
            audio_input: "?".into(),
            mic_muted: false,
            my_pr_count: 0,
            review_requests: 0,
            mergeable_count: 0,
            my_issue_count: 0,
            latest_pr_title: String::new(),
            latest_pr_status: String::new(),
            latest_pr_checks: String::new(),
            next_meeting: "—".into(),
            next_meeting_mins: -1,
            now_playing_title: String::new(),
            now_playing_artist: String::new(),
            now_playing_state: "none".into(),
            now_playing_changed: Instant::now(),
            in_meeting: false,
            meeting_changed: false,
            audio_updated: Instant::now(),
            github_updated: Instant::now(),
            audio_suppress_until: Instant::now(),
            input_flash: None,
            notifications: Vec::new(),
            notified_ci_failures: HashSet::new(),
            cpu_load: None,
            memory_percent: None,
            containers_running: None,
            containers_unhealthy: Vec::new(),
            network_latency_ms: None,
            uptime_hours: None,
        }
    }
}

pub type SharedDashboard = Arc<Mutex<DashboardState>>;

pub fn new_shared() -> SharedDashboard {
    Arc::new(Mutex::new(DashboardState::new()))
}

/// Optimistically adjust volume in the dashboard state without re-polling.
/// Suppresses background audio poll for 1.5s so it doesn't overwrite.
/// Force an immediate meeting state check
pub fn check_meeting(state: &SharedDashboard) {
    poll_in_meeting(state);
}

/// Force an immediate audio state re-poll
pub fn refresh_audio(state: &SharedDashboard) {
    // Temporarily clear suppress so poll_audio runs
    if let Ok(mut s) = state.lock() {
        s.audio_suppress_until = Instant::now();
    }
    poll_audio(state);
}

pub fn nudge_volume(state: &SharedDashboard, delta: i32) {
    if let Ok(mut s) = state.lock() {
        let current: i32 = s.volume.parse().unwrap_or(50);
        let new_vol = (current + delta).clamp(0, 100);
        s.volume = new_vol.to_string();
        s.audio_suppress_until = Instant::now() + Duration::from_millis(1500);
    }
}

/// Optimistically set mic mute state
pub fn set_mic_muted(state: &SharedDashboard, muted: bool) {
    if let Ok(mut s) = state.lock() {
        s.mic_muted = muted;
        s.audio_suppress_until = Instant::now() + Duration::from_millis(1500);
    }
}

/// Mark that the audio output device changed
pub fn mark_audio_changed(state: &SharedDashboard) {
    if let Ok(mut s) = state.lock() {
        s.audio_output = "...".into();
        // Don't suppress — we want the poller to pick up the new device name
    }
}

/// Start the background dashboard poller thread
pub fn start_poller(state: SharedDashboard, github_repo: Option<String>, monitoring: MonitoringConfig) {
    thread::spawn(move || {
        let boot_time = Instant::now();

        // Immediate poll (except sysmon — skip during boot spike)
        poll_audio(&state);
        poll_github(&state, github_repo.as_deref());
        poll_calendar(&state);

        let mut last_audio = Instant::now();
        let mut last_meeting_detect = Instant::now();
        let mut last_github = Instant::now();
        let mut last_meeting = Instant::now();
        let mut last_sysmon = Instant::now();
        let mut last_containers = Instant::now();
        let mut last_network = Instant::now();

        loop {
            thread::sleep(Duration::from_secs(2));

            if last_audio.elapsed() >= Duration::from_secs(3) {
                poll_audio(&state);
                poll_now_playing(&state);
                last_audio = Instant::now();
            }

            // Meeting detection: every 15 seconds (osascript is heavy)
            if last_meeting_detect.elapsed() >= Duration::from_secs(15) {
                poll_in_meeting(&state);
                last_meeting_detect = Instant::now();
            }

            // Calendar: poll every 60 seconds via Google Calendar API (no UI, no focus stealing)
            if last_meeting.elapsed() >= Duration::from_secs(60) {
                poll_calendar(&state);
                last_meeting = Instant::now();
            }

            // GitHub: poll every 90 seconds
            if last_github.elapsed() >= Duration::from_secs(90) {
                poll_github(&state, github_repo.as_deref());
                last_github = Instant::now();
            }

            // System stats: every 10 seconds (lightweight)
            if last_sysmon.elapsed() >= Duration::from_secs(10) {
                poll_sysmon(&state, &monitoring, boot_time);
                last_sysmon = Instant::now();
            }

            // Containers: every 30 seconds
            if last_containers.elapsed() >= Duration::from_secs(30) {
                poll_containers(&state, &monitoring);
                last_containers = Instant::now();
            }

            // Network ping: every 30 seconds
            if last_network.elapsed() >= Duration::from_secs(30) {
                poll_network(&state, &monitoring);
                last_network = Instant::now();
            }
        }
    });
}

fn poll_audio(state: &SharedDashboard) {
    // Skip if an optimistic update is still fresh — avoids overwriting local state
    if let Ok(s) = state.lock()
        && Instant::now() < s.audio_suppress_until
    {
        return;
    }
    let volume = Command::new("osascript")
        .arg("-e")
        .arg("output volume of (get volume settings)")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".into());

    let audio_output = Command::new("SwitchAudioSource")
        .args(["-c", "-t", "output"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".into());

    let audio_input = Command::new("SwitchAudioSource")
        .args(["-c", "-t", "input"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".into());

    let input_vol = Command::new("osascript")
        .arg("-e")
        .arg("input volume of (get volume settings)")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(50);

    let mic_muted = input_vol == 0;

    if let Ok(mut s) = state.lock() {
        s.volume = volume;
        s.audio_output = audio_output;
        s.audio_input = audio_input;
        s.mic_muted = mic_muted;
        s.audio_updated = Instant::now();
    }
}

fn poll_github(state: &SharedDashboard, repo: Option<&str>) {
    let repo = match repo {
        Some(r) => r,
        None => return, // No repo configured, skip GitHub polling
    };
    debug!("Polling GitHub via GraphQL...");

    // Single combined GraphQL query — fast, avoids timeouts from gh pr list
    let query = format!(r#"query {{
  myOpenPRs: search(query: "repo:{repo} is:pr is:open author:@me", type: ISSUE, first: 10) {{
    issueCount
    nodes {{
      ... on PullRequest {{
        number
        title
        state
        isDraft
        reviewDecision
        commits(last: 1) {{
          nodes {{
            commit {{
              statusCheckRollup {{
                state
              }}
            }}
          }}
        }}
      }}
    }}
  }}
  reviewRequested: search(query: "repo:{repo} is:pr is:open review-requested:@me", type: ISSUE, first: 0) {{
    issueCount
  }}
  approvedPRs: search(query: "repo:{repo} is:pr is:open author:@me review:approved", type: ISSUE, first: 100) {{
    issueCount
    nodes {{
      ... on PullRequest {{
        number
        commits(last: 1) {{
          nodes {{
            commit {{
              statusCheckRollup {{
                state
              }}
            }}
          }}
        }}
      }}
    }}
  }}
  myIssues: search(query: "repo:{repo} is:issue is:open assignee:@me", type: ISSUE, first: 0) {{
    issueCount
  }}
}}"#, repo = repo);

    let jq_filter = r#"{
  openPRCount: .data.myOpenPRs.issueCount,
  reviewRequestCount: .data.reviewRequested.issueCount,
  approvedCount: .data.approvedPRs.issueCount,
  mergeableCount: ([.data.approvedPRs.nodes[] | select(.commits.nodes[0].commit.statusCheckRollup.state == "SUCCESS")] | length),
  issueCount: .data.myIssues.issueCount,
  latestTitle: (.data.myOpenPRs.nodes[0].title // ""),
  latestStatus: (
    if .data.myOpenPRs.nodes[0].isDraft then "draft"
    elif .data.myOpenPRs.nodes[0].reviewDecision == "APPROVED" then "approved"
    elif .data.myOpenPRs.nodes[0].reviewDecision == "CHANGES_REQUESTED" then "changes"
    else "review"
    end
  ),
  latestChecks: (.data.myOpenPRs.nodes[0].commits.nodes[0].commit.statusCheckRollup.state // "unknown"),
  ciFailures: [.data.myOpenPRs.nodes[] | select(.commits.nodes[0].commit.statusCheckRollup.state == "FAILURE") | {number, title}],
  ciSuccesses: [.data.myOpenPRs.nodes[] | select(.commits.nodes[0].commit.statusCheckRollup.state == "SUCCESS") | {number}]
}"#;

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={}", query), "--jq", jq_filter])
        .output();

    let json = match output {
        Ok(o) if o.status.success() => {
            String::from_utf8(o.stdout).unwrap_or_default()
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!("GitHub GraphQL query failed: {}", stderr.trim());
            return;
        }
        Err(e) => {
            warn!("Failed to run gh: {}", e);
            return;
        }
    };

    // Parse the JSON response
    let open_prs = extract_json_number(&json, "openPRCount").unwrap_or(0);
    let review_requests = extract_json_number(&json, "reviewRequestCount").unwrap_or(0);
    let mergeable = extract_json_number(&json, "mergeableCount").unwrap_or(0);
    let issues = extract_json_number(&json, "issueCount").unwrap_or(0);
    let latest_title = extract_json_string(&json, "latestTitle").unwrap_or_default();
    let latest_status = extract_json_string(&json, "latestStatus").unwrap_or_default();
    let latest_checks = extract_json_string(&json, "latestChecks").unwrap_or_default();

    // Parse CI failures and successes for notification tracking
    let ci_failures = extract_json_pr_list(&json, "ciFailures");
    let ci_successes = extract_json_pr_numbers(&json, "ciSuccesses");

    if let Ok(mut s) = state.lock() {
        // CI failure notifications — only notify once per PR
        for (pr_num, pr_title) in &ci_failures {
            if !s.notified_ci_failures.contains(pr_num) {
                s.notified_ci_failures.insert(*pr_num);
                s.notifications.push(Notification {
                    message: format!("CI failing on #{}: {}", pr_num, truncate_str(pr_title, 30)),
                    created: Instant::now(),
                });
            }
        }

        // CI fixed notifications — when a previously-failing PR is now SUCCESS
        let failure_numbers: HashSet<u32> = ci_failures.iter().map(|(n, _)| *n).collect();
        let fixed: Vec<u32> = s.notified_ci_failures
            .iter()
            .filter(|n| ci_successes.contains(n) && !failure_numbers.contains(n))
            .copied()
            .collect();
        for pr_num in &fixed {
            s.notified_ci_failures.remove(pr_num);
            s.notifications.push(Notification {
                message: format!("CI fixed on #{}", pr_num),
                created: Instant::now(),
            });
        }

        // Detect changes and create notifications
        if review_requests > s.review_requests && s.review_requests > 0 {
            let delta = review_requests - s.review_requests;
            let msg = if delta == 1 {
                "New review requested".into()
            } else {
                format!("{} new reviews requested", delta)
            };
            s.notifications.push(Notification {
                message: msg,
                created: Instant::now(),
            });
        }

        if review_requests < s.review_requests && s.review_requests > 0 {
            let completed = s.review_requests - review_requests;
            let msg = if completed == 1 {
                "Review completed!".into()
            } else {
                format!("{} reviews completed!", completed)
            };
            s.notifications.push(Notification {
                message: msg,
                created: Instant::now(),
            });
        }

        if mergeable > s.mergeable_count && s.mergeable_count > 0 {
            let delta = mergeable - s.mergeable_count;
            let msg = if delta == 1 {
                "PR ready to merge!".into()
            } else {
                format!("{} PRs ready to merge!", delta)
            };
            s.notifications.push(Notification {
                message: msg,
                created: Instant::now(),
            });
        }

        if s.latest_pr_status != latest_status && !s.latest_pr_status.is_empty() {
            if latest_status == "approved" {
                s.notifications.push(Notification {
                    message: "Your PR was approved!".into(),
                    created: Instant::now(),
                });
            } else if latest_status == "changes" {
                s.notifications.push(Notification {
                    message: "Changes requested on PR".into(),
                    created: Instant::now(),
                });
            }
        }

        s.my_pr_count = open_prs;
        s.review_requests = review_requests;
        s.mergeable_count = mergeable;
        s.my_issue_count = issues;
        s.latest_pr_title = latest_title;
        s.latest_pr_status = latest_status;
        s.latest_pr_checks = latest_checks;
        s.github_updated = Instant::now();
    }

    info!(
        "GitHub: {} open PRs, {} reviews, {} mergeable",
        open_prs, review_requests, mergeable
    );
}

// Simple JSON field extraction (avoids serde_json dependency)
fn extract_json_number(json: &str, key: &str) -> Option<u32> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let start = json.find(&pattern)? + pattern.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

fn poll_now_playing(state: &SharedDashboard) {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"try
    tell application "System Events"
        if exists process "Spotify" then
            tell application "Spotify"
                if player state is playing then
                    return "playing|" & name of current track & "|" & artist of current track
                else if player state is paused then
                    return "paused|" & name of current track & "|" & artist of current track
                end if
            end tell
        end if
    end tell
end try
return "none||""#)
        .output();

    if let Ok(o) = output
        && let Ok(text) = String::from_utf8(o.stdout)
    {
        let parts: Vec<&str> = text.trim().splitn(3, '|').collect();
        if parts.len() == 3
            && let Ok(mut s) = state.lock()
        {
            let new_state = parts[0].to_string();
            if new_state != s.now_playing_state {
                s.now_playing_changed = Instant::now();
            }
            s.now_playing_state = new_state;
            s.now_playing_title = parts[1].to_string();
            s.now_playing_artist = parts[2].to_string();
        }
    }
}

fn poll_in_meeting(state: &SharedDashboard) {
    // Check Zoom: look for a "Meeting" or "Webinar" window
    let zoom_meeting = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events"
    if exists process "zoom.us" then
        set wins to name of every window of process "zoom.us"
        repeat with w in wins
            if w contains "Meeting" or w contains "Webinar" then return true
        end repeat
    end if
    return false
end tell"#)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "true")
        .unwrap_or(false);

    // Check Google Meet: look for a Chrome tab with meet.google.com
    // (only if Zoom isn't active, to avoid double-detection)
    let meet = if !zoom_meeting {
        Command::new("osascript")
            .arg("-e")
            .arg(r#"tell application "System Events"
    if exists process "Google Chrome" then
        tell application "Google Chrome"
            repeat with w in windows
                repeat with t in tabs of w
                    if URL of t contains "meet.google.com" and URL of t does not contain "meet.google.com/landing" then return true
                end repeat
            end repeat
        end tell
    end if
    return false
end tell"#)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "true")
            .unwrap_or(false)
    } else {
        false
    };

    let in_meeting = zoom_meeting || meet;

    if let Ok(mut s) = state.lock() {
        if s.in_meeting != in_meeting {
            s.meeting_changed = true;
            if in_meeting {
                info!("Meeting detected! ({})", if zoom_meeting { "Zoom" } else { "Google Meet" });
            } else {
                info!("Meeting ended");
            }
        }
        s.in_meeting = in_meeting;
    }
}

/// Truncate a string (for notification messages)
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}...", truncated)
    }
}

/// Extract a list of (number, title) pairs from a JSON array like `"ciFailures": [{...}, ...]`
fn extract_json_pr_list(json: &str, key: &str) -> Vec<(u32, String)> {
    let pattern = format!("\"{}\":", key);
    let Some(start) = json.find(&pattern) else { return Vec::new() };
    let rest = &json[start + pattern.len()..];
    let Some(arr_start) = rest.find('[') else { return Vec::new() };
    let rest = &rest[arr_start..];

    // Find matching closing bracket
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return Vec::new();
    }
    let arr_str = &rest[..end];

    // Simple extraction of {number, title} objects
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(obj_start) = arr_str[pos..].find('{') {
        let abs_start = pos + obj_start;
        if let Some(obj_end) = arr_str[abs_start..].find('}') {
            let obj = &arr_str[abs_start..abs_start + obj_end + 1];
            let num = extract_json_number(obj, "number");
            let title = extract_json_string(obj, "title").unwrap_or_default();
            if let Some(n) = num {
                results.push((n, title));
            }
            pos = abs_start + obj_end + 1;
        } else {
            break;
        }
    }
    results
}

/// Extract PR numbers from a JSON array like `"ciSuccesses": [{"number":123}, ...]`
fn extract_json_pr_numbers(json: &str, key: &str) -> HashSet<u32> {
    extract_json_pr_list(json, key)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

fn poll_sysmon(state: &SharedDashboard, monitoring: &MonitoringConfig, boot_time: Instant) {
    if monitoring.system_stats.unwrap_or(false) {
        let cpu = sysmon::poll_cpu_load();
        let mem = sysmon::poll_memory();
        let uptime = sysmon::poll_uptime();

        if let Ok(mut s) = state.lock() {
            // Notify when load average exceeds 1.5x core count (sustained overload,
            // not just normal full utilization). Skip first 60s for wake spike.
            let ncpu = sysmon::cpu_count() as f32;
            let cpu_threshold = ncpu * 1.5;
            if let Some(load) = cpu {
                if load > cpu_threshold
                    && boot_time.elapsed() > Duration::from_secs(60)
                    && s.cpu_load.map(|prev| prev <= cpu_threshold).unwrap_or(true)
                {
                    s.notifications.push(Notification {
                        message: format!("Load avg {:.1} ({}x cores)", load, (load / ncpu).round() as u32),
                        created: Instant::now(),
                    });
                }
            }
            s.cpu_load = cpu;
            s.memory_percent = mem;
            s.uptime_hours = uptime;
        }
    }
}

fn poll_containers(state: &SharedDashboard, monitoring: &MonitoringConfig) {
    if !monitoring.containers.unwrap_or(false) {
        return;
    }

    let (running, unhealthy) = sysmon::poll_containers();

    if let Ok(mut s) = state.lock() {
        // Notify on newly unhealthy containers
        for name in &unhealthy {
            if !s.containers_unhealthy.contains(name) {
                s.notifications.push(Notification {
                    message: format!("Container unhealthy: {}", name),
                    created: Instant::now(),
                });
            }
        }
        s.containers_running = running;
        s.containers_unhealthy = unhealthy;
    }
}

fn poll_network(state: &SharedDashboard, monitoring: &MonitoringConfig) {
    let Some(ref host) = monitoring.network_ping else {
        return;
    };

    let latency = sysmon::poll_network_latency(host);

    if let Ok(mut s) = state.lock() {
        // Notify when network goes down or comes back
        let was_up = s.network_latency_ms.is_some();
        let is_up = latency.is_some();

        if was_up && !is_up {
            s.notifications.push(Notification {
                message: "Network down!".into(),
                created: Instant::now(),
            });
        } else if !was_up && is_up && s.network_latency_ms.is_none() {
            // Only notify recovery if we previously had a value (not first poll)
            // This is intentionally skipping first-poll recovery
        }

        s.network_latency_ms = latency;
    }
}

fn poll_calendar(state: &SharedDashboard) {
    if !crate::gcal::is_configured() {
        return;
    }

    match crate::gcal::next_events(3) {
        Ok(events) => {
            if let Ok(mut s) = state.lock() {
                if let Some((title, mins)) = events.first() {
                    s.next_meeting = title.clone();
                    s.next_meeting_mins = *mins;
                } else {
                    s.next_meeting = "—".into();
                    s.next_meeting_mins = -1;
                }
            }
        }
        Err(e) => {
            debug!("Calendar poll failed: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_number_valid() {
        let json = r#"{"openPRCount":5,"reviewRequestCount":2}"#;
        assert_eq!(extract_json_number(json, "openPRCount"), Some(5));
        assert_eq!(extract_json_number(json, "reviewRequestCount"), Some(2));
    }

    #[test]
    fn extract_json_number_zero() {
        let json = r#"{"count":0}"#;
        assert_eq!(extract_json_number(json, "count"), Some(0));
    }

    #[test]
    fn extract_json_number_missing_key() {
        let json = r#"{"count":5}"#;
        assert_eq!(extract_json_number(json, "missing"), None);
    }

    #[test]
    fn extract_json_number_non_numeric_value() {
        let json = r#"{"count":"abc"}"#;
        assert_eq!(extract_json_number(json, "count"), None);
    }

    #[test]
    fn extract_json_string_valid() {
        let json = r#"{"latestTitle":"Fix bug","latestStatus":"approved"}"#;
        assert_eq!(extract_json_string(json, "latestTitle").unwrap(), "Fix bug");
        assert_eq!(extract_json_string(json, "latestStatus").unwrap(), "approved");
    }

    #[test]
    fn extract_json_string_empty() {
        let json = r#"{"title":""}"#;
        assert_eq!(extract_json_string(json, "title").unwrap(), "");
    }

    #[test]
    fn extract_json_string_missing_key() {
        let json = r#"{"title":"hello"}"#;
        assert_eq!(extract_json_string(json, "missing"), None);
    }

    #[test]
    fn extract_github_graphql_response() {
        let json = r#"{"openPRCount":3,"reviewRequestCount":1,"approvedCount":2,"mergeableCount":1,"issueCount":5,"latestTitle":"Add feature X","latestStatus":"review","latestChecks":"SUCCESS"}"#;
        assert_eq!(extract_json_number(json, "openPRCount"), Some(3));
        assert_eq!(extract_json_number(json, "mergeableCount"), Some(1));
        assert_eq!(extract_json_string(json, "latestTitle").unwrap(), "Add feature X");
        assert_eq!(extract_json_string(json, "latestChecks").unwrap(), "SUCCESS");
    }
}
