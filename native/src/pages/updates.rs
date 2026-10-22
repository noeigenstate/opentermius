//! Settings → About update controls and the optional check shortly after startup.
//! Network work runs on a background thread; installing always asks first because it
//! restarts OpenTermius and closes open sessions.
use crate::{AppWindow, SettingsBridge};
use opentermius::update::{self, Release};
use slint::ComponentHandle;
use std::{cell::RefCell, time::Duration};

const CURRENT: &str = env!("CARGO_PKG_VERSION");

thread_local! { static AVAILABLE: RefCell<Option<Release>> = const { RefCell::new(None) }; }

pub fn install(ui: &AppWindow) {
    let bridge = ui.global::<SettingsBridge>();
    let weak = ui.as_weak();
    bridge.on_check_updates(move || check(weak.clone(), false));
    let weak = ui.as_weak();
    bridge.on_install_update(move || install_update(weak.clone()));

    // Leftovers of a finished update (installer and helper copy); the helper may still be
    // exiting right after it started this process, in which case the next start cleans up.
    let _ = std::fs::remove_dir_all(update_dir());

    let scripted = std::env::args().any(|a| a == "--smoke-test" || a == "--benchmark");
    let enabled = crate::with_state(|state| state.settings.auto_update).unwrap_or(false);
    if enabled && !scripted && std::env::var_os("OPENTERMIUS_NO_UPDATE_CHECK").is_none() {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_secs(5), move || check(weak, true));
    }
}

fn update_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("opentermius-update")
}

fn check(weak: slint::Weak<AppWindow>, automatic: bool) {
    let Some(ui) = weak.upgrade() else { return };
    let bridge = ui.global::<SettingsBridge>();
    if bridge.get_update_busy() {
        return;
    }
    bridge.set_update_busy(true);
    bridge.set_update_status(crate::i18n::t("Checking for updates…").into());
    std::thread::spawn(move || {
        let result = update::check(CURRENT);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let bridge = ui.global::<SettingsBridge>();
            bridge.set_update_busy(false);
            match result {
                Ok(Some(release)) => {
                    bridge.set_update_status(opentermius::tf!("Version {} is available", "0" => release.version.clone()).into());
                    bridge.set_update_version(release.version.clone().into());
                    AVAILABLE.with(|available| *available.borrow_mut() = Some(release.clone()));
                    if automatic {
                        let install = rfd::MessageDialog::new()
                            .set_title(crate::i18n::t("Update available"))
                            .set_description(opentermius::tf!("OpenTermius {} is available (you have {}). Install it now? OpenTermius restarts and open sessions close.", "0" => release.version, "1" => CURRENT.to_string()))
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show()
                            == rfd::MessageDialogResult::Yes;
                        if install {
                            install_update(ui.as_weak());
                        }
                    }
                }
                Ok(None) => {
                    bridge.set_update_version("".into());
                    bridge.set_update_status(opentermius::tf!("OpenTermius is up to date ({})", "0" => CURRENT.to_string()).into());
                }
                Err(error) => {
                    bridge.set_update_status(opentermius::tf!("Update check failed: {error}", "error" => format!("{error:#}")).into());
                }
            }
        });
    });
}

fn install_update(weak: slint::Weak<AppWindow>) {
    let Some(release) = AVAILABLE.with(|available| available.borrow().clone()) else { return };
    let Some(ui) = weak.upgrade() else { return };
    let bridge = ui.global::<SettingsBridge>();
    let Some(install_dir) = update::install_dir() else {
        // Not an installed copy (a build or portable folder): download it by hand.
        open_url(&release.page_url);
        return;
    };
    if bridge.get_update_busy() {
        return;
    }
    bridge.set_update_busy(true);
    bridge.set_update_status(opentermius::tf!("Downloading OpenTermius {}…", "0" => release.version.clone()).into());
    std::thread::spawn(move || {
        let work = update_dir();
        let result = update::download(&release, &work);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let bridge = ui.global::<SettingsBridge>();
            match result.and_then(|installer| launch(&installer, &install_dir, &work)) {
                Ok(()) => {
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    bridge.set_update_busy(false);
                    bridge.set_update_status(opentermius::tf!("Update failed: {error}", "error" => format!("{error:#}")).into());
                }
            }
        });
    });
}

#[cfg(windows)]
fn launch(installer: &std::path::Path, install_dir: &std::path::Path, work: &std::path::Path) -> anyhow::Result<()> {
    update::launch_installer(installer, install_dir, work)
}

#[cfg(not(windows))]
fn launch(_: &std::path::Path, _: &std::path::Path, _: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("{}", crate::i18n::t("Automatic installation is only available on Windows"))
}

fn open_url(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
