use std::ffi::c_void;

const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWCP_ROUND: u32 = 2;

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut c_void,
        attribute: u32,
        value: *const c_void,
        size: u32,
    ) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowRgn(hwnd: *mut c_void, region: *mut c_void, redraw: i32) -> i32;
}

pub fn configure(window: &tauri::WebviewWindow, glass: bool) -> tauri::Result<()> {
    // Tao's undecorated shadow reserves non-client insets outside the WebView.
    window.set_shadow(false)?;
    if native_corners(&window.as_ref().window())? {
        if glass {
            let _ = window_vibrancy::apply_blur(window, Some((238, 241, 246, 90)));
        } else {
            window.set_background_color(Some(tauri::window::Color(238, 241, 246, 255)))?;
        }
    } else {
        // Blur paints square corners. Let the WebView composite its rounded
        // contour directly onto the desktop through a transparent native window.
        window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))?;
    }
    update_shape(&window.as_ref().window())
}

fn native_corners(window: &tauri::Window) -> tauri::Result<bool> {
    let hwnd = window.hwnd()?;
    // SAFETY: hwnd belongs to this live window; the u32 stays valid for the
    // synchronous call. Windows 10 rejects the optional DWM preference.
    Ok(unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        ) >= 0
    })
}

pub fn update_shape(window: &tauri::Window) -> tauri::Result<()> {
    use tauri::Manager;

    let native_corners = native_corners(window)?;
    if !native_corners {
        // SAFETY: hwnd belongs to this live window. Clear any old GDI region;
        // regions cannot anti-alias the contour drawn by the WebView.
        unsafe {
            SetWindowRgn(window.hwnd()?.0, std::ptr::null_mut(), 1);
        }
    }
    let shape = window_shape(
        native_corners,
        window.is_maximized()?,
        window.is_fullscreen()?,
    );
    if let Some(webview) = window.get_webview_window(window.label()) {
        webview.eval(match shape {
            "round" => "document.documentElement.dataset.windowShape = 'round';",
            _ => "document.documentElement.dataset.windowShape = 'square';",
        })?;
    }
    Ok(())
}

fn window_shape(native_corners: bool, maximized: bool, fullscreen: bool) -> &'static str {
    if native_corners || maximized || fullscreen {
        "square"
    } else {
        "round"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_rounds_only_restored_windows_without_native_corners() {
        for (native_corners, maximized, fullscreen, expected) in [
            (false, false, false, "round"),
            (false, true, false, "square"),
            (false, false, true, "square"),
            (false, true, true, "square"),
            (true, false, false, "square"),
            (true, true, false, "square"),
            (true, false, true, "square"),
            (true, true, true, "square"),
        ] {
            assert_eq!(
                window_shape(native_corners, maximized, fullscreen),
                expected,
                "native_corners={native_corners}, maximized={maximized}, fullscreen={fullscreen}"
            );
        }
    }
}
