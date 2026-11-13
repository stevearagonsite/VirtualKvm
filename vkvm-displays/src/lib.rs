//! Display (monitor) enumeration for VirtualKvm.
//!
//! Coordinates are in the global desktop space of the process that calls [`list`]:
//! - macOS: CoreGraphics global space (points, origin at the top-left of the main display, y down).
//! - Windows: virtual-screen coordinates as seen by the calling process (physical pixels when the
//!   process is per-monitor DPI aware).
//!
//! These are the same coordinates the input capture / emulation backends use, so a point taken
//! from a [`Display`] can be fed directly to a cursor warp on the same machine.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    /// identifier that is stable while the display stays connected
    /// (macOS: CGDirectDisplayID, Windows: GDI device name such as `\\.\DISPLAY1`)
    pub id: String,
    /// human readable name (e.g. `LF27T35`, `Built-in Retina Display`)
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// device pixels per logical unit (2.0 on a Retina display, 1.25 for 125 % scaling)
    pub scale: f64,
    pub primary: bool,
}

impl Display {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && x < (self.x + self.width as i32) as f64
            && y >= self.y as f64
            && y < (self.y + self.height as i32) as f64
    }

    /// whether `query` matches this display by id or (case insensitive) name
    pub fn matches(&self, query: &str) -> bool {
        self.id == query || self.name.eq_ignore_ascii_case(query)
    }
}

/// list the currently active displays
pub fn list() -> Vec<Display> {
    imp::list()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Display;
    use core_graphics::display::CGDisplay;
    use std::collections::HashMap;

    pub fn list() -> Vec<Display> {
        let names = screen_names();
        let main = CGDisplay::main().id;
        CGDisplay::active_displays()
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                let d = CGDisplay::new(id);
                let b = d.bounds();
                let scale = d
                    .display_mode()
                    .map(|m| m.pixel_width() as f64 / m.width().max(1) as f64)
                    .unwrap_or(1.0);
                let name = names.get(&id).cloned().unwrap_or_else(|| {
                    if d.is_builtin() {
                        "Built-in Display".to_owned()
                    } else {
                        format!("Display {id}")
                    }
                });
                Display {
                    id: id.to_string(),
                    name,
                    x: b.origin.x as i32,
                    y: b.origin.y as i32,
                    width: b.size.width as u32,
                    height: b.size.height as u32,
                    scale,
                    primary: id == main,
                }
            })
            .collect()
    }

    /// display names come from AppKit, which must be used from the main thread.
    /// Off the main thread we fall back to generic names.
    fn screen_names() -> HashMap<u32, String> {
        use objc2_app_kit::NSScreen;
        use objc2_foundation::{MainThreadMarker, NSNumber, NSString};

        let mut names = HashMap::new();
        let Some(mtm) = MainThreadMarker::new() else {
            return names;
        };
        let key = NSString::from_str("NSScreenNumber");
        for screen in NSScreen::screens(mtm).iter() {
            let desc = screen.deviceDescription();
            let Some(number) = desc.objectForKey(&key) else {
                continue;
            };
            let Ok(number) = number.downcast::<NSNumber>() else {
                continue;
            };
            let name = screen.localizedName().to_string();
            names.insert(number.unsignedIntValue(), name);
        }
        names
    }
}

#[cfg(windows)]
mod imp {
    use super::Display;
    use std::mem::size_of;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::core::BOOL;

    const MONITORINFOF_PRIMARY: u32 = 1;

    unsafe extern "system" fn monitor_enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let displays = unsafe { &mut *(data.0 as *mut Vec<Display>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if !unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
            return true.into();
        }
        let r = info.monitorInfo.rcMonitor;
        let len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let device = String::from_utf16_lossy(&info.szDevice[..len]);
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let scale = match unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        {
            Ok(()) => dpi_x as f64 / 96.0,
            Err(_) => 1.0,
        };
        displays.push(Display {
            id: device.clone(),
            name: device,
            x: r.left,
            y: r.top,
            width: (r.right - r.left) as u32,
            height: (r.bottom - r.top) as u32,
            scale,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
        true.into()
    }

    pub fn list() -> Vec<Display> {
        let mut displays: Vec<Display> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(monitor_enum_proc),
                LPARAM(&mut displays as *mut Vec<Display> as isize),
            );
        }
        displays
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::Display;

    /// not implemented yet on this platform (X11 RandR / wl_output, see PLAN.md §6)
    pub fn list() -> Vec<Display> {
        Vec::new()
    }
}
