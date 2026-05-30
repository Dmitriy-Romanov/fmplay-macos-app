#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Stream {
    pub label: String,
    pub bitrate: u16,
    pub url: String,
}

const FMPLAY_ROOT: &str = "https://fmplay.ru/";

pub fn absolute_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_owned()
    } else {
        format!("{FMPLAY_ROOT}{}", path.trim_start_matches('/'))
    }
}
