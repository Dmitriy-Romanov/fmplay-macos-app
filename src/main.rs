use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

const STATIONS_URL: &str = "https://fmplay.ru/stations.json";
const FMPLAY_ROOT: &str = "https://fmplay.ru/";

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

enum AppEvent {
    CatalogLoaded(String),
    SetWidth(f64),
    SetTitle(String),
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct AppConfig {
    #[serde(default)]
    favorites: Vec<String>,
    #[serde(default)]
    last_station: Option<String>,
    #[serde(default)]
    quality: Option<u16>,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    only_favorites: Option<bool>,
    #[serde(default)]
    window_width: Option<f64>,
    #[serde(default)]
    window_height: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct IpcMessage {
    #[serde(rename = "type")]
    message_type: String,
    favorites: Option<Vec<String>>,
    last_station: Option<String>,
    quality: Option<u16>,
    volume: Option<f64>,
    theme: Option<String>,
    only_favorites: Option<bool>,
    window_width: Option<f64>,
    window_height: Option<f64>,
    target_width: Option<f64>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StationRaw {
    name: String,
    #[serde(default)]
    site: String,
    #[serde(default)]
    logo: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    position: String,
    #[serde(default)]
    enabled: String,
    #[serde(default)]
    meta: Option<String>,
    #[serde(default)]
    xtra_low: Option<String>,
    #[serde(default)]
    url_low: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    url_hi: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct Station {
    id: String,
    name: String,
    site: String,
    logo: String,
    category: String,
    position: u32,
    meta: bool,
    streams: Vec<Stream>,
}

#[derive(Debug, serde::Serialize)]
struct Stream {
    label: &'static str,
    bitrate: u16,
    url: String,
}

fn main() -> AppResult<()> {
    let config_path = app_config_path()?;
    let startup_config = load_config(&config_path).unwrap_or_default();
    let ipc_config_path = config_path.clone();
    let loader_config_path = config_path.clone();
    let resize_config_path = config_path.clone();
    let initial_width = startup_config.window_width.unwrap_or(1040.0).max(300.0);
    let initial_height = startup_config.window_height.unwrap_or(744.0).max(744.0);

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let loader_proxy = event_loop.create_proxy();
    let ipc_proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("FMPLAY Radio")
        .with_inner_size(LogicalSize::new(initial_width, initial_height))
        .with_min_inner_size(LogicalSize::new(300.0, 744.0))
        .build(&event_loop)?;

    let webview = WebViewBuilder::new()
        .with_html(build_loading_html())
        .with_initialization_script("window.__FMPLAY_READY__ = true;")
        .with_ipc_handler(move |request| {
            if let Ok(message) = serde_json::from_str::<IpcMessage>(request.body()) {
                if message.message_type == "set_width" {
                    if let Some(width) = message.target_width {
                        let _ = ipc_proxy.send_event(AppEvent::SetWidth(width));
                    }
                    return;
                } else if message.message_type == "set_title" {
                    if let Some(title) = message.title {
                        let _ = ipc_proxy.send_event(AppEvent::SetTitle(title));
                    }
                    return;
                }
            }

            if let Err(error) = handle_ipc_message(request.body(), &ipc_config_path) {
                eprintln!("Failed to handle IPC message: {error:#}");
            }
        })
        .build(&window)?;

    std::thread::spawn(move || {
        let config = load_config(&loader_config_path).unwrap_or_default();
        let html = match load_stations().and_then(|stations| {
            let stations_json = serde_json::to_string(&stations)?;
            Ok(build_html(&stations, &stations_json, &config))
        }) {
            Ok(html) => html,
            Err(error) => build_error_html(&format!("{error:#}")),
        };

        let _ = loader_proxy.send_event(AppEvent::CatalogLoaded(html));
    });

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(AppEvent::CatalogLoaded(html)) => {
                if let Ok(html) = serde_json::to_string(&html) {
                    let _ = webview.evaluate_script(&format!(
                        "document.open(); document.write({html}); document.close();"
                    ));
                }
            }
            Event::UserEvent(AppEvent::SetWidth(target_width)) => {
                let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
                window.set_inner_size(LogicalSize::new(
                    target_width.clamp(300.0, 760.0),
                    logical_size.height.max(744.0),
                ));
            }
            Event::UserEvent(AppEvent::SetTitle(title)) => {
                window.set_title(&title);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let logical_size = size.to_logical::<f64>(window.scale_factor());
                let mut config = load_config(&resize_config_path).unwrap_or_default();
                config.window_width = Some(logical_size.width.max(300.0));
                config.window_height = Some(logical_size.height.max(744.0));
                if let Err(error) = save_config(&resize_config_path, &config) {
                    eprintln!("Failed to save window size: {error:#}");
                }
            }
            _ => {}
        }
    });
}

fn app_config_path() -> AppResult<PathBuf> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("FMPLAY Radio")
        .join("config.json"))
}

fn load_config(path: &PathBuf) -> AppResult<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn save_config(path: &PathBuf, config: &AppConfig) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(config)?;
    fs::write(path, content)?;
    Ok(())
}

