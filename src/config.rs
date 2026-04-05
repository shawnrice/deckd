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
