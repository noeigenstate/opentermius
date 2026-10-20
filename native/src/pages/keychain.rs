use crate::{AppWindow, KeyRow, KeychainBridge, with_state};
use anyhow::Context;
use opentermius::keychain::{self, KeyEntry, KeyKind};
use slint::ComponentHandle;
use std::{cell::RefCell, fs, path::PathBuf, rc::Rc};
use zeroize::Zeroize;

fn key_row(entry: &KeyEntry) -> KeyRow {
    KeyRow {
        id: entry.id.clone().into(),
        label: entry.label.clone().into(),
        key_type: entry.display_type().into(),
        fingerprint: entry.fingerprint.clone().into(),
        public_key: entry.public_key.clone().into(),
        created: entry.display_created().into(),
    }
}

fn set_message(ui: &AppWindow, text: impl AsRef<str>, error: bool) {
    let bridge = ui.global::<KeychainBridge>();
    bridge.set_message(text.as_ref().into());
    bridge.set_message_error(error);
}

fn keys_and_selected(selected: &str) -> (Vec<KeyRow>, Option<KeyEntry>) {
    with_state(|state| {
        let Ok(store) = state.store.lock() else {
            return (Vec::new(), None);
        };
        let selected = store
            .data
            .keys
            .iter()
            .find(|key| key.id == selected)
            .cloned()
            .or_else(|| store.data.keys.last().cloned());
        let rows = store.data.keys.iter().rev().map(key_row).collect();
        (rows, selected)
    })
    .unwrap_or((Vec::new(), None))
}

fn refresh(ui: &AppWindow) {
    let bridge = ui.global::<KeychainBridge>();
    let selected_id = bridge.get_selected_id().to_string();
    let (rows, selected) = keys_and_selected(&selected_id);
    let id = selected
        .as_ref()
        .map(|key| key.id.clone())
        .unwrap_or_default();
    let rename = selected
        .as_ref()
        .map(|key| key.label.clone())
        .unwrap_or_default();
    bridge.set_keys(crate::model(rows));
    bridge.set_selected_id(id.into());
    bridge.set_selected(selected.as_ref().map(key_row).unwrap_or_default());
    bridge.set_rename_text(rename.into());
}

fn entry(id: &str) -> Option<KeyEntry> {
    with_state(|state| {
        state
            .store
            .lock()
            .ok()
            .and_then(|store| store.key_entry(id).cloned())
    })
    .flatten()
}

fn commit_entry(ui: &AppWindow, entry: KeyEntry, success: &str) {
    let result = with_state(|state| {
        let mut store = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("配置锁不可用"))?;
        let id = entry.id.clone();
        store.add_key(entry)?;
        Ok::<String, anyhow::Error>(id)
    });
    match result {
        Some(Ok(id)) => {
            ui.global::<KeychainBridge>().set_selected_id(id.into());
            refresh(ui);
            set_message(ui, success, false);
        }
        Some(Err(error)) => set_message(ui, format!("{error:#}"), true),
        None => set_message(ui, "界面状态不可用", true),
    }
}

fn file_name(label: &str, suffix: &str) -> String {
    let name: String = label
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim().trim_matches('_');
    format!("{}{suffix}", if name.is_empty() { "ssh-key" } else { name })
}

