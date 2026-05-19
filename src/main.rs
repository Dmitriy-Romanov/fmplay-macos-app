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
    window_width: Option<f64>,
    window_height: Option<f64>,
    target_width: Option<f64>,
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
    let initial_height = startup_config.window_height.unwrap_or(760.0).max(760.0);

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let loader_proxy = event_loop.create_proxy();
    let ipc_proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("FMPLAY Radio")
        .with_inner_size(LogicalSize::new(initial_width, initial_height))
        .with_min_inner_size(LogicalSize::new(300.0, 760.0))
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
                    logical_size.height.max(760.0),
                ));
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
                config.window_height = Some(logical_size.height.max(760.0));
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
<style>
:root {{
  color-scheme: dark;
  --bg: #111312;
  --panel: #191d1b;
  --panel-2: #202622;
  --text: #f0f4ef;
  --muted: #9da89f;
  --line: #2b322e;
  --accent: #58d68d;
  --accent-2: #f7c948;
  --header: #151816;
  --sidebar: #141715;
  --card: rgba(255,255,255,.025);
  --card-border: rgba(255,255,255,.05);
  --cover-bg: #252b27;
  --content-bg: radial-gradient(circle at top left, rgba(88,214,141,.12), transparent 340px), var(--bg);
  --button-primary-text: #07120b;
  --favorite-muted: rgba(157,168,159,.75);
  --heart-muted: rgba(157,168,159,.35);
  --modal-shadow: rgba(0,0,0,.35);
}}
body.light {{
  color-scheme: light;
  --bg: #f5f7f4;
  --panel: #ffffff;
  --panel-2: #edf3ee;
  --text: #161a17;
  --muted: #667067;
  --line: #d6ded7;
  --accent: #2bb96f;
  --accent-2: #f0b429;
  --header: #ffffff;
  --sidebar: #f7faf7;
  --card: rgba(255,255,255,.8);
  --card-border: rgba(31,42,35,.12);
  --cover-bg: #e9eee9;
  --content-bg: radial-gradient(circle at top left, rgba(43,185,111,.12), transparent 340px), var(--bg);
  --button-primary-text: #06140b;
  --favorite-muted: rgba(88,100,91,.75);
  --heart-muted: rgba(88,100,91,.35);
  --modal-shadow: rgba(22,26,23,.2);
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  min-height: 100vh;
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  letter-spacing: 0;
}}
.shell {{
  display: grid;
  grid-template-rows: auto 1fr auto;
  height: 100vh;
}}
header {{
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 14px;
  align-items: center;
  padding: 18px 22px;
  border-bottom: 1px solid var(--line);
  background: var(--header);
}}
.window-toggle {{
  min-height: 38px;
  justify-self: start;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  padding: 0 14px;
  font-size: 14px;
  font-weight: 750;
  cursor: pointer;
}}
.window-toggle:hover {{
  background: var(--panel-2);
}}
.search {{
  width: 100%;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  padding: 10px 12px;
  font-size: 15px;
}}
main {{
  display: grid;
  grid-template-columns: minmax(220px, 1fr) 300px;
  min-height: 0;
  overflow-x: auto;
}}
.sidebar {{
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  align-content: start;
  gap: 8px;
  overflow: auto;
  border-right: 1px solid var(--line);
  padding: 12px;
  background: var(--sidebar);
}}
.station {{
  position: relative;
  width: 100%;
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr);
  gap: 10px;
  align-items: center;
  min-height: 62px;
  padding: 10px 28px 10px 10px;
  border: 1px solid var(--card-border);
  border-radius: 7px;
  color: var(--text);
  background: var(--card);
  text-align: left;
  cursor: pointer;
}}
.station:hover, .station.active {{ background: var(--panel-2); }}
.station.active {{
  border-color: rgba(88,214,141,.6);
  box-shadow: inset 3px 0 0 var(--accent);
}}
.station img {{
  width: 40px;
  height: 40px;
  border-radius: 7px;
  object-fit: cover;
  background: var(--cover-bg);
}}
.station-name {{
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: normal;
  font-size: 14px;
  font-weight: 650;
  line-height: 1.25;
  max-height: 2.5em;
}}
.heart {{
  position: absolute;
  right: 8px;
  top: 6px;
  color: var(--heart-muted);
  font-size: 15px;
  line-height: 1;
}}
.station.favorite .heart {{
  color: var(--accent-2);
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
  gap: 20px;
  align-content: start;
  width: 100%;
  max-width: 300px;
  margin: 0 auto;
  padding: 28px;
}}
.cover {{
  width: 100%;
  max-width: 240px;
  aspect-ratio: 1;
  border-radius: 8px;
  object-fit: cover;
  background: var(--panel);
  border: 1px solid var(--line);
}}
.title {{
  margin: 0 0 10px;
  font-size: 25px;
  font-weight: 780;
  line-height: 1.12;
}}
.subtitle {{
  min-height: 24px;
  color: var(--muted);
  font-size: 14px;
}}
.controls {{
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  align-items: center;
  margin-top: 22px;
}}
button.control, .quality button {{
  min-width: 44px;
  min-height: 38px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  font-weight: 700;
  cursor: pointer;
}}
button.control.primary {{
  min-width: 64px;
  background: var(--accent);
  color: var(--button-primary-text);
  border-color: transparent;
}}
button.control.settings-toggle {{
  min-width: 52px;
  font-size: 18px;
}}
button.control.favorite-toggle {{
  min-width: 52px;
  font-size: 20px;
  color: var(--favorite-muted);
}}
button.control.favorite-toggle.active {{
  color: #161204;
  background: var(--accent-2);
  border-color: transparent;
}}
.quality {{
  display: inline-flex;
  gap: 6px;
  padding: 5px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: rgba(0,0,0,.12);
}}
.quality button.active {{
  background: var(--accent-2);
  color: #161204;
  border-color: transparent;
}}
.volume {{
  width: 160px;
  accent-color: var(--accent);
}}
.now-track {{
  margin-top: 18px;
  padding-top: 14px;
  border-top: 1px solid var(--line);
}}
.track-label {{
  color: var(--accent);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
}}
.track-text {{
  margin-top: 6px;
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
  line-height: 1.35;
}}
.track-text.muted {{
  color: var(--muted);
  font-weight: 600;
}}
footer {{
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 12px;
  align-items: center;
  padding: 14px 18px;
  border-top: 1px solid var(--line);
  background: var(--header);
  color: var(--muted);
  font-size: 13px;
}}
audio {{ width: 280px; }}
.modal-backdrop {{
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 22px;
  background: var(--modal-shadow);
  z-index: 20;
}}
.modal-backdrop.hidden {{
  display: none;
}}
.settings-modal {{
  width: min(300px, 100%);
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  padding: 16px;
  box-shadow: 0 20px 48px var(--modal-shadow);
}}
.settings-header {{
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 16px;
}}
.settings-title {{
  font-size: 18px;
  font-weight: 800;
}}
.icon-button {{
  width: 34px;
  height: 34px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel-2);
  color: var(--text);
  font-size: 22px;
  line-height: 1;
  cursor: pointer;
}}
.settings-section {{
  display: grid;
  gap: 8px;
  margin-top: 14px;
}}
.settings-label {{
  color: var(--muted);
  font-size: 12px;
  font-weight: 800;
  text-transform: uppercase;
}}
.theme-select {{
  width: 100%;
  min-height: 38px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--panel);
  color: var(--text);
  padding: 8px 10px;
  font: inherit;
}}
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
    <button id="width-toggle" class="window-toggle">Свернуть</button>
    <input id="search" class="search" type="search" placeholder="Поиск станции">
  </header>
  <main>
    <aside id="list" class="sidebar">{list_html}</aside>
    <section class="content">
      <div class="now">
        <img id="cover" class="cover" alt="">
        <div>
          <div class="title" id="title">Выберите станцию</div>
          <div class="subtitle" id="subtitle">Выберите станцию</div>
          <div class="controls">
            <button id="play" class="control primary">Play</button>
            <button id="stop" class="control">Stop</button>
            <button id="settings" class="control settings-toggle" title="Настройки">⚙</button>
            <button id="favorite" class="control favorite-toggle" title="Избранное">♥</button>
            <input id="volume" class="volume" type="range" min="0" max="1" step="0.01" value="0.9">
          </div>
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
  </main>