fn handle_ipc_message(message: &str, config_path: &PathBuf) -> AppResult<()> {
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

fn load_stations() -> AppResult<Vec<Station>> {
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

    let mut stations: Vec<_> = raw
        .into_iter()
        .filter_map(|(id, raw)| {
            if raw.enabled != "1" {
                return None;
            }

            let mut streams = Vec::new();
            if let Some(url) = raw.xtra_low.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "16",
                    bitrate: 16,
                    url,
                });
            }
            if let Some(url) = raw.url_low.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "24",
                    bitrate: 24,
                    url,
                });
            }
            if let Some(url) = raw.url.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "32",
                    bitrate: 32,
                    url,
                });
            }
            if let Some(url) = raw.url_hi.filter(|url| !url.is_empty()) {
                streams.push(Stream {
                    label: "48",
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
                site: raw.site,
                logo: absolute_url(&raw.logo),
                category: raw.category,
                position: raw.position.parse().unwrap_or(u32::MAX),
                meta: raw.meta.as_deref() == Some("meta"),
                streams,
            })
        })
        .collect();

    stations.sort_by(|a, b| compare_station_names(&a.name, &b.name));
    Ok(stations)
}

fn compare_station_names(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

fn absolute_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_owned()
    } else {
        format!("{FMPLAY_ROOT}{}", path.trim_start_matches('/'))
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn station_list_html(stations: &[Station]) -> String {
    stations
        .iter()
        .map(|station| {
            format!(
                r#"<button class="station" type="button" data-station-id="{id}">
  <img src="{logo}" alt="">
  <span>
    <span class="station-name">{name}</span>
    <span class="heart" aria-hidden="true">♥</span>
  </span>
</button>"#,
                id = escape_html(&station.id),
                logo = escape_html(&station.logo),
                name = escape_html(&station.name),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_loading_html() -> String {
    r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>FMPLAY Radio</title>
<style>
body {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  background: #111312;
  color: #f0f4ef;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.loading {
  display: grid;
  gap: 10px;
  text-align: center;
}
.title {
  font-size: 22px;
  font-weight: 750;
}
.status {
  color: #9da89f;
}
</style>
</head>
<body>
  <div class="loading">
    <div class="title">FMPLAY Radio</div>
    <div class="status">Загрузка каталога станций...</div>
  </div>
</body>
</html>"#
        .to_owned()
}

fn build_error_html(message: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>FMPLAY Radio</title>
<style>
body {{
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  background: #111312;
  color: #f0f4ef;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}}
.error {{
  max-width: 620px;
  padding: 24px;
}}
.title {{
  font-size: 22px;
  font-weight: 750;
}}
.message {{
  margin-top: 12px;
  color: #f6b7b7;
  white-space: pre-wrap;
}}
</style>
</head>
<body>
  <div class="error">
    <div class="title">Не удалось загрузить каталог FMPLAY</div>
    <div class="message">{message}</div>
  </div>
</body>
</html>"#,
        message = escape_html(message)
    )
}

fn build_html(stations: &[Station], stations_json: &str, config: &AppConfig) -> String {
    let list_html = station_list_html(stations);
    let config_json = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_owned());

    format!(
        r#"<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>FMPLAY Radio</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;500;600;700;800&display=swap" rel="stylesheet">
<style>
:root {{
  color-scheme: dark;
  --bg: #090b0a;
  --panel: #111413;
  --panel-2: #181d1a;
  --text: #f3f4f6;
  --muted: #9ca3af;
  --line: rgba(255, 255, 255, 0.05);
  --accent: #10b981;
  --accent-rgb: 16, 185, 129;
  --accent-2: #f43f5e;
  --header: #0d0f0e;
  --sidebar: #0e1110;
  --card: rgba(255, 255, 255, 0.015);
  --card-border: rgba(255, 255, 255, 0.03);
  --cover-bg: #1a1f1d;
  --content-bg: radial-gradient(circle at top left, rgba(16, 185, 129, 0.08), transparent 400px), var(--bg);
  --button-primary-text: #022c22;
  --favorite-muted: rgba(156, 163, 175, 0.6);
  --heart-muted: rgba(156, 163, 175, 0.25);
  --modal-shadow: rgba(0, 0, 0, 0.6);
}}
body.light {{
  color-scheme: light;
  --bg: #f8fafc;
  --panel: #ffffff;
  --panel-2: #f1f5f9;
  --text: #0f172a;
  --muted: #64748b;
  --line: rgba(0, 0, 0, 0.06);
  --accent: #059669;
  --accent-rgb: 5, 150, 105;
  --accent-2: #e11d48;
  --header: #ffffff;
  --sidebar: #f8fafc;
  --card: rgba(255, 255, 255, 0.8);
  --card-border: rgba(0, 0, 0, 0.04);
  --cover-bg: #e2e8f0;
  --content-bg: radial-gradient(circle at top left, rgba(5, 150, 105, 0.06), transparent 400px), var(--bg);
  --button-primary-text: #ffffff;
  --favorite-muted: rgba(100, 116, 139, 0.7);
  --heart-muted: rgba(100, 116, 139, 0.3);
  --modal-shadow: rgba(15, 23, 42, 0.08);
}}
* {{
  box-sizing: border-box;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}}
body {{
  margin: 0;
  min-height: 100vh;
  background: var(--bg);
  color: var(--text);
  font-family: 'Outfit', -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  letter-spacing: -0.01em;
}}
.shell {{
  display: grid;
  grid-template-rows: auto 1fr auto;
  height: 100vh;
}}
header {{
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 16px;
  align-items: center;
  padding: 16px 24px;
  border-bottom: 1px solid var(--line);
  background: var(--header);
  backdrop-filter: blur(10px);
}}
.window-toggle {{
  width: 38px;
  height: 38px;
  min-width: 38px;
  min-height: 38px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  padding: 0;
  font-family: inherit;
  font-size: 16px;
  cursor: pointer;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}}
.window-toggle:hover {{
  background: var(--panel-2);
  border-color: rgba(var(--accent-rgb), 0.3);
  transform: translateY(-0.5px);
}}
.window-toggle:active {{
  transform: translateY(0.5px);
}}
.search {{
  width: 100%;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  padding: 10px 14px;
  font-family: inherit;
  font-size: 14px;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}}
.search:focus {{
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.15);
}}
main {{
  display: grid;
  grid-template-columns: 320px minmax(240px, 1fr);
  min-height: 0;
  overflow-x: auto;
}}
.sidebar {{
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  align-content: start;
  gap: 10px;
  overflow-y: auto;
  border-left: 1px solid var(--line);
  padding: 16px;
  background: var(--sidebar);
}}
.sidebar::-webkit-scrollbar {{
  width: 6px;
}}
.sidebar::-webkit-scrollbar-track {{
  background: transparent;
}}
.sidebar::-webkit-scrollbar-thumb {{
  background: rgba(255, 255, 255, 0.08);
  border-radius: 10px;
}}
body.light .sidebar::-webkit-scrollbar-thumb {{
  background: rgba(0, 0, 0, 0.08);
}}
.sidebar::-webkit-scrollbar-thumb:hover {{
  background: rgba(var(--accent-rgb), 0.3);
}}
.station {{
  position: relative;
  width: 100%;
  display: grid;
  grid-template-columns: 44px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
  min-height: 64px;
  padding: 10px 32px 10px 10px;
  border: 1px solid var(--card-border);
  border-radius: 10px;
  color: var(--text);
  background: var(--card);
  text-align: left;
  cursor: pointer;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1), transform 0.3s cubic-bezier(0.16, 1, 0.3, 1), box-shadow 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}}
.station:hover {{
  background: var(--panel-2);
  border-color: rgba(255, 255, 255, 0.08);
  transform: translateY(-2px);
  box-shadow: 0 6px 16px rgba(0, 0, 0, 0.25);
}}
body.light .station:hover {{
  border-color: rgba(0, 0, 0, 0.08);
  box-shadow: 0 6px 16px rgba(15, 23, 42, 0.05);
}}
.station.active {{
  background: rgba(var(--accent-rgb), 0.06);
  border-color: rgba(var(--accent-rgb), 0.3);
  box-shadow: inset 4px 0 0 var(--accent);
}}
.station img {{
  width: 44px;
  height: 44px;
  border-radius: 8px;
  object-fit: cover;
  background: var(--cover-bg);
  box-shadow: 0 2px 6px rgba(0,0,0,0.15);
}}
.station-name {{
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: normal;
  font-family: inherit;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.3;
  max-height: 2.6em;
}}
.heart {{
  position: absolute;
  right: 12px;
  top: 50%;
  transform: translateY(-50%);
  color: var(--heart-muted);
  font-size: 16px;
  line-height: 1;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}}
.station:hover .heart {{
  color: var(--favorite-muted);
}}
.station.favorite .heart {{
  color: var(--accent-2) !important;
  text-shadow: 0 0 8px rgba(244, 63, 94, 0.4);
}}

/* Equalizer styles for active and playing station */
.playing-eq {{
  display: none;
  align-items: flex-end;
  gap: 2px;
  width: 14px;
  height: 12px;
  position: absolute;
  right: 32px;
  top: 50%;
  transform: translateY(-50%);
}}
.playing-eq span {{
  width: 2px;
  height: 100%;
  background-color: var(--accent);
  border-radius: 1px;
  transform-origin: bottom;
  animation: eq-bounce 0.8s ease-in-out infinite alternate;
}}
.playing-eq span:nth-child(1) {{ animation-delay: -0.2s; }}
.playing-eq span:nth-child(2) {{ animation-delay: -0.4s; }}
.playing-eq span:nth-child(3) {{ animation-delay: -0.1s; }}

@keyframes eq-bounce {{
  0% {{ transform: scaleY(0.15); }}
  100% {{ transform: scaleY(1.0); }}
}}

.station.active.playing .playing-eq {{
  display: inline-flex;
}}
.content {{
  min-width: 0;
  display: grid;
  grid-template-rows: minmax(0, 1fr) auto;
  background: var(--content-bg);
}}
.now {{
  display: grid;
  grid-template-rows: auto 1fr;
  gap: 24px;
  align-content: start;
  width: 100%;
  max-width: 320px;
  margin: 0 auto;
  padding: 32px 20px;
}}
.cover {{
  width: 100%;
  max-width: 100%;
  aspect-ratio: 1;
  border-radius: 16px;
  object-fit: cover;
  background: var(--panel);
  border: 1px solid rgba(255, 255, 255, 0.05);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.35);
  transition: transform 0.6s cubic-bezier(0.16, 1, 0.3, 1), box-shadow 0.6s cubic-bezier(0.16, 1, 0.3, 1);
}}
body.light .cover {{
  border-color: rgba(0,0,0,0.05);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.1);
}}
.cover:hover {{
  transform: scale(1.02);
}}
.cover.playing {{
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.4), 0 0 30px rgba(var(--accent-rgb), 0.35);
  animation: cover-glow 4s ease-in-out infinite alternate;
}}
@keyframes cover-glow {{
  0% {{
    transform: scale(1.01);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.4), 0 0 30px rgba(var(--accent-rgb), 0.3);
  }}
  100% {{
    transform: scale(1.06);
    box-shadow: 0 16px 44px rgba(0, 0, 0, 0.35), 0 0 60px rgba(var(--accent-rgb), 0.7);
  }}
}}
.title {{
  margin: 0 0 6px;
  font-size: 24px;
  font-weight: 800;
  line-height: 1.15;
  letter-spacing: -0.02em;
}}
.subtitle {{
  min-height: 20px;
  color: var(--muted);
  font-size: 13px;
  font-weight: 500;
  letter-spacing: 0.02em;
}}
.controls {{
  display: flex;
  gap: 12px;
  align-items: center;
  margin-top: 24px;
  width: 100%;
}}
button.control, .quality button {{
  min-width: 44px;
  min-height: 40px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--panel);
  color: var(--text);
  font-family: inherit;
  font-weight: 600;
  font-size: 13px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1), transform 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}}
