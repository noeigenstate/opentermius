//! Known Hosts section: lists pinned fingerprints, imports OpenSSH
//! `known_hosts` files and forgets trust so the next connection re-confirms.
use crate::{AppWindow, KnownHostRow, KnownHostsBridge, State, model, with_state};
use anyhow::Context;
use opentermius::store;
use slint::{ComponentHandle, SharedString};
use std::collections::BTreeMap;

pub fn install(ui: &AppWindow) {
    let bridge = ui.global::<KnownHostsBridge>();
    let weak = ui.as_weak();
    bridge.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui);
        }
    });
    let weak = ui.as_weak();
    bridge.on_page_shown(move || {
        if let Some(ui) = weak.upgrade() {
            set_status(&ui, "", false);
            refresh(&ui);
        }
    });
    let weak = ui.as_weak();
    bridge.on_import_known_hosts(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title(crate::i18n::t("Import OpenSSH known_hosts"))
            .add_filter("known_hosts", &["known_hosts", "txt", "pub"])
            .pick_file()
        else {
            return;
        };
        let result = with_state(|state| import_file(state, &path));
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match result {
            Some(Ok((parsed, outcome))) => {
                refresh(&ui);
                set_status(&ui, import_summary(&parsed, outcome), false);
            }
            Some(Err(error)) => set_status(&ui, opentermius::tf!("Import failed: {error:#}", "error:#" => format!("{:#}", error)), true),
            None => set_status(&ui, crate::i18n::t("Configuration is not ready; cannot import"), true),
        }
    });
    let weak = ui.as_weak();
    bridge.on_remove_host(move |host, port| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Ok(port) = u16::try_from(port) else {
            set_status(&ui, crate::i18n::t("Invalid port; cannot remove this fingerprint"), true);
            return;
        };
        let result = with_state(|state| {
            state
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))
                .and_then(|mut store| store.forget(host.as_str(), port))
        });
        match result {
            Some(Ok(true)) => {
                refresh(&ui);
                set_status(
                    &ui,
                    opentermius::tf!("Removed host fingerprint for {host}:{port}; it will be confirmed on next connection", "host" => format!("{}", host), "port" => format!("{}", port)),
                    false,
                );
            }
            Some(Ok(false)) => {
                refresh(&ui);
                set_status(&ui, opentermius::tf!("Host fingerprint for {host}:{port} no longer exists", "host" => format!("{}", host), "port" => format!("{}", port)), false);
            }
            Some(Err(error)) => set_status(&ui, error.to_string(), true),
            None => set_status(&ui, crate::i18n::t("Configuration is not ready; cannot remove"), true),
        }
    });
    refresh(ui);
}

fn import_file(
    state: &mut State,
    path: &std::path::Path,
) -> anyhow::Result<(store::ParsedKnownHosts, store::KnownHostsMerge)> {
    let metadata = std::fs::metadata(path).context(crate::i18n::t("Cannot read import file"))?;
    anyhow::ensure!(metadata.len() <= 5 * 1024 * 1024, "{}", crate::i18n::t("Import file must not exceed 5 MB"));
    let bytes = std::fs::read(path).context(crate::i18n::t("Cannot read import file"))?;
    let parsed = store::parse_known_hosts(&String::from_utf8_lossy(&bytes));
    let outcome = state
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration lock is unavailable")))?
        .import_known_hosts(&parsed.entries)?;
    Ok((parsed, outcome))
}

fn import_summary(parsed: &store::ParsedKnownHosts, outcome: store::KnownHostsMerge) -> String {
    if parsed.entries.is_empty() {
        return if parsed.skipped > 0 {
            opentermius::tf!("No host fingerprints imported; skipped {} invalid or unsupported records", "0" => format!("{}", parsed.skipped))
        } else {
            crate::i18n::t("File does not contain known_hosts records").to_string()
        };
    }
    let mut message = opentermius::tf!("Imported {} host fingerprints", "0" => format!("{}", outcome.added));
    if outcome.unchanged > 0 {
        message.push_str(&opentermius::tf!(", {} already existed", "0" => format!("{}", outcome.unchanged)));
    }
    if outcome.conflicts > 0 {
        message.push_str(&opentermius::tf!(", {} fingerprint conflicts kept unchanged", "0" => format!("{}", outcome.conflicts)));
    }
    if parsed.skipped > 0 {
        message.push_str(&opentermius::tf!(", {} skipped", "0" => format!("{}", parsed.skipped)));
    }
    message
}

pub(super) fn refresh(ui: &AppWindow) {
    let bridge = ui.global::<KnownHostsBridge>();
    let search = bridge.get_search().to_lowercase();
    let (rows, total) = with_state(|state| {
        let Ok(store) = state.store.lock() else {
            return (Vec::new(), 0);
        };
        let mut usage = BTreeMap::<String, Vec<String>>::new();
        for profile in &store.data.hosts {
            usage
                .entry(store::key(&profile.host, profile.port))
                .or_default()
                .push(profile.name.clone());
        }
        let total = store.data.known_hosts.len() as i32;
        let mut rows = store
            .data
            .known_hosts
            .iter()
            .filter_map(|(key, fingerprint)| {
                let (host, port) = store::parse_key(key).unwrap_or_else(|| (key.clone(), 0));
                if !search.is_empty()
                    && !host.to_lowercase().contains(&search)
                    && !fingerprint.to_lowercase().contains(&search)
                {
                    return None;
                }
                let names = usage.get(key).map(|names| names.as_slice()).unwrap_or_default();
                Some(KnownHostRow {
                    host: host.into(),
                    port: port as i32,
                    fingerprint: fingerprint.clone().into(),
                    usage: usage_label(names).into(),
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            a.host
                .to_lowercase()
                .cmp(&b.host.to_lowercase())
                .then(a.port.cmp(&b.port))
        });
        (rows, total)
    })
    .unwrap_or_default();
    bridge.set_entries(model(rows));
    bridge.set_total(total);
}

fn usage_label(names: &[String]) -> String {
    match names {
        [] => crate::i18n::t("Not in saved hosts").into(),
        [only] => only.clone(),
        [first, second] => format!("{first}, {second}"),
        [first, second, rest @ ..] => format!("{first}, {second} +{}", rest.len()),
    }
}

fn set_status(ui: &AppWindow, message: impl Into<SharedString>, error: bool) {
    let bridge = ui.global::<KnownHostsBridge>();
    bridge.set_status(message.into());
    bridge.set_status_error(error);
}