</div>
<div id="settings-modal" class="modal-backdrop hidden">
  <div class="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
    <div class="settings-header">
      <div id="settings-title" class="settings-title">Настройки</div>
      <button id="settings-close" class="icon-button" title="Закрыть">×</button>
    </div>
    <div class="settings-section">
      <div class="settings-label">Качество</div>
      <div id="quality" class="quality" aria-label="Качество потока"></div>
    </div>
    <label class="settings-section">
      <span class="settings-label">Тема</span>
      <select id="theme-select" class="theme-select">
        <option value="dark">Темная</option>
        <option value="light">Светлая</option>
      </select>
    </label>
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
var themeSelect = document.getElementById("theme-select");
var track = document.getElementById("track");
var metaTimer = null;
var metaToken = 0;
var lastMetaId = null;

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
    window_width: window.innerWidth,
    window_height: window.innerHeight
  }};
  writeSetting("favorites", JSON.stringify(favorites));
  writeSetting("last_station", payload.last_station || "");
  writeSetting("quality", String(currentQuality));
  writeSetting("volume", String(audio.volume));
  writeSetting("theme", currentTheme);
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
  themeSelect.value = currentTheme;
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
  widthToggle.textContent = window.innerWidth <= 540 ? "Развернуть" : "Свернуть";
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
  for (var index = 0; index < items.length; index += 1) {{
    var station = items[index];
    var button = document.createElement("button");
    var img = document.createElement("img");
    var text = document.createElement("span");
    var name = document.createElement("span");
    var heart = document.createElement("span");

    button.className = "station"
      + (current && current.id === station.id ? " active" : "")
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
    }})
    .catch(function() {{
      if (token === metaToken) setTrackText("Не удалось обновить текущий трек", true);
    }});
}}

