//! Rust side of the sidebar section pages; each module wires one page bridge.
use crate::AppWindow;
use slint::ComponentHandle;

mod keychain;
mod forwarding;
mod snippets;
mod known_hosts;
pub(crate) mod logs;
mod settings;
mod recovery;
mod updates;

pub fn install(ui: &AppWindow) {
    keychain::install(ui);
    forwarding::install(ui);
    snippets::install(ui);
    known_hosts::install(ui);
    logs::install(ui);
    settings::install(ui);
    recovery::install(ui);
    updates::install(ui);
}

pub(super) fn refresh_language(ui: &AppWindow) {
    keychain::refresh(ui);
    known_hosts::refresh(ui);
    snippets::refresh(ui, &ui.global::<crate::SnippetsBridge>().get_query());
    logs::refresh();
    crate::with_state(|state| {
        for view in state.sessions.values_mut() {
            view.status = crate::i18n::translate(&view.status);
        }
        for error in state.forward_errors.values_mut() {
            *error = crate::i18n::translate(error);
        }
        for (_, row) in state.transfers.values_mut() {
            if let Some((direction, name)) = row.name.split_once(" · ") {
                row.name = format!("{} · {name}", crate::i18n::translate(direction)).into();
            }
            row.detail = crate::i18n::translate(&row.detail).into();
        }
        state.hosts();
        state.tabs();
        state.display();
        state.render_files();
        state.render_transfers();
        forwarding::refresh(state);
    });
    ui.set_status_text(crate::i18n::translate(&ui.get_status_text()).into());
    ui.set_prompt_title(crate::i18n::translate(&ui.get_prompt_title()).into());
    ui.set_prompt_message(crate::i18n::translate(&ui.get_prompt_message()).into());
    let keys = ui.global::<crate::KeychainBridge>();
    keys.set_message(crate::i18n::translate(&keys.get_message()).into());
    keys.set_passphrase_error(crate::i18n::translate(&keys.get_passphrase_error()).into());
    let known = ui.global::<crate::KnownHostsBridge>();
    known.set_status(crate::i18n::translate(&known.get_status()).into());
    let forwards = ui.global::<crate::ForwardingBridge>();
    forwards.set_message(crate::i18n::translate(&forwards.get_message()).into());
    let snippets = ui.global::<crate::SnippetsBridge>();
    snippets.set_error_text(crate::i18n::translate(&snippets.get_error_text()).into());
    let recovery = ui.global::<crate::RecoveryBridge>();
    recovery.set_message(crate::i18n::translate(&recovery.get_message()).into());
}
