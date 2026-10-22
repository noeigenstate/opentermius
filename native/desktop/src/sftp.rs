//! Per-session SFTP commands. The session pump owns all remote result events.
use crate::state::AppState;
use opentermius::{i18n::t, ssh};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    modified: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalListing {
    path: String,
    parent: Option<String>,
    entries: Vec<LocalEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDeletion {
    deleted: bool,
    files: u64,
    dirs: u64,
    bytes: u64,
}

fn component(name: &str, local: bool) -> Result<(), String> {
    if if local {
        ssh::safe_local_component(name)
    } else {
        ssh::safe_remote_component(name)
    } {
        Ok(())
    } else {
        Err(t("Invalid file name").into())
    }
}

fn remote_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 32768 || path.chars().any(char::is_control) {
        return Err(t("Invalid remote path").into());
    }
    Ok(())
}

fn local_root(path: &str) -> Result<PathBuf, String> {
    ssh::canonical_local_root(Path::new(path)).map_err(|error| error.to_string())
}

fn child(path: &str, name: &str) -> Result<PathBuf, String> {
    component(name, true)?;
    Ok(local_root(path)?.join(name))
}

fn unused_destination(path: &Path) -> Result<(), String> {
    // symlink_metadata also detects dangling links, which must never be replaced.
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err(t("Destination name already exists").into()),
    }
}

fn list_local(path: &str, show_hidden: bool) -> Result<LocalListing, String> {
    let root = local_root(path)?;
    let entries = ssh::read_local_dir(&root).map_err(|error| error.to_string())?;
    let entries = entries
        .into_iter()
        .filter(|entry| {
            if show_hidden {
                return true;
            }
            if ssh::is_hidden_name(&entry.name) {
                return false;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if entry
                    .path
                    .metadata()
                    .is_ok_and(|meta| meta.file_attributes() & 0x2 != 0)
                {
                    return false;
                }
            }
            true
        })
        .map(|entry| LocalEntry {
            name: entry.name,
            path: ssh::display_local_path(&entry.path),
            size: entry.size,
            is_dir: entry.is_dir,
            modified: entry.modified,
        })
        .collect();
    Ok(LocalListing {
        path: ssh::display_local_path(&root),
        parent: root.parent().map(ssh::display_local_path),
        entries,
    })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| t("Cannot read local folder").to_string())?
}

#[tauri::command]
pub async fn local_home() -> Result<String, String> {
    blocking(|| {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .ok_or_else(|| t("Select a local folder").to_string())?;
        local_root(&home.to_string_lossy()).map(|path| ssh::display_local_path(&path))
    })
    .await
}

#[tauri::command]
pub async fn local_list(path: String, show_hidden: bool) -> Result<LocalListing, String> {
    blocking(move || list_local(&path, show_hidden)).await
}

