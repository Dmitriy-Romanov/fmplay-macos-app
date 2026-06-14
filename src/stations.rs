use crate::config::AppResult;
use crate::player::Stream;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const STATIONS_URL: &str = "https://fmplay.ru/stations.json";
pub const CACHE_TTL_SECS: u64 = 3600;

#[derive(Debug, Deserialize)]
pub struct StationRaw {
    pub name: String,
    #[serde(default)]
    pub logo: String,
    #[serde(default)]
    pub enabled: String,
    #[serde(default)]
    pub meta: Option<String>,
    #[serde(default)]
    pub xtra_low: Option<String>,
    #[serde(default)]
    pub url_low: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub url_hi: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub logo: String,
    pub meta: bool,
    pub streams: Vec<Stream>,
}

pub fn load_stations() -> AppResult<Vec<Station>> {
    match load_cached_stations() {
        Some(stations) if !is_cache_expired() => return Ok(stations),
        _ => {}
    }

    match load_stations_remote() {
        Ok(mut stations) => {
            stations.sort_by(|a, b| compare_station_names(&a.name, &b.name));
            save_cached_stations(&stations);
            Ok(stations)
        }
        Err(_) => load_cached_stations()
            .ok_or_else(|| "Failed to load stations and no cache available".into()),
    }
}

pub fn is_cache_expired() -> bool {
    let path = match cache_path() {
        Ok(p) => p,
        Err(_) => return true,
    };
    if !path.exists() {
        return true;
    }
    let metadata = match fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return true,
    };
    modified
        .elapsed()
        .map(|e| e.as_secs() > CACHE_TTL_SECS)
        .unwrap_or(true)
}

fn load_stations_remote() -> AppResult<Vec<Station>> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            STATIONS_URL,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to download stations.json: {stderr}").into());
    }

    let raw: BTreeMap<String, StationRaw> = serde_json::from_slice(&output.stdout)?;
    let mut stations = parse_stations(raw);
    stations.sort_by(|a, b| compare_station_names(&a.name, &b.name));
    Ok(stations)
}

pub fn parse_stations(raw: BTreeMap<String, StationRaw>) -> Vec<Station> {
    raw.into_iter()
        .filter_map(|(id, raw)| {
            if raw.enabled != "1" {
                return None;
            }

            let mut streams = Vec::new();
            if let Some(url) = raw.xtra_low.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "16".to_owned(),
                    bitrate: 16,
                    url,
                });
            }
            if let Some(url) = raw.url_low.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "24".to_owned(),
                    bitrate: 24,
                    url,
                });
            }
            if let Some(url) = raw.url.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "32".to_owned(),
                    bitrate: 32,
                    url,
                });
            }
            if let Some(url) = raw.url_hi.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "48".to_owned(),
                    bitrate: 48,
                    url,
                });
            }

            if streams.is_empty() {
                return None;
            }

            Some(Station {
                id,
                name: raw.name,
                logo: crate::player::absolute_url(&raw.logo),
                meta: raw.meta.as_deref() == Some("meta"),
                streams,
            })
        })
        .collect()
}


pub fn cache_path() -> AppResult<PathBuf> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("FMPLAY Radio")
        .join("stations_cache.json"))
}

pub fn load_cached_stations() -> Option<Vec<Station>> {
    let path = cache_path().ok()?;
    if !path.exists() {
        return None;
    }

    let metadata = fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    let elapsed = modified.elapsed().ok()?.as_secs();

    if elapsed > CACHE_TTL_SECS {
        return None;
    }

    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save_cached_stations(stations: &[Station]) {
    if let Ok(path) = cache_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(content) = serde_json::to_string_pretty(stations) {
            let _ = fs::write(&path, content);
        }
    }
}

pub fn compare_station_names(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}
