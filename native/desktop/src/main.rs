//! OpenTermius desktop shell: hosts the React UI and exposes the Rust core as Tauri commands.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts;
mod sessions;
mod state;
mod settings;
mod data;
mod updates;

use state::AppState;

fn main() {
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).is_some_and(|arg| arg == "--apply-update") {
            if let Err(error) = opentermius::update::apply(&args[2..]) {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
            return;
        }
    }
    let state = match AppState::open() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };
    if let Ok(store) = state.store() {
        opentermius::i18n::set_language(store.data.settings.language);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(updates::UpdateState::default())
        .manage(state)
        .setup(|_app| {
            // Frosted glass: the compositor blurs the desktop behind the transparent window.
            #[cfg(windows)]
            {
                use tauri::Manager;
                let glass = _app.state::<AppState>().store()?.data.settings.glass_effect;
                if let Some(window) = _app.get_webview_window("main") {
                    if glass {
                        let _ = window_vibrancy::apply_blur(&window, Some((238, 241, 246, 90)));
                    } else {
                        window.set_background_color(Some(tauri::window::Color(238, 241, 246, 255)))?;
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![hosts::list_hosts,
            settings::get_settings, settings::save_settings, settings::get_app_info, settings::open_config_folder,
            data::import_hosts_json, data::detect_xshell_folder, data::import_xshell, data::import_windterm, data::export_hosts,
            data::recovery_generate, data::recovery_restore, updates::check_update, updates::install_update])
        .run(tauri::generate_context!())
        .expect("OpenTermius failed to start");
}
