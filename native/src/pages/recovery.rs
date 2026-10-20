use crate::{AppWindow, RecoveryBridge, with_state};
use opentermius::recovery::{self, RestoreMode};
use slint::ComponentHandle;
use zeroize::Zeroizing;

fn message(ui: &AppWindow, result: anyhow::Result<String>) {
    let bridge = ui.global::<RecoveryBridge>();
    bridge.set_busy(false);
    bridge.set_error(result.is_err());
    bridge.set_message(
        match result {
            Ok(text) => crate::i18n::translate(&text),
            Err(e) => crate::i18n::translate(&e.to_string()),
        }
        .into(),
    );
}

pub fn install(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<RecoveryBridge>().on_generate(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let bridge = ui.global::<RecoveryBridge>();
        if bridge.get_busy() {
            return;
        }
        let passphrase = Zeroizing::new(bridge.get_generation_passphrase().to_string());
        let include_secrets = bridge.get_include_secrets();
        bridge.set_generation_passphrase("".into());
        bridge.set_code("".into());
        bridge.set_busy(true);
        let weak = ui.as_weak();
        with_state(|s| {
            let store = s.store.clone();
            s.runtime.spawn_blocking(move || {
                let result = (|| {
                    let data = store
                        .lock()
                        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))?
                        .data
                        .clone();
                    recovery::generate(&data, Some(&passphrase), include_secrets)
                })();
                let _ = weak.upgrade_in_event_loop(move |ui| match result {
                    Ok(code) => {
                        ui.global::<RecoveryBridge>().set_code(code.into());
                        message(&ui, Ok(crate::i18n::t("Recovery code generated. Copy it or save it to a safe location.").into()));
                    }
                    Err(e) => message(&ui, Err(e)),
                });
            });
        });
    });
    let weak = ui.as_weak();
    ui.global::<RecoveryBridge>().on_restore(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let bridge = ui.global::<RecoveryBridge>();
        if bridge.get_busy() {
            return;
        }
        let passphrase = Zeroizing::new(bridge.get_restore_passphrase().to_string());
        let code = Zeroizing::new(bridge.get_pasted_code().to_string());
        let mode = if bridge.get_restore_mode() == 1 {
            RestoreMode::Replace
        } else {
            RestoreMode::Merge
        };
        bridge.set_restore_passphrase("".into());
        bridge.set_busy(true);
        let weak = ui.as_weak();
        with_state(|s| {
            let store = s.store.clone();
            s.runtime.spawn_blocking(move || {
                let result = store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                    .and_then(|mut store| {
                        recovery::restore(&mut store, &code, Some(&passphrase), mode)
                    });
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let result = result.map(|summary| summary.to_string());
                    if result.is_ok() {
                        ui.global::<RecoveryBridge>().set_pasted_code("".into());
                        super::settings::reload(&ui);
                        with_state(|s| s.hosts());
                    }
                    message(&ui, result);
                });
            });
        });
    });
    let weak = ui.as_weak();
    ui.global::<RecoveryBridge>().on_copy(move || {
        if let Some(ui) = weak.upgrade() {
            let result = arboard::Clipboard::new()
                .map_err(anyhow::Error::from)
                .and_then(|mut clipboard| {
                    clipboard
                        .set_text(ui.global::<RecoveryBridge>().get_code().to_string())
                        .map_err(Into::into)
                })
                .map(|_| crate::i18n::t("Recovery code copied. The clipboard contains sensitive credentials; clear it soon.").into());
            message(&ui, result);
        }
    });
    let weak = ui.as_weak();
    ui.global::<RecoveryBridge>().on_save_file(move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(crate::i18n::t("Save recovery code (contains sensitive credentials)"))
                .add_filter(crate::i18n::t("Recovery code"), &["txt"])
                .set_file_name("opentermius-recovery.txt")
                .save_file()
            {
                message(
                    &ui,
                    std::fs::write(path, ui.global::<RecoveryBridge>().get_code().as_bytes())
                        .map(|_| crate::i18n::t("Recovery code saved. Keep it safe.").into())
                        .map_err(Into::into),
                );
            }
        }
    });
    let weak = ui.as_weak();
    ui.global::<RecoveryBridge>().on_close(move || {
        if let Some(ui) = weak.upgrade() {
            let bridge = ui.global::<RecoveryBridge>();
            if bridge.get_busy() {
                return;
            }
            bridge.set_visible(false);
            bridge.set_code("".into());
            bridge.set_pasted_code("".into());
            bridge.set_generation_passphrase("".into());
            bridge.set_restore_passphrase("".into());
            bridge.set_message("".into());
        }
    });
}
