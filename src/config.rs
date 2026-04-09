use serde::Deserialize;
use std::collections::{HashMap, HashSet};
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
    /// Resolve buttons by walking the layer stack bottom-to-top.
    /// Higher layers override lower layers per-position.
    pub fn resolved_buttons(&self, stack: &[String]) -> HashMap<String, ButtonConfig> {
        let mut merged = self.buttons.clone(); // top-level as base
        for page_name in stack {
            if let Some(page) = self.pages.get(page_name) {
                for (key, btn) in &page.buttons {
                    merged.insert(key.clone(), btn.clone());
                }
            }
        }
        merged
    }

    /// Resolve encoders by walking the layer stack bottom-to-top.
    /// Higher layers override lower layers per-position.
    pub fn resolved_encoders(&self, stack: &[String]) -> HashMap<String, EncoderConfig> {
        let mut merged = self.encoders.clone(); // top-level as base
        for page_name in stack {
            if let Some(page) = self.pages.get(page_name) {
                for (key, enc) in &page.encoders {
                    merged.insert(key.clone(), enc.clone());
                }
            }
        }
        merged
    }

    /// Returns encoder positions defined by overlay layers (everything above the base).
    /// Used for LCD dispatch: overlay positions get static labels, base positions get
    /// live dashboard content.
    pub fn overlay_encoder_positions(&self, stack: &[String]) -> HashSet<String> {
        let mut positions = HashSet::new();
        // Skip the base layer (first element), collect labelled encoders from overlays.
        // Unlabelled overlay encoders fall through to the dashboard for that segment.
        for page_name in stack.iter().skip(1) {
            if let Some(page) = self.pages.get(page_name) {
                for (key, enc) in &page.encoders {
                    if enc.label.is_some() {
                        positions.insert(key.clone());
                    }
                }
            }
        }
        positions
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

    #[serde(rename = "ble_scan")]
    BleScan,

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
    fn resolved_buttons_falls_back_to_top_level() {
        let toml = r#"
            [buttons.b1]
            label = "Top"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let stack = vec!["nonexistent_page".into()];
        let btns = cfg.resolved_buttons(&stack);
        assert!(btns.contains_key("b1"));
        assert_eq!(btns["b1"].label.as_deref(), Some("Top"));
    }

    #[test]
    fn resolved_buttons_overlay_overrides_base() {
        let toml = r#"
            [buttons.b1]
            label = "Top"

            [pages.main.buttons.b1]
            label = "Base"

            [pages.tools.buttons.b1]
            label = "Overlay"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let stack = vec!["main".into(), "tools".into()];
        let btns = cfg.resolved_buttons(&stack);
        assert_eq!(btns["b1"].label.as_deref(), Some("Overlay"));
    }

    #[test]
    fn resolved_buttons_fallthrough_to_lower_layer() {
        let toml = r#"
            [pages.main.buttons.b1]
            label = "Base"

            [pages.main.buttons.b2]
            label = "Base B2"

            [pages.tools.buttons.b1]
            label = "Overlay"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let stack = vec!["main".into(), "tools".into()];
        let btns = cfg.resolved_buttons(&stack);
        // b1 comes from tools (overlay), b2 falls through to main (base)
        assert_eq!(btns["b1"].label.as_deref(), Some("Overlay"));
        assert_eq!(btns["b2"].label.as_deref(), Some("Base B2"));
    }

    #[test]
    fn resolved_encoders_falls_back_when_page_empty() {
        let toml = r#"
            [encoders.e1]
            label = "Vol"

            [pages.tools]
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let stack = vec!["tools".into()];
        let enc = cfg.resolved_encoders(&stack);
        assert!(enc.contains_key("e1"));
    }

    #[test]
    fn overlay_encoder_positions_skips_base() {
        let toml = r#"
            [pages.main.encoders.0]
            label = "Vol"
            [pages.main.encoders.1]
            label = "Audio"
            [pages.main.encoders.2]
            label = "Light"
            [pages.main.encoders.3]
            label = "Temp"

            [pages.keylights.encoders.0]
            label = "Brightness"
            [pages.keylights.encoders.1]
            label = "Color"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let stack = vec!["main".into(), "keylights".into()];
        let overlay = cfg.overlay_encoder_positions(&stack);
        assert!(overlay.contains("0"));
        assert!(overlay.contains("1"));
        assert!(!overlay.contains("2"));
        assert!(!overlay.contains("3"));
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