#[tauri::command]
pub async fn local_mkdir(path: String, name: String) -> Result<(), String> {
    blocking(move || {
        let destination = child(&path, &name)?;
        std::fs::create_dir(destination).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn local_rename(path: String, from: String, to: String) -> Result<(), String> {
    blocking(move || {
        let source = child(&path, &from)?;
        component(&to, true)?;
        let destination = source.with_file_name(to);
        unused_destination(&destination)?;
        std::fs::rename(source, destination).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn local_delete(
    path: String,
    name: String,
    confirmed: bool,
) -> Result<LocalDeletion, String> {
    blocking(move || {
        let target = child(&path, &name)?;
        if confirmed {
            ssh::delete_local_tree(&target).map_err(|error| error.to_string())?;
            return Ok(LocalDeletion {
                deleted: true,
                files: 0,
                dirs: 0,
                bytes: 0,
            });
        }
        let summary = ssh::local_tree_summary(&target).map_err(|error| error.to_string())?;
        Ok(LocalDeletion {
            deleted: false,
            files: summary.files,
            dirs: summary.dirs,
            bytes: summary.bytes,
        })
    })
    .await
}

#[tauri::command]
pub fn sftp_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<(), String> {
    remote_path(&path)?;
    state.sessions.send(&session_id, ssh::Command::List(path))
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    local_root: String,
    name: String,
    transfer: u64,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&name, false)?;
    let command =
        blocking(move || upload_command(&local_root, &name, directory, transfer, false)).await?;
    state.sessions.send(&session_id, command)
}

fn upload_command(
    root: &str,
    name: &str,
    directory: String,
    transfer: u64,
    overwrite: bool,
) -> Result<ssh::Command, String> {
    let local = child(root, name)?;
    let local_root = local
        .parent()
        .ok_or_else(|| t("Invalid local file path").to_string())?
        .to_path_buf();
    let meta = std::fs::symlink_metadata(&local).map_err(|error| error.to_string())?;
    if meta.is_dir() {
        Ok(ssh::Command::UploadTree {
            local_root,
            transfer,
            directory,
            local,
            overwrite,
        })
    } else {
        Ok(ssh::Command::UploadFile {
            local_root,
            transfer,
            directory,
            local,
            overwrite,
        })
    }
}

#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    name: String,
    local_root: String,
    folder: bool,
    transfer: u64,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&name, false)?;
    component(&name, true)?;
    let command = blocking(move || {
        let local_root = self::local_root(&local_root)?;
        let remote = format!("{}/{}", directory.trim_end_matches('/'), name);
        if folder {
            Ok(ssh::Command::DownloadTree {
                local_dir: local_root.clone(),
                local_root,
                transfer,
                remote,
            })
        } else {
            let local = local_root.join(name);
            unused_destination(&local)?;
            Ok(ssh::Command::DownloadFile {
                local_root,
                transfer,
                remote,
                local,
            })
        }
    })
    .await?;
    state.sessions.send(&session_id, command)
}

#[tauri::command]
pub fn sftp_mkdir(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    name: String,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&name, false)?;
    state
        .sessions
        .send(&session_id, ssh::Command::CreateDir { directory, name })
}

#[tauri::command]
pub fn sftp_rename(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    from: String,
    to: String,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&from, false)?;
    component(&to, false)?;
    state.sessions.send(
        &session_id,
        ssh::Command::Rename {
            directory,
            from,
            to,
        },
    )
}

#[tauri::command]
pub fn sftp_delete(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    name: String,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&name, false)?;
    state.sessions.send(
        &session_id,
        ssh::Command::Delete {
            directory,
            name,
            confirmed: false,
        },
    )
}

#[tauri::command]
pub fn sftp_confirm_delete(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    name: String,
) -> Result<(), String> {
    remote_path(&directory)?;
    component(&name, false)?;
    state.sessions.send(
        &session_id,
        ssh::Command::Delete {
            directory,
            name,
            confirmed: true,
        },
    )
}

/// Resume exactly the paused upload described by sftp://overwrite.
#[tauri::command]
pub async fn sftp_confirm_overwrite(
    state: State<'_, AppState>,
    session_id: String,
    directory: String,
    local_root: String,
    local: String,
    transfer: u64,
    folder: bool,
) -> Result<(), String> {
    remote_path(&directory)?;
    let command = blocking(move || {
        let original_root = Path::new(&local_root);
        let relative = Path::new(&local)
            .strip_prefix(original_root)
            .map_err(|_| t("Local path is outside the selected folder").to_string())?;
        let mut parts = relative.components();
        let Some(std::path::Component::Normal(name)) = parts.next() else {
            return Err(t("Invalid local file name").into());
        };
        if parts.next().is_some() {
            return Err(t("Invalid local file name").into());
        }
        let name = name
            .to_str()
            .ok_or_else(|| t("Invalid local file name").to_string())?;
        component(name, true)?;
        component(name, false)?;
        let root = self::local_root(&local_root)?;
        let local = root.join(name);
        Ok(if transfer == 0 {
            ssh::Command::Upload {
                local_root: root,
                directory,
                local,
                overwrite: true,
            }
        } else if folder {
            ssh::Command::UploadTree {
                local_root: root,
                transfer,
                directory,
                local,
                overwrite: true,
            }
        } else {
            ssh::Command::UploadFile {
                local_root: root,
                transfer,
                directory,
                local,
                overwrite: true,
            }
        })
    })
    .await?;
    state.sessions.send(&session_id, command)
}