pub fn install(ui: &AppWindow) {
    refresh(ui);

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>()
        .on_generate(move |label, kind| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some((weak_ui, handle, store)) = with_state(|state| {
                (
                    state.ui.clone(),
                    state.runtime.handle().clone(),
                    state.store.clone(),
                )
            }) else {
                set_message(&ui, "界面状态不可用", true);
                return;
            };
            let label = label.to_string();
            let bridge = ui.global::<KeychainBridge>();
            bridge.set_busy(true);
            set_message(&ui, "正在生成密钥…", false);
            handle.spawn_blocking(move || {
                let result =
                    keychain::generate(&label, KeyKind::from_index(kind)).and_then(|entry| {
                        let id = entry.id.clone();
                        let mut store =
                            store.lock().map_err(|_| anyhow::anyhow!("配置锁不可用"))?;
                        store.add_key(entry)?;
                        Ok::<String, anyhow::Error>(id)
                    });
                let _ = weak_ui.upgrade_in_event_loop(move |ui| {
                    ui.global::<KeychainBridge>().set_busy(false);
                    match result {
                        Ok(id) => {
                            ui.global::<KeychainBridge>().set_selected_id(id.into());
                            refresh(&ui);
                            set_message(&ui, "密钥已生成", false);
                        }
                        Err(error) => set_message(&ui, format!("{error:#}"), true),
                    }
                });
            });
        });

    let pending: Rc<RefCell<Option<(PathBuf, String)>>> = Rc::new(RefCell::new(None));
    let weak = ui.as_weak();
    let pending_import = pending.clone();
    ui.global::<KeychainBridge>().on_import_key(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import SSH private key")
            .pick_file()
        else {
            return;
        };
        let label = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported key")
            .to_string();
        match keychain::import_file(&path, &label, None) {
            Ok(entry) => commit_entry(&ui, entry, "密钥已导入"),
            Err(error) => {
                if keychain::file_requires_passphrase(&path) {
                    *pending_import.borrow_mut() = Some((path, label));
                    let bridge = ui.global::<KeychainBridge>();
                    bridge.set_passphrase("".into());
                    bridge.set_passphrase_error("".into());
                    bridge.set_passphrase_visible(true);
                    set_message(&ui, "该私钥受口令保护，请输入口令", false);
                } else {
                    set_message(&ui, format!("{error:#}"), true);
                }
            }
        }
    });

    let weak = ui.as_weak();
    let pending_import = pending.clone();
    ui.global::<KeychainBridge>()
        .on_submit_passphrase(move |passphrase| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some((path, label)) = pending_import.borrow_mut().take() else {
                ui.global::<KeychainBridge>().set_passphrase_visible(false);
                return;
            };
            let mut passphrase = passphrase.to_string();
            let result = keychain::import_file(&path, &label, Some(&passphrase));
            passphrase.zeroize();
            match result {
                Ok(entry) => {
                    let bridge = ui.global::<KeychainBridge>();
                    bridge.set_passphrase("".into());
                    bridge.set_passphrase_error("".into());
                    bridge.set_passphrase_visible(false);
                    commit_entry(&ui, entry, "密钥已导入");
                }
                Err(error) => {
                    *pending_import.borrow_mut() = Some((path, label));
                    ui.global::<KeychainBridge>()
                        .set_passphrase_error(format!("{error:#}").into());
                    set_message(&ui, "私钥口令无效", true);
                }
            }
        });

    let weak = ui.as_weak();
    let pending_import = pending;
    ui.global::<KeychainBridge>().on_cancel_passphrase(move || {
        pending_import.borrow_mut().take();
        if let Some(ui) = weak.upgrade() {
            let bridge = ui.global::<KeychainBridge>();
            bridge.set_passphrase("".into());
            bridge.set_passphrase_error("".into());
            bridge.set_passphrase_visible(false);
            set_message(&ui, "", false);
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_select_key(move |id| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<KeychainBridge>().set_selected_id(id);
            refresh(&ui);
            set_message(&ui, "", false);
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_rename(move |id, label| {
        let result = with_state(|state| {
            state
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("配置锁不可用"))?
                .rename_key(&id, &label)
        });
        if let Some(ui) = weak.upgrade() {
            match result {
                Some(Ok(())) => {
                    ui.global::<KeychainBridge>().set_selected_id(id);
                    refresh(&ui);
                    set_message(&ui, "名称已更新", false);
                }
                Some(Err(error)) => set_message(&ui, format!("{error:#}"), true),
                None => set_message(&ui, "界面状态不可用", true),
            }
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_delete_key(move |id| {
        let result = with_state(|state| {
            state
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("配置锁不可用"))?
                .delete_key(&id)
        });
        if let Some(ui) = weak.upgrade() {
            match result {
                Some(Ok(())) => {
                    ui.global::<KeychainBridge>().set_selected_id("".into());
                    refresh(&ui);
                    set_message(&ui, "密钥已删除", false);
                }
                Some(Err(error)) => set_message(&ui, format!("{error:#}"), true),
                None => set_message(&ui, "界面状态不可用", true),
            }
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_copy_public(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(entry) = entry(&id) else {
            set_message(&ui, "Keychain 密钥不存在", true);
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            let mut clipboard = arboard::Clipboard::new().context("无法访问剪贴板")?;
            clipboard
                .set_text(entry.public_key)
                .context("复制公钥失败")?;
            Ok(())
        })();
        match result {
            Ok(()) => set_message(&ui, "公钥已复制到剪贴板", false),
            Err(error) => set_message(&ui, format!("{error:#}"), true),
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_save_public(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(entry) = entry(&id) else {
            set_message(&ui, "Keychain 密钥不存在", true);
            return;
        };
        let name = file_name(&entry.label, ".pub");
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save SSH public key")
            .set_file_name(name.as_str())
            .add_filter("Public key", &["pub", "txt"])
            .save_file()
        else {
            return;
        };
        match fs::write(&path, format!("{}\n", entry.public_key)) {
            Ok(()) => set_message(&ui, format!("公钥已保存到 {}", path.display()), false),
            Err(error) => set_message(&ui, format!("保存公钥失败：{error}"), true),
        }
    });

    let weak = ui.as_weak();
    ui.global::<KeychainBridge>().on_save_private(move |id| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(entry) = entry(&id) else {
            set_message(&ui, "Keychain 密钥不存在", true);
            return;
        };
        let confirmed = rfd::MessageDialog::new()
            .set_title("Export private key")
            .set_description(
                "The exported file is not protected by Windows DPAPI. Save it only in a trusted location and keep it secure.",
            )
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show()
            == rfd::MessageDialogResult::Yes;
        if !confirmed {
            return;
        }
        let name = file_name(&entry.label, "_key");
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save SSH private key")
            .set_file_name(name.as_str())
            .add_filter("Private key", &["pem", "key", "txt"])
            .save_file()
        else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            let text = entry.export_private().context("无法导出私钥")?;
            fs::write(&path, text.as_bytes()).context("写入私钥文件失败")?;
            Ok(())
        })();
        match result {
            Ok(()) => set_message(&ui, format!("私钥已导出到 {}", path.display()), false),
            Err(error) => set_message(&ui, format!("{error:#}"), true),
        }
    });
}
