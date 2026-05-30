pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn station_list_html(stations: &[crate::stations::Station]) -> String {
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

pub fn build_loading_html() -> String {
    include_str!("ui/loading.html").to_owned()
}

pub fn build_error_html(message: &str) -> String {
    let template = include_str!("ui/error.html");
    template.replace("{message}", &escape_html(message))
}

pub fn build_html(
    stations: &[crate::stations::Station],
    stations_json: &str,
    config: &crate::config::AppConfig,
) -> String {
    let list_html = station_list_html(stations);
    let config_json = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_owned());
    let mut template = include_str!("ui/app.html").to_owned();
    template = template.replace("{list_html}", &list_html);
    template = template.replace("{stations_json}", stations_json);
    template = template.replace("{config_json}", &config_json);
    template = template.replace("{count}", &stations.len().to_string());
    template
}