#[tauri::command]
pub fn sftp_cancel_transfer(
    state: State<'_, AppState>,
    session_id: String,
    transfer: u64,
) -> Result<(), String> {
    state
        .sessions
        .send(&session_id, ssh::Command::CancelTransfer { transfer })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        // Keep all fixtures on the workspace drive, even when TEMP points at C:.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test-output/sftp")
            .join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn names_cannot_escape_or_target_windows_devices() {
        for name in ["", ".", "..", "../escape", "a/b", "a\\b", "x\n"] {
            assert!(component(name, true).is_err());
            assert!(component(name, false).is_err());
        }
        for name in ["NUL", "COM1.txt", "a:stream", "trailing.", "trailing "] {
            assert!(component(name, true).is_err());
        }
        assert!(component("report.txt", true).is_ok());
        assert!(component("report.txt", false).is_ok());
        for path in ["", "x\0", "x\n"] {
            assert!(remote_path(path).is_err());
        }
    }

    #[test]
    fn listing_filters_hidden_and_preserves_metadata() {
        let root = fixture();
        std::fs::write(root.join("report.txt"), b"data").unwrap();
        std::fs::write(root.join(".hidden"), b"secret").unwrap();
        std::fs::create_dir(root.join("folder")).unwrap();
        let listing = list_local(root.to_str().unwrap(), false).unwrap();
        assert_eq!(listing.entries.len(), 2);
        let file = listing
            .entries
            .iter()
            .find(|entry| entry.name == "report.txt")
            .unwrap();
        assert_eq!(file.size, 4);
        assert!(!file.is_dir);
        assert!(file.modified > 0);
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.name == "folder" && entry.is_dir)
        );
        assert_eq!(
            list_local(root.to_str().unwrap(), true)
                .unwrap()
                .entries
                .len(),
            3
        );
        assert!(listing.parent.is_some());
        assert!(list_local(root.join("report.txt").to_str().unwrap(), true).is_err());
        assert!(child(root.to_str().unwrap(), "../escape").is_err());
        assert!(unused_destination(&root.join("report.txt")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn local_mutations_require_safe_names_and_explicit_delete_confirmation() {
        let root = fixture();
        let path = root.to_string_lossy().to_string();
        assert!(
            local_mkdir(path.clone(), "../outside".into())
                .await
                .is_err()
        );
        local_mkdir(path.clone(), "folder".into()).await.unwrap();
        std::fs::write(root.join("folder/item.txt"), b"data").unwrap();
        std::fs::write(root.join("existing.txt"), b"keep").unwrap();
        assert!(
            local_rename(path.clone(), "folder".into(), "existing.txt".into())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(root.join("existing.txt")).unwrap(), b"keep");
        assert!(
            local_rename(path.clone(), "folder".into(), "NUL".into())
                .await
                .is_err()
        );
        local_rename(path.clone(), "folder".into(), "renamed".into())
            .await
            .unwrap();
        let preview = local_delete(path.clone(), "renamed".into(), false)
            .await
            .unwrap();
        assert!(!preview.deleted);
        assert_eq!((preview.files, preview.dirs, preview.bytes), (1, 0, 4));
        assert!(root.join("renamed/item.txt").exists());
        assert!(local_delete(path.clone(), "..".into(), true).await.is_err());
        assert!(
            local_delete(path, "renamed".into(), true)
                .await
                .unwrap()
                .deleted
        );
        assert!(!root.join("renamed").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upload_uses_canonical_root_and_file_or_tree_command() {
        let root = fixture();
        std::fs::write(root.join("item.txt"), b"data").unwrap();
        std::fs::create_dir(root.join("folder")).unwrap();
        let path = root.to_str().unwrap();
        match upload_command(path, "item.txt", "/destination".into(), 17, false).unwrap() {
            ssh::Command::UploadFile {
                local_root,
                local,
                transfer,
                overwrite,
                ..
            } => {
                assert_eq!(local_root, root.canonicalize().unwrap());
                assert_eq!(local, local_root.join("item.txt"));
                assert_eq!(transfer, 17);
                assert!(!overwrite);
            }
            _ => panic!("expected file upload"),
        }
        assert!(matches!(
            upload_command(path, "folder", "/destination".into(), 18, true).unwrap(),
            ssh::Command::UploadTree {
                transfer: 18,
                overwrite: true,
                ..
            }
        ));
        assert!(upload_command(path, "../outside", "/destination".into(), 19, false).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
