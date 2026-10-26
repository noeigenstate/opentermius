use std::ffi::c_void;
use std::sync::Mutex;

const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWCP_ROUND: u32 = 2;
const WINDOWS_APP_ICON_RESOURCE_ID: usize = 32512;
const IMAGE_ICON: u32 = 1;
const LR_DEFAULTCOLOR: u32 = 0;
const WM_SETICON: u32 = 0x0080;
const ICON_SMALL: usize = 0;
const ICON_BIG: usize = 1;
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;

// Only the main window uses these handles; Tauri's original icon stays Tao-owned.
static WINDOW_ICONS: Mutex<[usize; 2]> = Mutex::new([0; 2]);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

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
    fn GetDpiForWindow(hwnd: *mut c_void) -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn LoadImageW(
        instance: *mut c_void,
        name: *const u16,
        image_type: u32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> *mut c_void;
    fn SendMessageW(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
    fn DestroyIcon(icon: *mut c_void) -> i32;
}

pub fn configure(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    // Tao's undecorated shadow reserves non-client insets outside the WebView.
    window.set_shadow(false)?;
    update_icons(&window.as_ref().window())?;
    if native_corners(&window.as_ref().window())? {
        window.set_background_color(Some(tauri::window::Color(238, 241, 246, 255)))?;
    } else {
        // Blur paints square corners. Let the WebView composite its rounded
        // contour directly onto the desktop through a transparent native window.
        window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)))?;
    }
    update_shape(&window.as_ref().window())
}

pub fn update_icons(window: &tauri::Window) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    let mut icons = WINDOW_ICONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Tauri sets only a fixed 32px ICON_SMALL. Supply both DPI-sized icons so
    // the taskbar does not fall back to that undersized small icon.
    // SAFETY: hwnd belongs to this live main window; the integer resource is
    // embedded in this executable. Without LR_SHARED, loaded icons are ours.
    unsafe {
        let dpi = GetDpiForWindow(hwnd.0);
        let instance = GetModuleHandleW(std::ptr::null());
        if dpi == 0 || instance.is_null() {
            return Ok(());
        }
        for (kind, width_metric, height_metric) in [
            (ICON_BIG, SM_CXICON, SM_CYICON),
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
        ] {
            let width = GetSystemMetricsForDpi(width_metric, dpi);
            let height = GetSystemMetricsForDpi(height_metric, dpi);
            if width <= 0 || height <= 0 {
                continue;
            }
            let icon = LoadImageW(
                instance,
                WINDOWS_APP_ICON_RESOURCE_ID as *const u16,
                IMAGE_ICON,
                width,
                height,
                LR_DEFAULTCOLOR,
            );
            if icon.is_null() {
                continue;
            }
            SendMessageW(hwnd.0, WM_SETICON, kind, icon as isize);
            let previous_owned = std::mem::replace(&mut icons[kind], icon as usize);
            // WM_SETICON can return Tao's original handle. Free only the one
            // we loaded, now detached even if another caller replaced it first.
            if previous_owned != 0 {
                DestroyIcon(previous_owned as *mut c_void);
            }
        }
    }
    Ok(())
}

pub fn release_icons() {
    let mut icons = WINDOW_ICONS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for icon in std::mem::take(&mut *icons) {
        if icon != 0 {
            // SAFETY: the main window has been destroyed; these non-shared
            // handles were loaded by us and are no longer attached to it.
            unsafe { DestroyIcon(icon as *mut c_void) };
        }
    }
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