button.control:hover {{
  background: var(--panel-2);
  border-color: rgba(255, 255, 255, 0.1);
  transform: translateY(-1px);
}}
body.light button.control:hover {{
  border-color: rgba(0, 0, 0, 0.08);
}}
button.control:active {{
  transform: translateY(0.5px);
}}
button.control.primary {{
  flex: 1;
  height: 40px;
  min-height: 40px;
  background: linear-gradient(135deg, var(--accent) 0%, #059669 100%);
  color: var(--button-primary-text);
  border-color: transparent;
  box-shadow: 0 4px 14px rgba(var(--accent-rgb), 0.35);
  font-size: 14px;
  font-weight: 700;
}}
button.control.primary:hover {{
  background: linear-gradient(135deg, #10b981 0%, #047857 100%);
  box-shadow: 0 6px 20px rgba(var(--accent-rgb), 0.5);
}}
button.control.primary.playing {{
  background: linear-gradient(135deg, #f43f5e 0%, #e11d48 100%) !important;
  color: #ffffff !important;
  box-shadow: 0 4px 14px rgba(225, 29, 72, 0.35) !important;
}}
button.control.primary.playing:hover {{
  background: linear-gradient(135deg, #fb7185 0%, #be123c 100%) !important;
  box-shadow: 0 6px 20px rgba(225, 29, 72, 0.5) !important;
}}
button.control.settings-toggle {{
  width: 40px;
  height: 40px;
  min-width: 40px;
  min-height: 40px;
  font-size: 16px;
  padding: 0;
}}
button.control.favorite-toggle {{
  width: 40px;
  height: 40px;
  min-width: 40px;
  min-height: 40px;
  font-size: 18px;
  padding: 0;
  color: var(--favorite-muted);
}}
button.control.favorite-toggle.active {{
  color: #ffffff;
  background: linear-gradient(135deg, #f43f5e 0%, #be123c 100%);
  border-color: transparent;
  box-shadow: 0 4px 12px rgba(244, 63, 94, 0.3);
}}
.quality {{
  display: flex;
  gap: 4px;
  padding: 4px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: rgba(0, 0, 0, 0.2);
  width: 100%;
}}
body.light .quality {{
  background: rgba(0, 0, 0, 0.04);
}}
.quality button {{
  flex: 1;
  min-width: 0;
  min-height: 32px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font-family: inherit;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
  transition: all 0.15s ease;
}}
.quality button:hover {{
  background: rgba(255, 255, 255, 0.05);
}}
body.light .quality button:hover {{
  background: rgba(0, 0, 0, 0.03);
}}
.quality button.active {{
  background: var(--panel);
  color: var(--accent);
  border-color: var(--line);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.15);
}}
.volume {{
  display: block;
  width: 100%;
  height: 16px;
  accent-color: var(--accent);
  cursor: pointer;
  background: transparent;
  border: none;
  outline: none;
  margin-top: 16px;
}}
.now-track {{
  margin-top: 24px;
  padding: 16px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.02);
  border: 1px solid rgba(255, 255, 255, 0.04);
  backdrop-filter: blur(10px);
}}
body.light .now-track {{
  background: rgba(0, 0, 0, 0.02);
  border-color: rgba(0, 0, 0, 0.04);
}}
.track-label {{
  color: var(--accent);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}}
.track-text {{
  margin-top: 8px;
  color: var(--text);
  font-size: 14px;
  font-weight: 600;
  line-height: 1.4;
}}
.track-text.muted {{
  color: var(--muted);
  font-weight: 500;
}}
footer {{
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 16px;
  align-items: center;
  padding: 14px 24px;
  border-top: 1px solid var(--line);
  background: var(--header);
  color: var(--muted);
  font-size: 13px;
  font-weight: 500;
}}
audio {{
  display: none;
}}
.modal-backdrop {{
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(8px);
  z-index: 100;
  animation: backdropFadeIn 0.2s ease-out;
}}
@keyframes backdropFadeIn {{
  from {{ opacity: 0; }}
  to {{ opacity: 1; }}
}}
.modal-backdrop.hidden {{
  display: none;
}}
.settings-modal {{
  width: min(290px, 100%);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 16px;
  background: rgba(17, 20, 19, 0.92);
  color: var(--text);
  padding: 20px;
  backdrop-filter: blur(25px);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.5);
  animation: modalFadeIn 0.25s cubic-bezier(0.16, 1, 0.3, 1);
}}
body.light .settings-modal {{
  border-color: rgba(0, 0, 0, 0.08);
  background: rgba(255, 255, 255, 0.92);
  box-shadow: 0 20px 40px rgba(15, 23, 42, 0.15);
}}
@keyframes modalFadeIn {{
  from {{ transform: scale(0.95); opacity: 0; }}
  to {{ transform: scale(1); opacity: 1; }}
}}
.settings-header {{
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 20px;
}}
.settings-title {{
  font-size: 18px;
  font-weight: 800;
  letter-spacing: -0.01em;
}}
.icon-button {{
  width: 28px;
  height: 28px;
  border: 1px solid var(--line);
  border-radius: 50%;
  background: var(--panel-2);
  color: var(--text);
  font-size: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}}
.icon-button:hover {{
  background: rgba(244, 63, 94, 0.2);
  color: #f43f5e;
  border-color: rgba(244, 63, 94, 0.3);
  transform: scale(1.05);
}}
.settings-section {{
  display: grid;
  gap: 8px;
  margin-top: 18px;
}}
.settings-label {{
  color: var(--muted);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}}
/* Deleted .theme-select since we use premium segmented controls */
@media (max-width: 700px) {{
  footer {{ grid-template-columns: 1fr; }}
}}
@media (max-width: 540px) {{
  header {{
    grid-template-columns: 1fr;
  }}
  .search {{
    display: none;
  }}
  main {{
    grid-template-columns: 300px;
    justify-content: center;
  }}
  .sidebar {{
    display: none;
  }}
  .content {{
    border-left: 1px solid var(--line);
    border-right: 1px solid var(--line);
  }}
}}
</style>
</head>
<body>
<div class="shell">
  <header>
    <input id="search" class="search" type="search" placeholder="Поиск станции">
    <button id="width-toggle" class="window-toggle" title="Свернуть боковую панель">▶</button>
  </header>
  <main>
    <section class="content">
      <div class="now">
        <img id="cover" class="cover" alt="">
        <div>
          <div class="title" id="title">Выберите станцию</div>
          <div class="subtitle" id="subtitle">Выберите станцию</div>
          <div class="controls">
            <button id="play" class="control primary">Play</button>
            <button id="settings" class="control settings-toggle" title="Настройки">⚙</button>
            <button id="favorite" class="control favorite-toggle" title="Избранное">♥</button>
          </div>
          <input id="volume" class="volume" type="range" min="0" max="1" step="0.01" value="0.9">
          <div class="now-track">
            <div class="track-label">Текущий трек</div>
            <div id="track" class="track-text muted">Выберите станцию</div>
          </div>
        </div>
      </div>
      <footer>
        <span id="status">Каталог загружен: {count} станций</span>
        <audio id="audio" crossorigin="anonymous"></audio>
      </footer>
    </section>
    <aside id="list" class="sidebar">{list_html}</aside>
  </main>
</div>
<div id="settings-modal" class="modal-backdrop hidden">
  <div class="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
    <div class="settings-header">
      <div id="settings-title" class="settings-title">Настройки</div>
      <button id="settings-close" class="icon-button" title="Закрыть">×</button>
    </div>
    <div class="settings-section">
      <span class="settings-label">Качество потока</span>
      <div id="quality" class="quality" aria-label="Качество потока"></div>
    </div>
    <div class="settings-section">
      <span class="settings-label">Тема оформления</span>
      <div id="theme-segmented" class="quality" aria-label="Тема оформления">
        <button class="theme-btn" data-theme="dark">Темная</button>
        <button class="theme-btn" data-theme="light">Светлая</button>
      </div>
    </div>
    <div class="settings-section">
      <span class="settings-label">Список станций</span>
      <div id="favorites-segmented" class="quality" aria-label="Фильтрация списка">
        <button class="fav-filter-btn" data-filter="all">Все станции</button>
        <button class="fav-filter-btn" data-filter="favorites">Только избранное</button>
      </div>
    </div>
  </div>
</div>
<script>
var stations = {stations_json};
var initialConfig = {config_json};
var current = null;
var currentQuality = Number(readSetting("quality", String(initialConfig.quality || 48)));
var currentTheme = readSetting("theme", initialConfig.theme || "dark");
var favorites = parseFavorites(readSetting("favorites", JSON.stringify(initialConfig.favorites || [])));
var savedLastStation = readSetting("last_station", initialConfig.last_station || "");
var savedVolume = Number(readSetting("volume", String(initialConfig.volume || 0.9)));
var onlyFavorites = readSetting("only_favorites", String(initialConfig.only_favorites || false)) === "true";
var list = document.getElementById("list");
var search = document.getElementById("search");
var audio = document.getElementById("audio");
var cover = document.getElementById("cover");
var title = document.getElementById("title");
var subtitle = document.getElementById("subtitle");
var status = document.getElementById("status");
var quality = document.getElementById("quality");
var favoriteButton = document.getElementById("favorite");
var widthToggle = document.getElementById("width-toggle");
var settingsButton = document.getElementById("settings");
var settingsModal = document.getElementById("settings-modal");
var settingsClose = document.getElementById("settings-close");
var themeSegmented = document.getElementById("theme-segmented");
var track = document.getElementById("track");
var metaTimer = null;
var metaToken = 0;
var lastMetaId = null;

function updateDocumentTitle(stationName, trackName) {{
  var newTitle = "FMPLAY Radio";
  if (stationName) {{
    if (trackName && trackName.indexOf("Выберите станцию") === -1 && trackName.indexOf("Обновление") === -1 && trackName.indexOf("Не удалось") === -1) {{
      newTitle = stationName + " — " + trackName + " | FMPLAY";
    }} else {{
      newTitle = stationName + " | FMPLAY";
    }}
  }}
  document.title = newTitle;
  try {{
    if (window.ipc && window.ipc.postMessage) {{
      window.ipc.postMessage(JSON.stringify({{ type: "set_title", title: newTitle }}));
    }}
  }} catch (error) {{}}
}}

function readSetting(key, fallback) {{
  try {{
    return window.localStorage ? window.localStorage.getItem(key) || fallback : fallback;
  }} catch (error) {{
    return fallback;
  }}
}}

function writeSetting(key, value) {{
  try {{
    if (window.localStorage) window.localStorage.setItem(key, value);
  }} catch (error) {{}}
}}

function parseFavorites(value) {{
  try {{
    var parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
  }} catch (error) {{
    return [];
  }}
}}

function isFavorite(station) {{
  return favorites.indexOf(station.id) !== -1;
}}

function saveFavorites() {{
  saveAppConfig();
}}

function saveAppConfig() {{
  var payload = {{
    type: "config",
    favorites: favorites,
    last_station: current ? current.id : savedLastStation,
    quality: currentQuality,
    volume: audio.volume,
    theme: currentTheme,
    only_favorites: onlyFavorites,
    window_width: window.innerWidth,
    window_height: window.innerHeight
  }};
  writeSetting("favorites", JSON.stringify(favorites));
  writeSetting("last_station", payload.last_station || "");
  writeSetting("quality", String(currentQuality));
  writeSetting("volume", String(audio.volume));
  writeSetting("theme", currentTheme);
  writeSetting("only_favorites", String(onlyFavorites));
  try {{
    if (window.ipc && window.ipc.postMessage) {{
      window.ipc.postMessage(JSON.stringify(payload));
    }}
  }} catch (error) {{}}
}}

function normalizeTheme(theme) {{
  return theme === "light" ? "light" : "dark";
}}

function applyTheme(theme) {{
  currentTheme = normalizeTheme(theme);
  document.body.className = currentTheme === "light" ? "light" : "";
  var buttons = themeSegmented.getElementsByClassName("theme-btn");
  for (var index = 0; index < buttons.length; index += 1) {{
    var btn = buttons[index];
    if (btn.getAttribute("data-theme") === currentTheme) {{
      btn.className = "theme-btn active";
    }} else {{
      btn.className = "theme-btn";
    }}
  }}
}}

function applyFavoritesFilter(onlyFav) {{
  onlyFavorites = onlyFav;
  var buttons = document.getElementById("favorites-segmented").getElementsByClassName("fav-filter-btn");
  for (var index = 0; index < buttons.length; index += 1) {{
    var btn = buttons[index];
    var isFavBtn = btn.getAttribute("data-filter") === "favorites";
    if (isFavBtn === onlyFavorites) {{
      btn.className = "fav-filter-btn active";
    }} else {{
      btn.className = "fav-filter-btn";
    }}
  }}
  render(filteredStations());
}}

function openSettings() {{
  settingsModal.className = "modal-backdrop";
}}

function closeSettings() {{
  settingsModal.className = "modal-backdrop hidden";
}}

function postToggleWidth() {{
  try {{
    if (window.ipc && window.ipc.postMessage) {{
      var targetWidth = window.innerWidth <= 540 ? 760 : 300;
      window.ipc.postMessage(JSON.stringify({{ type: "set_width", target_width: targetWidth }}));
    }}
  }} catch (error) {{}}
}}

function updateWidthToggle() {{
  var collapsed = window.innerWidth <= 540;
  widthToggle.textContent = collapsed ? "◀" : "▶";
  widthToggle.title = collapsed ? "Развернуть боковую панель" : "Свернуть боковую панель";
}}

function toggleFavorite(station) {{
  if (!station) return;
  var index = favorites.indexOf(station.id);
  if (index === -1) {{
    favorites.push(station.id);
    status.textContent = "В избранном: " + station.name;
  }} else {{
    favorites.splice(index, 1);
    status.textContent = "Удалено из избранного: " + station.name;
  }}
  saveFavorites();
  updateFavoriteButton();
  render(filteredStations());
}}

function updateFavoriteButton() {{
  if (!current) {{
    favoriteButton.className = "control favorite-toggle";
    favoriteButton.title = "Избранное";
    return;
  }}
  var active = isFavorite(current);
  favoriteButton.className = "control favorite-toggle" + (active ? " active" : "");
  favoriteButton.title = active ? "Убрать из избранного" : "Добавить в избранное";
}}

function sortedStations(items) {{
  return items.slice().sort(function(a, b) {{
    var favA = isFavorite(a) ? 1 : 0;
    var favB = isFavorite(b) ? 1 : 0;
    if (favA !== favB) return favB - favA;
    return a.name.localeCompare(b.name, "ru", {{ sensitivity: "base" }});
  }});
}}

function streamFor(station) {{
  var hi = null;
  for (var index = 0; index < station.streams.length; index += 1) {{
    if (station.streams[index].bitrate === currentQuality) return station.streams[index];
    if (station.streams[index].bitrate === 48) hi = station.streams[index];
  }}
  return hi || station.streams[station.streams.length - 1];
}}

function render(items) {{
  items = sortedStations(items || stations);
  list.innerHTML = "";
  var isCurrentlyPlaying = !audio.paused && audio.src;
  for (var index = 0; index < items.length; index += 1) {{
    var station = items[index];
    var button = document.createElement("button");
    var img = document.createElement("img");
    var text = document.createElement("span");
    var name = document.createElement("span");
    var heart = document.createElement("span");

    var eq = document.createElement("span");
    eq.className = "playing-eq";
    var bar1 = document.createElement("span");
    var bar2 = document.createElement("span");
    var bar3 = document.createElement("span");
    eq.appendChild(bar1);
    eq.appendChild(bar2);
    eq.appendChild(bar3);

    button.className = "station"
      + (current && current.id === station.id ? " active" : "")
      + (current && current.id === station.id && isCurrentlyPlaying ? " playing" : "")
      + (isFavorite(station) ? " favorite" : "");
    img.src = station.logo;
    img.alt = "";
    name.className = "station-name";
    name.textContent = station.name;
    heart.className = "heart";
    heart.textContent = "♥";
    text.appendChild(name);
    text.appendChild(heart);
    button.appendChild(img);
    button.appendChild(text);
    button.appendChild(eq);
    button.addEventListener("click", stationClickHandler(station));
    button.addEventListener("contextmenu", stationFavoriteHandler(station));
    list.appendChild(button);
  }}
}}

function stationClickHandler(station) {{
  return function() {{
    selectStation(station, true);
  }};
}}

function stationFavoriteHandler(station) {{
  return function(event) {{
    event.preventDefault();
    toggleFavorite(station);
  }};
}}

function renderQuality() {{
  quality.innerHTML = "";
  if (!current) return;
  for (var index = 0; index < current.streams.length; index += 1) {{
    var stream = current.streams[index];
    var button = document.createElement("button");
    button.textContent = stream.label;
    button.className = stream.bitrate === streamFor(current).bitrate ? "active" : "";
    button.title = stream.bitrate + " кбит/с";
    button.addEventListener("click", qualityClickHandler(stream));
    quality.appendChild(button);
  }}
}}

function qualityClickHandler(stream) {{
  return function() {{
    currentQuality = stream.bitrate;
    saveAppConfig();
    selectStation(current, !audio.paused);
  }};
}}

function selectStation(station, shouldPlay) {{
  current = station;
  savedLastStation = station.id;
  metaToken += 1;
  lastMetaId = null;
  var stream = streamFor(station);
  title.textContent = station.name;
  subtitle.textContent = station.site || "fmplay.ru";
  updateDocumentTitle(station.name, "");
  cover.src = station.logo;
  audio.src = stream.url;
  renderQuality();
  updateFavoriteButton();
  render(filteredStations());
  status.textContent = "Готово: " + station.name;
  startMetadataPolling();
  saveAppConfig();
  if (shouldPlay) play();
}}

function setTrackText(text, muted) {{
  track.textContent = text;
  track.className = "track-text" + (muted ? " muted" : "");
}}

function startMetadataPolling() {{
  if (metaTimer) window.clearInterval(metaTimer);
  if (!current || !current.meta) {{
    setTrackText("Для этой станции нет текущего трека", true);
    return;
  }}
  setTrackText("Обновление...", true);
  pollMetadata(metaToken);
  metaTimer = window.setInterval(function() {{
    pollMetadata(metaToken);
  }}, 5000);
}}

function pollMetadata(token) {{
  if (!current || !current.meta || token !== metaToken) return;
  var stationId = current.id;
  var stamp = Date.now();
  fetch("https://pic.fmplay.ru/stations/" + stationId + "/fmplay_id.json?" + stamp)
    .then(function(response) {{ return response.json(); }})
    .then(function(idData) {{
      if (token !== metaToken || !current || current.id !== stationId) return null;
      if (!idData || idData.uniqueid === lastMetaId) return null;
      lastMetaId = idData.uniqueid;
      return fetch("https://pic.fmplay.ru/stations/" + stationId + "/fmplay_current.json?" + Date.now());
    }})
    .then(function(response) {{
      if (!response) return null;
      return response.json();
    }})
    .then(function(meta) {{
      if (!meta || token !== metaToken || !current || current.id !== stationId) return;
      var artist = meta.artist || "";
      var songTitle = meta.title || "";
      var text = "";
      if (artist && songTitle) text = artist + " - " + songTitle;
      else text = artist || songTitle;
      setTrackText(text || "Сейчас нет данных о треке", !text);
      if (current && current.id === stationId) {{
        updateDocumentTitle(current.name, text);
      }}
    }})
    .catch(function() {{
      if (token === metaToken) setTrackText("Не удалось обновить текущий трек", true);
    }});
}}

function play() {{
  if (!current && stations.length > 0) selectStation(stations[0], false);
  if (current && !audio.src) {{
    var stream = streamFor(current);
    audio.src = stream.url;
  }}
  audio.play()
    .then(function() {{ status.textContent = "Играет: " + current.name; }})
    .catch(function(error) {{ status.textContent = "Не удалось запустить поток: " + error.message; }});
}}

function stop() {{
  audio.pause();
  audio.removeAttribute("src");
  audio.load();
  status.textContent = current ? "Остановлено: " + current.name : "Остановлено";
  updateDocumentTitle(current ? current.name : "", "");
}}

function togglePlay() {{
  if (audio.paused || !audio.src) {{
    play();
  }} else {{
    stop();
  }}
}}

function updatePlayButtonState(isPlaying) {{
  var playBtn = document.getElementById("play");
  var cover = document.getElementById("cover");
  if (playBtn) {{
    if (isPlaying) {{
      playBtn.textContent = "Stop";
      playBtn.className = "control primary playing";
      if (cover) cover.classList.add("playing");
    }} else {{
      playBtn.textContent = "Play";
      playBtn.className = "control primary";
      if (cover) cover.classList.remove("playing");
    }}
  }}
  var activeStationBtn = document.querySelector(".station.active");
  if (activeStationBtn) {{
    if (isPlaying) {{
      activeStationBtn.classList.add("playing");
    }} else {{
      activeStationBtn.classList.remove("playing");
    }}
  }}
}}

function filteredStations() {{
  var q = search.value.trim().toLowerCase();
  var filtered = stations;
  if (onlyFavorites) {{
    filtered = filtered.filter(function(station) {{
      return isFavorite(station);
    }});
  }}
  if (!q) return filtered;
  return filtered.filter(function(station) {{
    return station.name.toLowerCase().indexOf(q) !== -1 || station.id.toLowerCase().indexOf(q) !== -1;
  }});
}}

function findStationById(id) {{
  if (!id) return null;
  for (var index = 0; index < stations.length; index += 1) {{
    if (stations[index].id === id) return stations[index];
  }}
  return null;
}}

document.getElementById("play").addEventListener("click", togglePlay);
favoriteButton.addEventListener("click", function() {{ toggleFavorite(current); }});
widthToggle.addEventListener("click", postToggleWidth);
settingsButton.addEventListener("click", openSettings);
settingsClose.addEventListener("click", closeSettings);
settingsModal.addEventListener("click", function(event) {{
  if (event.target === settingsModal) closeSettings();
}});
var themeBtns = themeSegmented.getElementsByClassName("theme-btn");
for (var btnIndex = 0; btnIndex < themeBtns.length; btnIndex += 1) {{
  themeBtns[btnIndex].addEventListener("click", function(event) {{
    applyTheme(event.target.getAttribute("data-theme"));
    saveAppConfig();
  }});
}}
var filterBtns = document.getElementById("favorites-segmented").getElementsByClassName("fav-filter-btn");
for (var btnIndex = 0; btnIndex < filterBtns.length; btnIndex += 1) {{
  filterBtns[btnIndex].addEventListener("click", function(event) {{
    applyFavoritesFilter(event.target.getAttribute("data-filter") === "favorites");
    saveAppConfig();
  }});
}}
document.addEventListener("keydown", function(event) {{
  if (event.key === "Escape") closeSettings();
}});
document.getElementById("volume").addEventListener("input", function(event) {{
  audio.volume = Number(event.target.value);
  saveAppConfig();
}});
search.addEventListener("input", function() {{ render(filteredStations()); }});
function updateWindowSize() {{
  updateWidthToggle();
  saveAppConfig();
}}
window.addEventListener("resize", updateWindowSize);
applyTheme(currentTheme);
applyFavoritesFilter(onlyFavorites);
audio.volume = isFinite(savedVolume) ? Math.min(1, Math.max(0, savedVolume)) : 0.9;
document.getElementById("volume").value = String(audio.volume);
updateWindowSize();
audio.addEventListener("play", function() {{ updatePlayButtonState(true); }});
audio.addEventListener("playing", function() {{ updatePlayButtonState(true); }});
audio.addEventListener("pause", function() {{ updatePlayButtonState(false); }});
audio.addEventListener("ended", function() {{ updatePlayButtonState(false); }});
audio.addEventListener("emptied", function() {{ updatePlayButtonState(false); }});
audio.addEventListener("waiting", function() {{ status.textContent = "Буферизация..."; }});
audio.addEventListener("error", function() {{ status.textContent = "Ошибка аудиопотока. Попробуйте другое качество или станцию."; }});
render(filteredStations());
if (stations.length > 0) selectStation(findStationById(savedLastStation) || stations[0], false);
updateFavoriteButton();
</script>
</body>
</html>"#,
        list_html = list_html,
        stations_json = stations_json,
        config_json = config_json,
        count = stations.len()
    )
}
