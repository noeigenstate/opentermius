//! Update metadata and verified installers never come from frontend-supplied URLs.
use opentermius::{
    i18n,
    settings::VERSION,
    update::{self, Release},
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
pub struct UpdateState(Mutex<Option<Release>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    version: Option<String>,
    up_to_date: bool,
    error: Option<String>,
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> Result<UpdateCheck, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UpdateState>();
        let mut available = state
            .0
            .try_lock()
            .map_err(|_| i18n::t("An update operation is already running").to_string())?;
        match update::check(VERSION) {
            Ok(release) => {
                let version = release.as_ref().map(|r| r.version.clone());
                *available = release;
                Ok(UpdateCheck {
                    up_to_date: version.is_none(),
                    version,
                    error: None,
                })
            }
            Err(error) => {
                *available = None;
                Ok(UpdateCheck {
                    version: None,
                    up_to_date: false,
                    error: Some(format!("{error:#}")),
                })
            }
        }
    })
    .await
    .map_err(|_| i18n::t("Update failed").to_string())?
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UpdateState>();
        let available = state
            .0
            .try_lock()
            .map_err(|_| i18n::t("An update operation is already running").to_string())?;
        let release = available
            .as_ref()
            .ok_or_else(|| i18n::t("Check for updates before installing").to_string())?;
        #[cfg(windows)]
        if let Some(install_dir) = update::install_dir() {
            let work = std::env::temp_dir().join("opentermius-update");
            let installer = update::download(release, &work).map_err(|e| format!("{e:#}"))?;
            update::launch_installer(&installer, &install_dir, &work).map_err(|e| format!("{e:#}"))?;
            app.exit(0);
            return Ok(());
        }
        if !valid_release_page(&release.page_url) {
            return Err(i18n::t("Invalid release page URL").into());
        }
        app.opener()
            .open_url(&release.page_url, None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| i18n::t("Update failed").to_string())?
}

fn valid_release_page(url: &str) -> bool {
    url.starts_with("https://github.com/noeigenstate/opentermius/releases/")
        && !url.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_links_must_stay_on_the_project_release_page() {
        assert!(valid_release_page(
            "https://github.com/noeigenstate/opentermius/releases/tag/v1.0.0"
        ));
        for url in [
            "file:///release",
            "https://example.invalid/",
            "https://github.com.evil.invalid/noeigenstate/opentermius/releases/",
            "https://github.com/noeigenstate/opentermius/releases/\n",
        ] {
            assert!(!valid_release_page(url));
        }
    }
}
