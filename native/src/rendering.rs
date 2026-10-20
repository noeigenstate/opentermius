//! Renderer policy and native window polish. No network access.
use anyhow::{Context, bail};
use slint::winit_030::{CustomApplicationHandler, EventResult, winit};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    Software,
    Femtovg,
    Skia,
}

impl Renderer {
    pub fn name(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Femtovg => "femtovg",
            Self::Skia => "skia",
        }
    }

    pub fn backend(self) -> String {
        format!("winit-{}", self.name())
    }

    pub fn slint_name(self) -> &'static str {
        if self == Self::Skia {
            "skia-opengl"
        } else {
            self.name()
        }
    }

    pub fn available(self) -> bool {
        match self {
            Self::Software => true,
            Self::Femtovg => cfg!(feature = "renderer-femtovg"),
            Self::Skia => cfg!(feature = "renderer-skia"),
        }
    }

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "software" => Ok(Self::Software),
            "femtovg" => Ok(Self::Femtovg),
            "skia" => Ok(Self::Skia),
            _ => bail!("无效的渲染器 {value:?}；请使用 software、femtovg 或 skia"),
        }
    }
}

pub fn requested(
    args: &[String],
    environment: Option<&str>,
    gpu_rendering: bool,
) -> anyhow::Result<Renderer> {
    let mut cli = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--renderer" {
            cli = Some(args.next().context("--renderer 缺少参数")?.as_str());
        } else if let Some(value) = arg.strip_prefix("--renderer=") {
            cli = Some(value);
        }
    }
    match cli.or(environment) {
        Some(value) => Renderer::parse(value),
        None if gpu_rendering => Ok(Renderer::Femtovg),
        None => Ok(Renderer::Software),
    }
}

// Slint's platform is process-global. GL creation happens on event-loop resume,
// after BackendSelector::select succeeds, so retry in a fresh process instead of
// trying to replace an already installed platform. Only platform errors before
// the first frame qualify; file/configuration/SSH failures never trigger retry.
#[derive(Debug)]
pub struct PlatformFailure;
impl std::fmt::Display for PlatformFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("无法初始化窗口或渲染上下文")
    }
}
impl std::error::Error for PlatformFailure {}

pub fn should_retry(renderer: Renderer, first_frame: Option<f64>, error: &anyhow::Error) -> bool {
    renderer != Renderer::Software
        && first_frame.is_none()
        && error.downcast_ref::<PlatformFailure>().is_some()
}

pub fn retry_software() -> anyhow::Result<i32> {
    let status = std::process::Command::new(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        // Last CLI occurrence wins, including over OPENTERMIUS_RENDERER.
        .arg("--renderer=software")
        .env(
            "OPENTERMIUS_RENDERER_LAUNCHER_PID",
            std::process::id().to_string(),
        )
        .status()
        .context("无法启动软件渲染回退进程")?;
    Ok(status.code().unwrap_or(1))
}

pub fn launcher_pid() -> u32 {
    std::env::var("OPENTERMIUS_RENDERER_LAUNCHER_PID")
        .ok()
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(std::process::id)
}

pub fn window_attributes(
    attributes: winit::window::WindowAttributes,
) -> winit::window::WindowAttributes {
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_undecorated_shadow(true)
    }
    #[cfg(not(windows))]
    {
        attributes
    }
}

#[cfg(any(windows, test))]
fn windows_monospace(cascadia_installed: bool) -> &'static str {
    if cascadia_installed {
        "Cascadia Mono"
    } else {
        "Consolas"
    }
}

pub fn configure_fonts() -> (String, &'static str) {
    #[cfg(windows)]
    {
        use slint::fontique_011::{fontique, shared_collection};
        let mut collection = shared_collection();
        if let Some(cjk) = collection.family_id("Microsoft YaHei UI") {
            collection.set_fallbacks(
                fontique::FallbackKey::new(fontique::Script::from_bytes(*b"Hani"), None),
                [cjk].into_iter(),
            );
        }
        let mono = windows_monospace(collection.family_id("Cascadia Mono").is_some());
        ("Segoe UI".into(), mono)
    }
    #[cfg(target_os = "macos")]
    {
        use slint::fontique_011::{fontique, shared_collection};
        let mut collection = shared_collection();
        let family = collection
            .generic_families(fontique::GenericFamily::SystemUi)
            .next();
        let family = family
            .and_then(|id| collection.family_name(id).map(str::to_owned))
            .unwrap_or_else(|| "sans-serif".into());
        (family, "Menlo")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        ("sans-serif".into(), "DejaVu Sans Mono")
    }
}

pub struct WindowPolish {
    started: Instant,
    renderer: Rc<Cell<Renderer>>,
    output: Option<PathBuf>,
    first_frame: Rc<Cell<Option<f64>>>,
    drew: bool,
    polished: bool,
}

impl WindowPolish {
    pub fn new(
        started: Instant,
        renderer: Rc<Cell<Renderer>>,
        output: Option<PathBuf>,
        first_frame: Rc<Cell<Option<f64>>>,
    ) -> Self {
        Self {
            started,
            renderer,
            output,
            first_frame,
            drew: false,
            polished: false,
        }
    }
}

