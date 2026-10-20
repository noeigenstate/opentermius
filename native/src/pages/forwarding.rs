use crate::{AppWindow, ForwardDraft, ForwardRow, ForwardingBridge, State, model, with_state};
use anyhow::{Context, Result, ensure};
use opentermius::{
    forwarding::{Forward, Kind, Phase, RunningRule},
    store,
};
use slint::ComponentHandle;
use std::time::Duration;
use zeroize::Zeroizing;

fn busy(s: &State, id: &str) -> bool {
    s.forwards
        .get(id)
        .is_some_and(|r| matches!(r.status().phase, Phase::Starting | Phase::Running))
}
pub(super) fn refresh(s: &State) {
    let Some(ui) = s.ui.upgrade() else { return };
    let Ok(store) = s.store.lock() else { return };
    let bridge = ui.global::<ForwardingBridge>();
    bridge.set_hosts(model(
        store
            .data
            .hosts
            .iter()
            .map(|h| format!("{} · {}", h.name, h.host).into())
            .collect(),
    ));
    bridge.set_rules(model(
        store
            .data
            .forwards
            .iter()
            .map(|f| {
                let status = s.forwards.get(&f.id).map(RunningRule::status);
                let error = s.forward_errors.get(&f.id);
                let host = store
                    .data
                    .hosts
                    .iter()
                    .find(|h| h.id == f.host_id)
                    .map(|h| h.name.as_str())
                    .unwrap_or(crate::i18n::t("Host deleted"));
                let route = if f.kind == Kind::Dynamic {
                    format!("SOCKS5 · [{}]:{} → {host}", f.bind_address, f.bind_port)
                } else {
                    format!(
                        "{} · [{}]:{} → {}:{} · {host}",
                        match f.kind { Kind::Local => crate::i18n::t("Local"), Kind::Remote => crate::i18n::t("Remote"), Kind::Dynamic => crate::i18n::t("Dynamic") }, f.bind_address, f.bind_port, f.destination_host, f.destination_port
                    )
                };
                ForwardRow {
                    id: f.id.clone().into(),
                    label: f.label.clone().into(),
                    route: route.into(),
                    status: crate::i18n::translate(&error
                        .cloned()
                        .or_else(|| status.as_ref().map(|s| s.message.clone()))
                        .unwrap_or_else(|| crate::i18n::t("Stopped").into()))
                        .into(),
                    busy: busy(s, &f.id),
                    error: error.is_some()
                        || status.as_ref().is_some_and(|s| s.phase == Phase::Error),
                    connections: status.map_or(0, |s| s.active_connections as i32),
                }
            })
            .collect(),
    ));
}
fn start(s: &mut State, id: &str) -> Result<()> {
    ensure!(!busy(s, id), "{}", crate::i18n::t("Rule is running"));
    ensure!(s.forwards
            .values()
            .filter(|rule| matches!(rule.status().phase, Phase::Starting | Phase::Running))
            .count()
            < opentermius::forwarding::MAX_RULES, "{}", crate::i18n::t("Too many forwarding rules are running"));
    let (rule, host, secret, key_material, known) = {
        let store = s.store.lock().map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration is unavailable")))?;
        let rule = store
            .data
            .forwards
            .iter()
            .find(|f| f.id == id)
            .context(crate::i18n::t("Rule not found"))?
            .clone();
        let host = store
            .data
            .hosts
            .iter()
            .find(|h| h.id == rule.host_id)
            .context(crate::i18n::t("SSH host was deleted; select another host"))?
            .clone();
        let known = store
            .data
            .known_hosts
            .get(&store::key(&host.host, host.port))
            .context(crate::i18n::t("Connect to this host once and trust its fingerprint first"))?
            .clone();
        let key_material = if host.auth_type == "key" && !host.key_id.is_empty() {
            Some(store.key_material(&host.key_id)?)
        } else {
            None
        };
        let secret = match &host.encrypted_secret {
            _ if key_material.is_some() => Zeroizing::new(String::new()),
            Some(encrypted) => Zeroizing::new(
                store::unprotect(encrypted)
                    .context(crate::i18n::t("Cannot decrypt credentials; connect to this host and save them again"))?,
            ),
            None if host.auth_type == "key" => Zeroizing::new(String::new()),
            None => anyhow::bail!("{}", crate::i18n::t("Password is not saved; connect to this host once and save the password")),
        };
        (rule, host, secret, key_material, known)
    };
    let running = RunningRule::start(s.runtime.handle(), rule, host, secret, key_material, known)?;
    s.forwards.insert(id.into(), running);
    s.forward_errors.remove(id);
    Ok(())
}
fn draft(f: &Forward, hosts: &[store::Profile]) -> ForwardDraft {
    ForwardDraft {
        id: f.id.clone().into(),
        label: f.label.clone().into(),
        kind_index: match f.kind {
            Kind::Local => 0,
            Kind::Remote => 1,
            Kind::Dynamic => 2,
        },
        host_index: hosts
            .iter()
            .position(|h| h.id == f.host_id)
            .map_or(-1, |i| i as i32),
        bind_address: f.bind_address.clone().into(),
        bind_port: if f.bind_port == 0 {
            String::new()
        } else {
            f.bind_port.to_string()
        }
        .into(),
        destination_host: f.destination_host.clone().into(),
        destination_port: if f.destination_port == 0 {
            String::new()
        } else {
            f.destination_port.to_string()
        }
        .into(),
        auto_start: f.auto_start,
    }
}
fn save(s: &mut State, d: ForwardDraft) -> Result<()> {
    ensure!(!busy(s, d.id.as_str()), "{}", crate::i18n::t("Stop the rule before editing it"));
    let mut store = s.store.lock().map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration is unavailable")))?;
    let host = store
        .data
        .hosts
        .get(usize::try_from(d.host_index).unwrap_or(usize::MAX))
        .context(crate::i18n::t("Select a saved SSH host"))?;
    let kind = match d.kind_index {
        0 => Kind::Local,
        1 => Kind::Remote,
        2 => Kind::Dynamic,
        _ => anyhow::bail!("{}", crate::i18n::t("Invalid forwarding type")),
    };
    let mut rule = Forward {
        id: d.id.to_string(),
        label: d.label.to_string(),
        kind,
        host_id: host.id.clone(),
        bind_address: d.bind_address.to_string(),
        bind_port: d
            .bind_port
            .trim()
            .parse()
            .context(crate::i18n::t("Bind port must be between 1 and 65535"))?,
        destination_host: d.destination_host.to_string(),
        destination_port: if kind == Kind::Dynamic {
            0
        } else {
            d.destination_port
                .trim()
                .parse()
                .context(crate::i18n::t("Destination port must be between 1 and 65535"))?
        },
        auto_start: d.auto_start,
    };
    rule.validate(&store.data.forwards)?;
    let old = store.data.forwards.clone();
    let id = rule.id.clone();
    if let Some(previous) = store.data.forwards.iter_mut().find(|f| f.id == id) {
        *previous = rule;
    } else {
        store.data.forwards.push(rule);
    }
    if let Err(e) = store.persist() {
        store.data.forwards = old;
        return Err(e);
    }
    s.forwards.remove(&id);
    s.forward_errors.remove(&id);
    Ok(())
}
pub fn install(ui: &AppWindow) {
    ui.global::<ForwardingBridge>().on_new_rule(|| {
        with_state(|s| {
            if let (Some(ui), Ok(store)) = (s.ui.upgrade(), s.store.lock()) {
                let mut d = draft(&Forward::default(), &store.data.hosts);
                d.host_index = if store.data.hosts.is_empty() { -1 } else { 0 };
                let bridge = ui.global::<ForwardingBridge>();
                bridge.set_draft(d);
                bridge.set_message("".into());
                bridge.set_editor_open(true);
            }
        });
    });
    ui.global::<ForwardingBridge>().on_edit_rule(|id| {
        with_state(|s| {
            if busy(s, &id) {
                return;
            }
            if let (Some(ui), Ok(store)) = (s.ui.upgrade(), s.store.lock()) {
                if let Some(f) = store.data.forwards.iter().find(|f| f.id == id.as_str()) {
                    let bridge = ui.global::<ForwardingBridge>();
                    bridge.set_draft(draft(f, &store.data.hosts));
                    bridge.set_message("".into());
                    bridge.set_editor_open(true);
                }
            }
        });
    });
    ui.global::<ForwardingBridge>().on_save_rule(|| {
        with_state(|s| {
            if let Some(ui) = s.ui.upgrade() {
                let bridge = ui.global::<ForwardingBridge>();
                match save(s, bridge.get_draft()) {
                    Ok(()) => {
                        bridge.set_editor_open(false);
                        bridge.set_message("".into());
                    }
                    Err(e) => bridge.set_message(format!("{e:#}").into()),
                }
                refresh(s);
            }
        });
    });
    ui.global::<ForwardingBridge>().on_toggle_rule(|id| {
        with_state(|s| {
            if busy(s, &id) {
                if let Some(r) = s.forwards.get(id.as_str()) {
                    r.request_stop();
                }
            } else if let Err(e) = start(s, &id) {
                s.forward_errors.insert(id.to_string(), format!("{e:#}"));
            }
            refresh(s);
        });
    });
    ui.global::<ForwardingBridge>().on_delete_rule(|id| {
        with_state(|s| {
            let result = (|| -> Result<()> {
                ensure!(!busy(s, &id), "{}", crate::i18n::t("Stop the rule before deleting it"));
                let mut store = s.store.lock().map_err(|_| anyhow::anyhow!("{}", crate::i18n::t("Configuration is unavailable")))?;
                let old = store.data.forwards.clone();
                store.data.forwards.retain(|f| f.id != id.as_str());
                if let Err(e) = store.persist() {
                    store.data.forwards = old;
                    return Err(e);
                }
                s.forwards.remove(id.as_str());
                s.forward_errors.remove(id.as_str());
                Ok(())
            })();
            if let Some(ui) = s.ui.upgrade() {
                ui.global::<ForwardingBridge>().set_message(
                    result
                        .err()
                        .map(|e| format!("{e:#}"))
                        .unwrap_or_default()
                        .into(),
                );
            }
            refresh(s);
        });
    });
    with_state(|s| {
        let auto = s
            .store
            .lock()
            .map(|store| {
                store
                    .data
                    .forwards
                    .iter()
                    .filter(|f| f.auto_start)
                    .map(|f| f.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in auto {
            if let Err(e) = start(s, &id) {
                s.forward_errors.insert(id, format!("{e:#}"));
            }
        }
        refresh(s);
        s.forwarding_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            || {
                with_state(|s| refresh(s));
            },
        );
    });
}
