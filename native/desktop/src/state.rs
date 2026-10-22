//! State shared by all commands.
use crate::sessions::SessionManager;
use anyhow::{Context, ensure};
use opentermius::store::Store;
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

pub struct AppState {
    /// The local configuration; lock briefly and never across `.await`.
    pub store: Mutex<Store>,
    pub sessions: SessionManager,
}

impl AppState {
    pub fn open() -> anyhow::Result<Self> {
        let store = store_from_args(std::env::args_os().skip(1))?;
        let sessions = SessionManager::new(store.path.parent().unwrap_or(std::path::Path::new(".")))?;
        Ok(Self { store: Mutex::new(store), sessions })
    }

    pub fn store(&self) -> Result<MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| opentermius::i18n::t("Configuration lock is unavailable").to_string())
    }
}

fn store_from_args(args: impl IntoIterator<Item = OsString>) -> anyhow::Result<Store> {
    match data_directory(args)? {
        Some(directory) => Store::open(directory.join("opentermius-native.json")),
        None => opentermius::store::open_default(),
    }
}

fn data_directory(args: impl IntoIterator<Item = OsString>) -> anyhow::Result<Option<PathBuf>> {
    let mut args = args.into_iter();
    let mut directory = None;
    while let Some(arg) = args.next() {
        let path = if arg == "--data-dir" {
            let path = args.next().context(opentermius::i18n::t("Invalid data directory"))?;
            ensure!(
                !path.to_string_lossy().starts_with("--"),
                "{}",
                opentermius::i18n::t("Invalid data directory")
            );
            path
        } else if arg.to_string_lossy().starts_with("--data-dir=") {
            let value = arg.to_str().context(opentermius::i18n::t("Invalid data directory"))?;
            OsString::from(&value["--data-dir=".len()..])
        } else {
            continue;
        };
        ensure!(!path.to_string_lossy().trim().is_empty(), "{}", opentermius::i18n::t("Invalid data directory"));
        directory = Some(PathBuf::from(path));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_equals_argument_selects_directory() {
        assert_eq!(
            data_directory([OsString::from("--data-dir=fixture-profile")]).unwrap(),
            Some(PathBuf::from("fixture-profile"))
        );
    }

    #[test]
    fn split_argument_preserves_paths_with_spaces() {
        assert_eq!(
            data_directory([OsString::from("--data-dir"), OsString::from("fixture profile")]).unwrap(),
            Some(PathBuf::from("fixture profile"))
        );
    }

    #[test]
    fn missing_or_empty_directory_is_rejected() {
        for args in [
            vec![OsString::from("--data-dir=")],
            vec![OsString::from("--data-dir= ")],
            vec![OsString::from("--data-dir")],
            vec![OsString::from("--data-dir"), OsString::new()],
            vec![OsString::from("--data-dir"), OsString::from("--other-option")],
        ] {
            assert!(data_directory(args).is_err());
        }
    }

    #[test]
    fn absent_override_keeps_default_selection() {
        assert_eq!(data_directory([]).unwrap(), None);
        assert_eq!(data_directory([OsString::from("--other-option")]).unwrap(), None);
    }

    #[test]
    fn non_ascii_directory_is_preserved() {
        let directory = PathBuf::from("测试资料").join("独立配置");
        let mut argument = OsString::from("--data-dir=");
        argument.push(&directory);
        assert_eq!(data_directory([argument]).unwrap(), Some(directory));
    }

    #[test]
    fn isolated_store_does_not_load_or_modify_other_profile_metadata() {
        let root = tempfile::tempdir().unwrap();
        let other_path = root.path().join("other-profile").join("opentermius-native.json");
        let mut other = Store::open(other_path.clone()).unwrap();
        other.data.snippets.push(opentermius::snippets::Snippet {
            name: "Other profile metadata".into(),
            script: "pwd".into(),
            ..Default::default()
        });
        other.persist().unwrap();
        let original = std::fs::read(&other_path).unwrap();

        let isolated = root.path().join("独立配置");
        let mut argument = OsString::from("--data-dir=");
        argument.push(&isolated);
        let store = store_from_args([argument]).unwrap();
        assert_eq!(store.path, isolated.join("opentermius-native.json"));
        assert_eq!(store.data.version, 2);
        assert!(store.data.hosts.is_empty());
        assert!(store.data.known_hosts.is_empty());
        assert!(store.data.keys.is_empty());
        assert!(store.data.snippets.is_empty());
        store.persist().unwrap();
        assert!(store.path.exists());
        assert_eq!(std::fs::read(&other_path).unwrap(), original);
    }
}
