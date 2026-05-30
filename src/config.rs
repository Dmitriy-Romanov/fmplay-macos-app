use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub enum AppEvent {
    CatalogLoaded(String),
    SetWidth(f64),
    SetTitle(String),
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default)]
    pub last_station: Option<String>,
    #[serde(default)]
    pub quality: Option<u16>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub only_favorites: Option<bool>,
    #[serde(default)]
    pub window_width: Option<f64>,
    #[serde(default)]
    pub window_height: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct IpcMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub favorites: Option<Vec<String>>,
    pub last_station: Option<String>,
    pub quality: Option<u16>,
    pub volume: Option<f64>,
    pub theme: Option<String>,
    pub only_favorites: Option<bool>,
    pub window_width: Option<f64>,
    pub window_height: Option<f64>,
    pub target_width: Option<f64>,
    pub title: Option<String>,
}

pub fn app_config_path() -> AppResult<PathBuf> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("FMPLAY Radio")
        .join("config.json"))
}

pub fn load_config(path: &PathBuf) -> AppResult<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

pub fn save_config(path: &PathBuf, config: &AppConfig) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(config)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn handle_ipc_message(message: &str, config_path: &PathBuf) -> AppResult<()> {
    let message: IpcMessage = serde_json::from_str(message)?;
    if message.message_type == "config" || message.message_type == "favorites" {
        let mut config = load_config(config_path).unwrap_or_default();
        if let Some(favorites) = message.favorites {
            config.favorites = favorites;
        }
        if message.last_station.is_some() {
            config.last_station = message.last_station;
        }
        if message.quality.is_some() {
            config.quality = message.quality;
        }
        if message.volume.is_some() {
            config.volume = message.volume;
        }
        if message.theme.is_some() {
            config.theme = message.theme;
        }
        if message.only_favorites.is_some() {
            config.only_favorites = message.only_favorites;
        }
        if message.window_width.is_some() {
            config.window_width = message.window_width;
        }
        if message.window_height.is_some() {
            config.window_height = message.window_height;
        }
        save_config(config_path, &config)?;
    }

    Ok(())
}