impl CustomApplicationHandler for WindowPolish {
    fn window_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        winit_window: Option<&winit::window::Window>,
        _slint_window: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        if !self.polished {
            if let Some(window) = winit_window {
                polish_window(window);
                self.polished = true;
            }
        }
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            self.drew = true;
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) -> EventResult {
        // Called after Slint has handled RedrawRequested, including presenting
        // the frame. This works for software too (RenderingNotifier does not).
        if self.drew && self.first_frame.get().is_none() {
            let elapsed = self.started.elapsed().as_secs_f64() * 1000.0;
            self.first_frame.set(Some(elapsed));
            if let Some(output) = &self.output {
                let result = (|| -> anyhow::Result<()> {
                    std::fs::create_dir_all(output)?;
                    std::fs::write(
                        output.join("first-frame.json"),
                        serde_json::json!({
                        "backend": self.renderer.get().backend(), "firstFrameMs": elapsed,
                            "criterion": "first event-loop idle after RedrawRequested/present",
                            "pid": std::process::id(),
                        })
                        .to_string(),
                    )?;
                    Ok(())
                })();
                if let Err(error) = result {
                    eprintln!("无法记录首帧计时：{error}");
                }
            }
        }
        EventResult::Propagate
    }
}

fn polish_window(_window: &winit::window::Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::{Graphics::Dwm::*, UI::Controls::MARGINS};
        if let Ok(handle) = _window.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                let hwnd = handle.hwnd.get() as _;
                let corner = DWMWCP_ROUND;
                let policy = DWMNCRP_ENABLED;
                // Windows 10 returns E_INVALIDARG for the Win11 corner attribute.
                // Native compositor polish is optional; never fail startup for it.
                unsafe {
                    let _ = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_WINDOW_CORNER_PREFERENCE as _,
                        (&corner as *const i32).cast(),
                        std::mem::size_of_val(&corner) as _,
                    );
                    let _ = DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_NCRENDERING_POLICY as _,
                        (&policy as *const i32).cast(),
                        std::mem::size_of_val(&policy) as _,
                    );
                    let _ = DwmExtendFrameIntoClientArea(
                        hwnd,
                        &MARGINS {
                            cxLeftWidth: 1,
                            cxRightWidth: 1,
                            cyTopHeight: 1,
                            cyBottomHeight: 1,
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cli_overrides_environment_and_last_override_wins() {
        assert_eq!(
            requested(
                &args(&["app", "--renderer=femtovg", "--renderer", "software"]),
                Some("skia"),
                true
            )
            .unwrap(),
            Renderer::Software
        );
        assert_eq!(
            requested(&args(&["app"]), Some("skia"), true).unwrap(),
            Renderer::Skia
        );
        assert_eq!(
            requested(&args(&["app", "--renderer=software"]), Some("typo"), true).unwrap(),
            Renderer::Software
        );
    }

    #[test]
    fn invalid_or_missing_values_are_errors() {
        for values in [
            &["--renderer="][..],
            &["--renderer"],
            &["--renderer", "--smoke-test"],
            &["--renderer=unknown"],
        ] {
            assert!(requested(&args(values), None, false).is_err());
        }
        assert!(requested(&[], Some("unknown"), true).is_err());
    }

    #[test]
    fn software_default_is_independent_of_compiled_gpu_features() {
        assert_eq!(requested(&[], None, false).unwrap(), Renderer::Software);
        assert_eq!(Renderer::Software.backend(), "winit-software");
    }

    #[test]
    fn persisted_gpu_preference_is_overridden_by_environment_and_cli() {
        assert_eq!(requested(&[], None, true).unwrap(), Renderer::Femtovg);
        for gpu in [false, true] {
            for (name, renderer) in [
                ("software", Renderer::Software),
                ("femtovg", Renderer::Femtovg),
                ("skia", Renderer::Skia),
            ] {
                assert_eq!(requested(&[], Some(name), gpu).unwrap(), renderer);
                assert_eq!(
                    requested(&args(&["app", "--renderer", name]), Some("software"), gpu).unwrap(),
                    renderer
                );
            }
        }
    }

    #[test]
    fn terminal_font_uses_cascadia_only_when_installed() {
        assert_eq!(windows_monospace(true), "Cascadia Mono");
        assert_eq!(windows_monospace(false), "Consolas");
    }

    #[test]
    fn retry_is_limited_to_gpu_platform_startup_failure() {
        let platform = anyhow::anyhow!("GL context failed").context(PlatformFailure);
        assert!(should_retry(Renderer::Femtovg, None, &platform));
        assert!(should_retry(Renderer::Skia, None, &platform));
        assert!(!should_retry(Renderer::Software, None, &platform));
        assert!(!should_retry(Renderer::Femtovg, Some(1.0), &platform));
        assert!(!should_retry(
            Renderer::Femtovg,
            None,
            &anyhow::anyhow!("config failed")
        ));
    }
}
