//! OpenTermius desktop shell: hosts the React UI and exposes the Rust core as Tauri commands.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts;
mod keychain;
mod snippets;
mod known_hosts;
mod logs;
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
        .invoke_handler(tauri::generate_handler![hosts::list_hosts,
            keychain::list_keys, keychain::generate_key, keychain::key_requires_passphrase,
            keychain::import_key, keychain::rename_key, keychain::delete_key, keychain::export_public_key,
            snippets::list_snippets, snippets::save_snippet, snippets::delete_snippet,
            known_hosts::list_known_hosts, known_hosts::delete_known_host, known_hosts::import_known_hosts,
            logs::list_logs, logs::clear_logs])
        .run(tauri::generate_context!())
        .expect("OpenTermius failed to start");
}
