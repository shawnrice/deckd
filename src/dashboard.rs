use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, warn};

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
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub created: Instant,
}

impl DashboardState {
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
            in_meeting: false,
            meeting_changed: false,
            audio_updated: Instant::now(),
            github_updated: Instant::now(),
            audio_suppress_until: Instant::now(),
            input_flash: None,
            notifications: Vec::new(),
        }
    }
}

pub type SharedDashboard = Arc<Mutex<DashboardState>>;

pub fn new_shared() -> SharedDashboard {
    Arc::new(Mutex::new(DashboardState::new()))
}

/// Optimistically adjust volume in the dashboard state without re-polling.
/// Suppresses background audio poll for 1.5s so it doesn't overwrite.
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
pub fn start_poller(state: SharedDashboard) {
    thread::spawn(move || {
        // Immediate poll
        poll_audio(&state);
        poll_github(&state);
        poll_calendar(&state);

        let mut last_audio = Instant::now();
        let mut last_github = Instant::now();
        let mut last_meeting = Instant::now();

        loop {
            thread::sleep(Duration::from_secs(2));

            if last_audio.elapsed() >= Duration::from_secs(3) {
                poll_audio(&state);
                poll_now_playing(&state);
                poll_in_meeting(&state);
                last_audio = Instant::now();
            }

            // Calendar: poll every 60 seconds via Google Calendar API (no UI, no focus stealing)
            if last_meeting.elapsed() >= Duration::from_secs(60) {
                poll_calendar(&state);
                last_meeting = Instant::now();
            }

            // GitHub: poll every 90 seconds
            if last_github.elapsed() >= Duration::from_secs(90) {
                poll_github(&state);
                last_github = Instant::now();
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

fn poll_github(state: &SharedDashboard) {
    debug!("Polling GitHub via GraphQL...");

    // Single combined GraphQL query — fast, avoids timeouts from gh pr list
    let query = r#"query {
  myOpenPRs: search(query: "repo:shawnrice/deckd is:pr is:open author:@me", type: ISSUE, first: 1) {
    issueCount
    nodes {
      ... on PullRequest {
        number
        title
        state
        isDraft
        reviewDecision
        commits(last: 1) {
          nodes {
            commit {
              statusCheckRollup {
                state
              }
            }
          }
        }
      }
    }
  }
  reviewRequested: search(query: "repo:shawnrice/deckd is:pr is:open review-requested:@me", type: ISSUE, first: 0) {
    issueCount
  }
  approvedPRs: search(query: "repo:shawnrice/deckd is:pr is:open author:@me review:approved", type: ISSUE, first: 100) {
    issueCount
    nodes {
      ... on PullRequest {
        number
        commits(last: 1) {
          nodes {
            commit {
              statusCheckRollup {
                state
              }
            }
          }
        }
      }
    }
  }
  myIssues: search(query: "repo:shawnrice/deckd is:issue is:open assignee:@me", type: ISSUE, first: 0) {
    issueCount
  }
}"#;

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
  latestChecks: (.data.myOpenPRs.nodes[0].commits.nodes[0].commit.statusCheckRollup.state // "unknown")
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

    if let Ok(mut s) = state.lock() {
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
            s.now_playing_state = parts[0].to_string();
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
