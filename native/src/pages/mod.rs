//! Rust side of the sidebar section pages; each module wires one page bridge.
use crate::AppWindow;

mod keychain;
mod forwarding;
mod snippets;
mod known_hosts;
mod logs;
mod settings;
mod recovery;

pub fn install(ui: &AppWindow) {
    keychain::install(ui);
    forwarding::install(ui);
    snippets::install(ui);
    known_hosts::install(ui);
    logs::install(ui);
    settings::install(ui);
    recovery::install(ui);
}
