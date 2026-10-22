//! Local import/export and portable recovery; decrypted credentials stay in the core.
use crate::{settings, state::AppState};
use opentermius::{i18n, import_windterm, import_xshell, recovery};
use serde::Serialize;
use std::path::PathBuf;
use tauri::Manager;
use zeroize::Zeroizing;

fn input_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if value.trim().is_empty() || value.chars().any(char::is_control) || !path.is_absolute() {
        return Err(i18n::t("Select an absolute local path").into());
    }
    Ok(path)
}

fn restore_mode(value: &str) -> Result<recovery::RestoreMode, String> {
    match value {
        "merge" => Ok(recovery::RestoreMode::Merge),
        "replace" => Ok(recovery::RestoreMode::Replace),
        _ => Err(i18n::t("Invalid recovery restore mode").into()),
    }
}

#[tauri::command]
pub async fn import_hosts_json(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let path = input_path(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let count = state.store()?.import(&path).map_err(|e| e.to_string())?;
        Ok(opentermius::tf!("Imported {n} hosts", "n" => count.to_string()))
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[tauri::command]
pub fn detect_xshell_folder() -> Option<String> {
    import_xshell::default_sessions_dir().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn import_xshell(app: tauri::AppHandle, path: Option<String>) -> Result<String, String> {
    let path = path.map(|p| input_path(&p)).transpose()?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = path
            .or_else(import_xshell::default_sessions_dir)
            .ok_or_else(|| {
                i18n::t("No Xshell sessions folder was found; select a folder").to_string()
            })?;
        if !path.is_dir() {
            return Err(i18n::t("No Xshell sessions folder was found; select a folder").into());
        }
        let scanned = import_xshell::scan(path);
        let state = app.state::<AppState>();
        import_xshell::merge(&mut *state.store()?, scanned)
            .map(|s| s.message())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindTermResult {
    files: Vec<String>,
    summary: Option<String>,
}

fn windterm_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() || paths.len() > 256 {
        return Err(i18n::t("Select between 1 and 256 WindTerm session files").into());
    }
    let mut paths = paths
        .iter()
        .map(|p| input_path(p))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    if paths
        .iter()
        .any(|p| !p.is_file() || p.file_name() != Some(std::ffi::OsStr::new("user.sessions")))
    {
        return Err(i18n::t("Select WindTerm user.sessions files").into());
    }
    Ok(paths)
}

#[tauri::command]
pub async fn import_windterm(
    app: tauri::AppHandle,
    paths: Option<Vec<String>>,
) -> Result<WindTermResult, String> {
    // No paths means detection only. Reading and merging require explicit confirmed files.
    tauri::async_runtime::spawn_blocking(move || {
        let home = dirs::home_dir()
            .ok_or_else(|| i18n::t("Cannot find the home directory").to_string())?;
        let Some(paths) = paths else {
            return Ok(WindTermResult {
                files: import_windterm::detect(&home)
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                summary: None,
            });
        };
        let paths = windterm_paths(paths)?;
        let mut scanned = import_xshell::ScanResult::default();
        for path in &paths {
            let mut one = import_windterm::scan_file(path, &home).map_err(|e| e.to_string())?;
            scanned.hosts.append(&mut one.hosts);
            scanned.skipped.append(&mut one.skipped);
        }
        let state = app.state::<AppState>();
        let summary = import_xshell::merge(&mut *state.store()?, scanned)
            .map_err(|e| e.to_string())?
            .message();
        Ok(WindTermResult {
            files: paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            summary: Some(summary),
        })
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[tauri::command]
pub async fn export_hosts(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let path = input_path(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.store()?.export(&path).map_err(|e| e.to_string())?;
        Ok(i18n::t("Host configuration exported (without credentials)").into())
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[tauri::command]
pub async fn recovery_generate(
    app: tauri::AppHandle,
    passphrase: Option<String>,
    include_keys: bool,
) -> Result<String, String> {
    let passphrase = passphrase.map(Zeroizing::new);
    tauri::async_runtime::spawn_blocking(move || {
        let data = app.state::<AppState>().store()?.data.clone();
        recovery::generate(&data, passphrase.as_ref().map(|p| p.as_str()), include_keys)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[tauri::command]
pub async fn recovery_restore(
    app: tauri::AppHandle,
    code: String,
    passphrase: Option<String>,
    mode: String,
) -> Result<String, String> {
    let mode = restore_mode(&mode)?;
    let code = Zeroizing::new(code);
    let passphrase = passphrase.map(Zeroizing::new);
    tauri::async_runtime::spawn_blocking(move || {
        let (summary, restored) = {
            let state = app.state::<AppState>();
            let mut store = state.store()?;
            let summary = recovery::restore(
                &mut store,
                &code,
                passphrase.as_ref().map(|p| p.as_str()),
                mode,
            )
            .map_err(|e| e.to_string())?;
            (
                summary.to_string(),
                store.data.settings.clone().normalized(),
            )
        };
        settings::publish(&app, &restored);
        Ok(summary)
    })
    .await
    .map_err(|_| i18n::t("Data operation failed").to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn paths_and_restore_modes_reject_invalid_input() {
        for value in ["", " ", "relative.json", "bad\0path", "\n"] {
            assert!(input_path(value).is_err());
        }
        assert!(input_path(Path::new(env!("CARGO_MANIFEST_DIR")).to_str().unwrap()).is_ok());
        assert!(restore_mode("merge").is_ok());
        assert!(restore_mode("replace").is_ok());
        for value in ["", "Replace", "delete", "merge "] {
            assert!(restore_mode(value).is_err());
        }
    }

    #[test]
    fn windterm_requires_a_bounded_nonempty_selection_of_session_files() {
        assert!(windterm_paths(vec![]).is_err());
        assert!(windterm_paths(vec!["unused".into(); 257]).is_err());
        assert!(windterm_paths(vec![env!("CARGO_MANIFEST_DIR").into()]).is_err());
    }
}
