//! OpenTermius desktop shell: hosts the React UI and exposes the Rust core as Tauri commands.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts;
mod sessions;
mod state;

use state::AppState;

fn main() {
    let state = match AppState::open() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|_app| {
            // Frosted glass: the compositor blurs the desktop behind the transparent window.
            #[cfg(windows)]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
                    let _ = window_vibrancy::apply_blur(&window, Some((238, 241, 246, 90)));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hosts::list_hosts, hosts::get_host, hosts::save_host, hosts::delete_host,
            hosts::duplicate_host, hosts::rename_group, hosts::list_keychain_keys,
            hosts::parse_quick_connect
        ])
        .run(tauri::generate_context!())
        .expect("OpenTermius failed to start");
}
