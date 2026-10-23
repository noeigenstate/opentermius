//! OpenTermius desktop shell: hosts the React UI and exposes the Rust core as Tauri commands.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts;
mod keychain;
mod snippets;
mod logs;
mod sessions;
mod sftp;
mod state;
mod settings;
mod data;
mod updates;
#[cfg(windows)]
mod window_frame;

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
            #[cfg(windows)]
            {
                use tauri::Manager;
                let glass = _app.state::<AppState>().store()?.data.settings.glass_effect;
                if let Some(window) = _app.get_webview_window("main") {
                    window_frame::configure(&window, glass)?;
                }
            }
            Ok(())
        })
        .on_page_load(|_webview, _payload| {
            #[cfg(windows)]
            if _webview.label() == "main" && matches!(_payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = window_frame::update_shape(&_webview.window());
            }
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }) {
                let _ = window_frame::update_shape(window);
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                use tauri::Manager;
                window.state::<AppState>().sessions.shutdown();
            }
        })
        .invoke_handler(tauri::generate_handler![
            hosts::list_hosts, hosts::get_host, hosts::save_host, hosts::delete_host,
            hosts::set_host_favorite, hosts::duplicate_host, hosts::rename_group, hosts::list_keychain_keys,
            hosts::parse_quick_connect,
            sessions::connect_host, sessions::open_local_terminal, sessions::reconnect_session,
            sessions::duplicate_session, sessions::session_input, sessions::session_resize,
            sessions::close_session, sessions::answer_prompt, sessions::list_sessions,
            sftp::local_home, sftp::local_list, sftp::local_mkdir, sftp::local_rename, sftp::local_delete,
            sftp::sftp_list, sftp::sftp_upload, sftp::sftp_download, sftp::sftp_mkdir,
            sftp::sftp_rename, sftp::sftp_delete, sftp::sftp_confirm_delete,
            sftp::sftp_confirm_overwrite, sftp::sftp_cancel_transfer,
            keychain::list_keys, keychain::generate_key, keychain::key_requires_passphrase,
            keychain::import_key, keychain::rename_key, keychain::delete_key, keychain::export_public_key,
            snippets::list_snippets, snippets::save_snippet, snippets::delete_snippet,
            logs::list_logs, logs::clear_logs,
            settings::get_settings, settings::save_settings, settings::get_app_info, settings::open_config_folder,
            data::import_hosts_json, data::detect_xshell_folder, data::import_xshell, data::import_windterm,
            data::export_hosts, data::recovery_generate, data::recovery_restore,
            updates::check_update, updates::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("OpenTermius failed to start");
}
