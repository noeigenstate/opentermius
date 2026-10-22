//! OpenTermius desktop shell: hosts the React UI and exposes the Rust core as Tauri commands.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts;
mod sessions;
mod sftp;
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
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                use tauri::Manager;
                window.state::<AppState>().sessions.shutdown();
            }
        })
        .invoke_handler(tauri::generate_handler![
            hosts::list_hosts, hosts::get_host, hosts::save_host, hosts::delete_host,
            hosts::duplicate_host, hosts::rename_group, hosts::list_keychain_keys,
            hosts::parse_quick_connect,
            sessions::connect_host, sessions::open_local_terminal, sessions::reconnect_session,
            sessions::duplicate_session, sessions::session_input, sessions::session_resize,
            sessions::close_session, sessions::answer_prompt, sessions::list_sessions,
            sftp::local_home, sftp::local_list, sftp::local_mkdir, sftp::local_rename, sftp::local_delete,
            sftp::sftp_list, sftp::sftp_upload, sftp::sftp_download, sftp::sftp_mkdir,
            sftp::sftp_rename, sftp::sftp_delete, sftp::sftp_confirm_delete,
            sftp::sftp_confirm_overwrite, sftp::sftp_cancel_transfer,
        ])
        .run(tauri::generate_context!())
        .expect("OpenTermius failed to start");
}
