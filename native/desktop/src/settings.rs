//! Settings commands share the core's persisted format and normalization rules.
use crate::state::AppState;
use opentermius::{
    i18n,
    settings::{self, Settings},
};
use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.store()?.data.settings.clone().normalized())
}

#[tauri::command]
pub fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let saved = settings::persist(&mut *state.store()?, settings).map_err(|e| e.to_string())?;
    publish(&app, &saved);
    Ok(saved)
}

pub fn publish(app: &tauri::AppHandle, settings: &Settings) {
    i18n::set_language(settings.language);
    let _ = app.emit("settings://changed", settings);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    config_folder: String,
}

#[tauri::command]
pub fn get_app_info(state: tauri::State<'_, AppState>) -> Result<AppInfo, String> {
    let store = state.store()?;
    let folder = store
        .path
        .parent()
        .ok_or_else(|| i18n::t("Invalid configuration directory").to_string())?;
    Ok(AppInfo {
        version: settings::VERSION,
        config_folder: folder.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn open_config_folder(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let folder = {
            let state = app.state::<AppState>();
            let store = state.store()?;
            store
                .path
                .parent()
                .ok_or_else(|| i18n::t("Invalid configuration directory").to_string())?
                .to_path_buf()
        };
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        app.opener()
            .open_path(folder.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| i18n::t("Settings operation failed").to_string())?
}

/// Project links are fixed here; the page only names one, it never supplies a URL.
fn project_link(link: &str) -> Option<&'static str> {
    match link {
        "repository" => Some("https://github.com/noeigenstate/opentermius"),
        "releases" => Some("https://github.com/noeigenstate/opentermius/releases"),
        "issues" => Some("https://github.com/noeigenstate/opentermius/issues"),
        _ => None,
    }
}

#[tauri::command]
pub fn open_project_link(app: tauri::AppHandle, link: String) -> Result<(), String> {
    let url = project_link(&link).ok_or_else(|| i18n::t("Settings operation failed").to_string())?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_links_are_fixed_project_urls() {
        for link in ["repository", "releases", "issues"] {
            assert!(project_link(link).unwrap().starts_with("https://github.com/noeigenstate/opentermius"));
        }
        for link in ["", "https://example.invalid/", "Repository", "../issues"] {
            assert_eq!(project_link(link), None);
        }
    }

    #[test]
    fn command_settings_round_trip_normalizes_without_changing_other_preferences() {
        let source = r#"{"language":"zh-CN","fontSize":30,"scrollbackLines":0,"keepaliveSeconds":0,"connectionTimeoutSeconds":0,"autoUpdate":false,"colorScheme":"monokai"}"#;
        let settings = serde_json::from_str::<Settings>(source)
            .unwrap()
            .normalized();
        assert_eq!(settings.font_size, 24);
        assert_eq!(settings.scrollback_lines, 1000);
        assert_eq!(settings.connection_timeout_seconds, 1);
        assert_eq!(settings.keepalive_seconds, 0);
        assert!(!settings.auto_update);
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        assert_eq!(serde_json::to_value(settings).unwrap()["language"], "zh-CN");
    }

    #[test]
    fn command_settings_reject_invalid_types_and_language() {
        for value in [
            r#"{"language":"other"}"#,
            r#"{"fontSize":-1}"#,
            r#"{"autoUpdate":"yes"}"#,
        ] {
            assert!(serde_json::from_str::<Settings>(value).is_err());
        }
    }
}
