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
            .set_title("导入 OpenSSH known_hosts")
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
            Some(Err(error)) => set_status(&ui, format!("导入失败：{error:#}"), true),
            None => set_status(&ui, "配置尚未就绪，无法导入", true),
        }
    });
    let weak = ui.as_weak();
    bridge.on_remove_host(move |host, port| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Ok(port) = u16::try_from(port) else {
            set_status(&ui, "端口无效，无法移除该指纹", true);
            return;
        };
        let result = with_state(|state| {
            state
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("配置锁不可用"))
                .and_then(|mut store| store.forget(host.as_str(), port))
        });
        match result {
            Some(Ok(true)) => {
                refresh(&ui);
                set_status(
                    &ui,
                    format!("已移除 {host}:{port} 的主机指纹，下次连接将重新确认"),
                    false,
                );
            }
            Some(Ok(false)) => {
                refresh(&ui);
                set_status(&ui, format!("{host}:{port} 的主机指纹已不存在"), false);
            }
            Some(Err(error)) => set_status(&ui, error.to_string(), true),
            None => set_status(&ui, "配置尚未就绪，无法移除", true),
        }
    });
    refresh(ui);
}

fn import_file(
    state: &mut State,
    path: &std::path::Path,
) -> anyhow::Result<(store::ParsedKnownHosts, store::KnownHostsMerge)> {
    let metadata = std::fs::metadata(path).context("无法读取导入文件")?;
    anyhow::ensure!(metadata.len() <= 5 * 1024 * 1024, "导入文件不能超过 5 MB");
    let bytes = std::fs::read(path).context("无法读取导入文件")?;
    let parsed = store::parse_known_hosts(&String::from_utf8_lossy(&bytes));
    let outcome = state
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("配置锁不可用"))?
        .import_known_hosts(&parsed.entries)?;
    Ok((parsed, outcome))
}

fn import_summary(parsed: &store::ParsedKnownHosts, outcome: store::KnownHostsMerge) -> String {
    if parsed.entries.is_empty() {
        return if parsed.skipped > 0 {
            format!("未导入主机指纹，跳过 {} 条无效或不支持的记录", parsed.skipped)
        } else {
            "文件不包含 known_hosts 记录".to_string()
        };
    }
    let mut message = format!("已导入 {} 条主机指纹", outcome.added);
    if outcome.unchanged > 0 {
        message.push_str(&format!("，{} 条已存在", outcome.unchanged));
    }
    if outcome.conflicts > 0 {
        message.push_str(&format!("，{} 条与已有指纹冲突未覆盖", outcome.conflicts));
    }
    if parsed.skipped > 0 {
        message.push_str(&format!("，跳过 {} 条", parsed.skipped));
    }
    message
}

fn refresh(ui: &AppWindow) {
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
        [] => "Not in saved hosts".into(),
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
