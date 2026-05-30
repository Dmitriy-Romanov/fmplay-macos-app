mod config;
mod player;
mod stations;
mod ui;

use config::{AppEvent, app_config_path, handle_ipc_message, load_config};
use stations::load_stations;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use ui::{build_error_html, build_html, build_loading_html};
use wry::WebViewBuilder;

fn main() {
    let config_path = match app_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to get config path: {e:#}");
            return;
        }
    };
    let startup_config = load_config(&config_path).unwrap_or_default();
    let ipc_config_path = config_path.clone();
    let loader_config_path = config_path.clone();
    let resize_config_path = config_path.clone();
    let initial_width = startup_config.window_width.unwrap_or(1040.0).max(300.0);
    let initial_height = startup_config.window_height.unwrap_or(744.0).max(744.0);

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let loader_proxy = event_loop.create_proxy();
    let ipc_proxy = event_loop.create_proxy();
    let window = match WindowBuilder::new()
        .with_title("FMPLAY Radio")
        .with_inner_size(LogicalSize::new(initial_width, initial_height))
        .with_min_inner_size(LogicalSize::new(300.0, 744.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to create window: {e:#}");
            return;
        }
    };

    let webview = match WebViewBuilder::new()
        .with_html(build_loading_html())
        .with_initialization_script("window.__FMPLAY_READY__ = true;")
        .with_ipc_handler(move |request| {
            if let Ok(message) = serde_json::from_str::<config::IpcMessage>(request.body()) {
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
        .build(&window)
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to create webview: {e:#}");
            return;
        }
    };

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
                if let Err(error) = config::save_config(&resize_config_path, &config) {
                    eprintln!("Failed to save window size: {error:#}");
                }
            }
            _ => {}
        }
    });
}
