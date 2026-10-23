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

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateRoundRectRgn(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: i32,
        height: i32,
    ) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
    #[cfg(test)]
    fn PtInRegion(region: *mut c_void, x: i32, y: i32) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowRgn(hwnd: *mut c_void, region: *mut c_void, redraw: i32) -> i32;
}

pub fn configure(window: &tauri::Window) -> tauri::Result<()> {
    // Tao's undecorated shadow reserves non-client insets. Blur paints those
    // insets as a second solid frame outside the WebView, so omit that shadow.
    window.set_shadow(false)?;
    update_shape(window)
}

pub fn update_shape(window: &tauri::Window) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    // SAFETY: hwnd belongs to this live window; the u32 stays valid for the
    // synchronous call. Windows 10 rejects the optional DWM preference.
    let native_corners = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        ) >= 0
    };
    if native_corners {
        return Ok(());
    }
    let size = window.outer_size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let square = window.is_maximized()? || window.is_fullscreen()?;
    let diameter = (28.0 * window.scale_factor()?).round() as i32;
    // SAFETY: dimensions come from this live native window. On success Windows
    // owns the region; on failure it remains ours and must be deleted.
    unsafe {
        let region = if square {
            std::ptr::null_mut()
        } else {
            rounded_region(size.width as i32, size.height as i32, diameter)
        };
        if (square || !region.is_null())
            && SetWindowRgn(hwnd.0, region, 1) == 0
            && !region.is_null()
        {
            DeleteObject(region);
        }
    }
    Ok(())
}

/// GDI excludes the right and bottom edges of a rectangle region, so extend
/// it by one pixel to keep the last column and row of the window visible.
unsafe fn rounded_region(width: i32, height: i32, diameter: i32) -> *mut c_void {
    // SAFETY: the caller owns or transfers the returned region.
    unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rounding_keeps_full_edges_at_standard_and_high_dpi() {
        for (width, height, diameter) in [(1240, 800, 28), (1860, 1200, 42)] {
            // SAFETY: this disposable GDI region is never transferred to a window.
            unsafe {
                let region = rounded_region(width, height, diameter);
                assert!(!region.is_null());
                assert_eq!(PtInRegion(region, 0, 0), 0);
                assert_eq!(PtInRegion(region, width - 1, height - 1), 0);
                assert_ne!(PtInRegion(region, width / 2, 0), 0);
                assert_ne!(PtInRegion(region, width - 1, height / 2), 0);
                assert_ne!(PtInRegion(region, width / 2, height - 1), 0);
                assert_eq!(PtInRegion(region, width, height / 2), 0);
                assert_ne!(DeleteObject(region), 0);
            }
        }
    }
}