function play() {{
  if (!current && stations.length > 0) selectStation(stations[0], false);
  audio.play()
    .then(function() {{ status.textContent = "Играет: " + current.name; }})
    .catch(function(error) {{ status.textContent = "Не удалось запустить поток: " + error.message; }});
}}

function stop() {{
  audio.pause();
  audio.removeAttribute("src");
  audio.load();
  status.textContent = current ? "Остановлено: " + current.name : "Остановлено";
}}

function filteredStations() {{
  var q = search.value.trim().toLowerCase();
  if (!q) return stations;
  return stations.filter(function(station) {{
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

document.getElementById("play").addEventListener("click", play);
document.getElementById("stop").addEventListener("click", stop);
favoriteButton.addEventListener("click", function() {{ toggleFavorite(current); }});
widthToggle.addEventListener("click", postToggleWidth);
settingsButton.addEventListener("click", openSettings);
settingsClose.addEventListener("click", closeSettings);
settingsModal.addEventListener("click", function(event) {{
  if (event.target === settingsModal) closeSettings();
}});
themeSelect.addEventListener("change", function(event) {{
  applyTheme(event.target.value);
  saveAppConfig();
}});
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
audio.volume = isFinite(savedVolume) ? Math.min(1, Math.max(0, savedVolume)) : 0.9;
document.getElementById("volume").value = String(audio.volume);
updateWindowSize();
audio.addEventListener("waiting", function() {{ status.textContent = "Буферизация..."; }});
audio.addEventListener("error", function() {{ status.textContent = "Ошибка аудиопотока. Попробуйте другое качество или станцию."; }});
render();
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
