use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub brightness: Option<u8>,
    pub default_page: Option<String>,
    pub pet_name: Option<String>,

    /// Pages where the pet always shows on the LCD
    #[serde(default)]
    pub pet_pages: Vec<String>,

    /// GitHub repo for dashboard (e.g. "shawnrice/deckd")
    pub github_repo: Option<String>,

    /// Camera vendor:product ID (e.g. "046d:0944"). Auto-detects if not set.
    pub camera: Option<String>,

    #[serde(default)]
    pub monitoring: MonitoringConfig,

    #[serde(default)]
    pub output_devices: Vec<AudioDevice>,

    #[serde(default)]
    pub input_devices: Vec<AudioDevice>,

    #[serde(default)]
    pub buttons: HashMap<String, ButtonConfig>,

    #[serde(default)]
    pub encoders: HashMap<String, EncoderConfig>,

    #[serde(default)]
    pub pages: HashMap<String, PageConfig>,
}

impl Config {
    /// Returns the active page's buttons, falling back to top-level buttons
    pub fn active_buttons<'a>(&'a self, page: &str) -> &'a HashMap<String, ButtonConfig> {
        self.pages
            .get(page)
            .map(|p| &p.buttons)
            .unwrap_or(&self.buttons)
    }

    /// Returns the active page's encoders, falling back to top-level encoders
    pub fn active_encoders<'a>(&'a self, page: &str) -> &'a HashMap<String, EncoderConfig> {
        self.pages
            .get(page)
            .map(|p| &p.encoders)
            .filter(|e| !e.is_empty())
            .unwrap_or(&self.encoders)
    }

    pub fn start_page(&self) -> String {
        self.default_page.clone().unwrap_or_else(|| "main".into())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ButtonConfig {
    pub label: Option<String>,
    pub icon: Option<String>,       // Path to image file
    pub icon_name: Option<String>,  // Built-in icon name (e.g. "terminal", "volume", "rocket")
    pub on_press: Option<Action>,
    #[allow(dead_code)]
    pub on_long_press: Option<Action>,
    pub bg_color: Option<String>,
    pub fg_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EncoderConfig {
    pub label: Option<String>,
    pub on_turn_cw: Option<Action>,
    pub on_turn_ccw: Option<Action>,
    pub on_press: Option<Action>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageConfig {
    #[serde(default)]
    pub buttons: HashMap<String, ButtonConfig>,

    #[serde(default)]
    pub encoders: HashMap<String, EncoderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum Action {
    #[serde(rename = "shell")]
    Shell { command: String },

    #[serde(rename = "open")]
    Open { path: String },

    #[serde(rename = "url")]
    Url { url: String },

    #[serde(rename = "keystroke")]
    Keystroke { keys: String },

    #[serde(rename = "page")]
    SwitchPage { page: String },

    #[serde(rename = "neewer")]
    Neewer { command: String, group: Option<String> },

    #[serde(rename = "camera")]
    Camera { command: String },

    #[serde(rename = "audio")]
    Audio { command: String },

    #[serde(rename = "sound")]
    Sound { name: String },

    #[serde(rename = "timer")]
    Timer { command: String },

    #[serde(rename = "light_preset")]
    LightPreset {
        brightness: u8,
        temp_k: u16,
        group: Option<String>,
    },

    #[serde(rename = "multi")]
    Multi { actions: Vec<Action> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioDevice {
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MonitoringConfig {
    /// Enable CPU/memory monitoring
    pub system_stats: Option<bool>,
    /// Enable Docker/Podman container monitoring
    pub containers: Option<bool>,
    /// Host to ping for network latency (e.g. "1.1.1.1")
    pub network_ping: Option<String>,
}

pub fn resolve_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("DECKD_CONFIG") {
        return PathBuf::from(path);
    }

    // Prefer ~/.config/deckd (XDG-style) over macOS ~/Library/Application Support
    let xdg_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config/deckd/config.toml");

    if xdg_path.exists() {
        return xdg_path;
    }

    let app_support_path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("deckd/config.toml");

    if app_support_path.exists() {
        return app_support_path;
    }

    // Default to XDG-style
    xdg_path
}

pub fn load(path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            brightness = 80
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.brightness, Some(80));
        assert!(cfg.buttons.is_empty());
        assert!(cfg.pages.is_empty());
    }

    #[test]
    fn start_page_defaults_to_main() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.start_page(), "main");
    }

    #[test]
    fn start_page_uses_configured_value() {
        let toml = r#"default_page = "tools""#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.start_page(), "tools");
    }

    #[test]
    fn active_buttons_falls_back_to_top_level() {
        let toml = r#"
            [buttons.b1]
            label = "Top"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let btns = cfg.active_buttons("nonexistent_page");
        assert!(btns.contains_key("b1"));
        assert_eq!(btns["b1"].label.as_deref(), Some("Top"));
    }

    #[test]
    fn active_buttons_uses_page_when_present() {
        let toml = r#"
            [buttons.b1]
            label = "Top"

            [pages.tools.buttons.b1]
            label = "Page"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let btns = cfg.active_buttons("tools");
        assert_eq!(btns["b1"].label.as_deref(), Some("Page"));
    }

    #[test]
    fn active_encoders_falls_back_when_page_encoders_empty() {
        let toml = r#"
            [encoders.e1]
            label = "Vol"

            [pages.tools]
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let enc = cfg.active_encoders("tools");
        assert!(enc.contains_key("e1"));
    }

    #[test]
    fn parse_multi_action() {
        let toml = r#"
            [buttons.b1]
            label = "Multi"
            [buttons.b1.on_press]
            type = "multi"
            [[buttons.b1.on_press.actions]]
            type = "shell"
            command = "echo hello"
            [[buttons.b1.on_press.actions]]
            type = "url"
            url = "https://example.com"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let action = cfg.buttons["b1"].on_press.as_ref().unwrap();
        match action {
            Action::Multi { actions } => assert_eq!(actions.len(), 2),
            _ => panic!("expected Multi action"),
        }
    }

    #[test]
    fn parse_light_preset_with_group() {
        let toml = r#"
            [buttons.b1]
            label = "Meeting"
            [buttons.b1.on_press]
            type = "light_preset"
            brightness = 70
            temp_k = 5000
            group = "keylights"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        match cfg.buttons["b1"].on_press.as_ref().unwrap() {
            Action::LightPreset { brightness, temp_k, group } => {
                assert_eq!(*brightness, 70);
                assert_eq!(*temp_k, 5000);
                assert_eq!(group.as_deref(), Some("keylights"));
            }
            _ => panic!("expected LightPreset action"),
        }
    }

    #[test]
    fn parse_audio_device_list() {
        let toml = r#"
            [[output_devices]]
            uid = "BuiltInSpeaker"
            name = "Speakers"

            [[output_devices]]
            uid = "HyperX-1234"
            name = "HyperX"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.output_devices.len(), 2);
        assert_eq!(cfg.output_devices[0].name, "Speakers");
        assert_eq!(cfg.output_devices[1].uid, "HyperX-1234");
    }

    #[test]
    fn parse_monitoring_config_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.monitoring.system_stats, None);
        assert_eq!(cfg.monitoring.containers, None);
        assert_eq!(cfg.monitoring.network_ping, None);
    }

    #[test]
    fn parse_monitoring_config_full() {
        let toml = r#"
            [monitoring]
            system_stats = true
            containers = true
            network_ping = "1.1.1.1"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.monitoring.system_stats, Some(true));
        assert_eq!(cfg.monitoring.containers, Some(true));
        assert_eq!(cfg.monitoring.network_ping, Some("1.1.1.1".into()));
    }

    #[test]
    fn parse_monitoring_config_partial() {
        let toml = r#"
            [monitoring]
            system_stats = true
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.monitoring.system_stats, Some(true));
        assert_eq!(cfg.monitoring.containers, None);
        assert_eq!(cfg.monitoring.network_ping, None);
    }
}
