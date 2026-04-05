use std::process::Command;

use log::debug;

/// System monitoring stats, all optional depending on config
#[derive(Debug, Clone, Default)]
pub struct SystemStats {
    pub cpu_load: Option<f32>,
    pub memory_percent: Option<u8>,
    pub containers_running: Option<u32>,
    pub containers_unhealthy: Vec<String>,
    pub network_latency_ms: Option<u32>,
    pub uptime_hours: Option<u32>,
}

/// Collect CPU load average (1-minute) via sysctl
pub fn poll_cpu_load() -> Option<f32> {
    let output = Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    parse_load_avg(&text)
}

/// Parse macOS `sysctl -n vm.loadavg` output: `{ 1.23 0.89 0.67 }`
pub fn parse_load_avg(text: &str) -> Option<f32> {
    let trimmed = text.trim().trim_start_matches('{').trim_end_matches('}').trim();
    let first = trimmed.split_whitespace().next()?;
    first.parse().ok()
}

/// Collect memory usage percentage via vm_stat + sysctl
pub fn poll_memory() -> Option<u8> {
    let total = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())?;

    let vm_stat = Command::new("vm_stat")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())?;

    let used_pages = parse_vm_stat_used(&vm_stat)?;
    // macOS page size is 16384 on Apple Silicon, 4096 on Intel
    let page_size = Command::new("sysctl")
        .args(["-n", "hw.pagesize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(16384);

    let used_bytes = used_pages * page_size;
    let pct = ((used_bytes as f64 / total as f64) * 100.0) as u8;
    Some(pct.min(100))
}

/// Parse vm_stat output to get used pages (active + wired + speculative + compressor)
pub fn parse_vm_stat_used(text: &str) -> Option<u64> {
    let extract = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.contains(key))
            .and_then(|l| {
                l.split(':')
                    .nth(1)
                    .map(|v| v.trim().trim_end_matches('.'))
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(0)
    };

    let active = extract("Pages active");
    let wired = extract("Pages wired down");
    let speculative = extract("Pages speculative");
    let compressor = extract("Pages occupied by compressor");

    let total = active + wired + speculative + compressor;
    if total > 0 { Some(total) } else { None }
}

/// Poll Docker/Podman container status
pub fn poll_containers() -> (Option<u32>, Vec<String>) {
    let runtime = if Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "docker"
    } else if Command::new("podman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "podman"
    } else {
        debug!("No container runtime found");
        return (None, Vec::new());
    };

    let running = Command::new(runtime)
        .args(["ps", "--format", "{{.Status}}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().filter(|l| !l.is_empty()).count() as u32);

    let unhealthy = Command::new(runtime)
        .args(["ps", "--filter", "health=unhealthy", "--format", "{{.Names}}"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.trim().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (running, unhealthy)
}

/// Ping a host and return round-trip time in ms
pub fn poll_network_latency(host: &str) -> Option<u32> {
    let output = Command::new("ping")
        .args(["-c", "1", "-t", "2", host])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8(output.stdout).ok()?;
    parse_ping_rtt(&text)
}

/// Parse ping output to extract round-trip time
/// macOS format: `round-trip min/avg/max/stddev = 5.123/5.456/5.789/0.123 ms`
pub fn parse_ping_rtt(text: &str) -> Option<u32> {
    let rtt_line = text.lines().find(|l| l.contains("round-trip") || l.contains("rtt"))?;
    let equals_part = rtt_line.split('=').nth(1)?.trim();
    // avg is the second value: min/avg/max/stddev
    let avg_str = equals_part.split('/').nth(1)?;
    let ms: f64 = avg_str.trim().parse().ok()?;
    Some(ms.round() as u32)
}

/// Get system uptime in hours
pub fn poll_uptime() -> Option<u32> {
    let output = Command::new("sysctl")
        .args(["-n", "kern.boottime"])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    parse_boottime(&text)
}

/// Parse `kern.boottime` output: `{ sec = 1712345678, usec = 123456 } ...`
pub fn parse_boottime(text: &str) -> Option<u32> {
    let sec_str = text.split("sec = ").nth(1)?;
    let sec_end = sec_str.find(',')?;
    let boot_sec: u64 = sec_str[..sec_end].trim().parse().ok()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();

    let uptime_secs = now.saturating_sub(boot_sec);
    Some((uptime_secs / 3600) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_load_avg_normal() {
        assert_eq!(parse_load_avg("{ 1.23 0.89 0.67 }"), Some(1.23));
    }

    #[test]
    fn parse_load_avg_high() {
        assert_eq!(parse_load_avg("{ 12.50 8.30 4.10 }"), Some(12.5));
    }

    #[test]
    fn parse_load_avg_zero() {
        assert_eq!(parse_load_avg("{ 0.00 0.01 0.05 }"), Some(0.0));
    }

    #[test]
    fn parse_load_avg_empty() {
        assert_eq!(parse_load_avg(""), None);
    }

    #[test]
    fn parse_load_avg_garbage() {
        assert_eq!(parse_load_avg("not a load average"), None);
    }

    #[test]
    fn parse_vm_stat_used_typical() {
        let input = r#"Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               12345.
Pages active:                            200000.
Pages inactive:                           50000.
Pages speculative:                          500.
Pages throttled:                              0.
Pages wired down:                        100000.
Pages purgeable:                           5000.
"Pages occupied by compressor":           30000.
"#;
        // active=200000 + wired=100000 + speculative=500 + compressor=30000 = 330500
        assert_eq!(parse_vm_stat_used(input), Some(330500));
    }

    #[test]
    fn parse_vm_stat_used_empty() {
        assert_eq!(parse_vm_stat_used(""), None);
    }

    #[test]
    fn parse_ping_rtt_macos() {
        let input = r#"PING 1.1.1.1 (1.1.1.1): 56 data bytes
64 bytes from 1.1.1.1: icmp_seq=0 ttl=55 time=5.432 ms

--- 1.1.1.1 ping statistics ---
1 packets transmitted, 1 packets received, 0.0% packet loss
round-trip min/avg/max/stddev = 5.432/5.432/5.432/0.000 ms"#;
        assert_eq!(parse_ping_rtt(input), Some(5));
    }

    #[test]
    fn parse_ping_rtt_high_latency() {
        let input = "round-trip min/avg/max/stddev = 100.1/150.7/200.3/50.1 ms";
        assert_eq!(parse_ping_rtt(input), Some(151));
    }

    #[test]
    fn parse_ping_rtt_no_match() {
        assert_eq!(parse_ping_rtt("Request timeout"), None);
    }

    #[test]
    fn parse_boottime_valid() {
        // Use a recent timestamp to test (won't be exact, but should be > 0)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let boot = now - 7200; // 2 hours ago
        let input = format!("{{ sec = {}, usec = 123456 }} Thu Apr  3 10:00:00 2025", boot);
        let result = parse_boottime(&input);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 2); // ~2 hours
    }

    #[test]
    fn parse_boottime_garbage() {
        assert_eq!(parse_boottime("not valid"), None);
    }
}
