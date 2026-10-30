//! Main window frame and full-screen state. `AppKit` saves the frame in the app's user defaults.
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

const FRAME: &str = "main";
const FULL_SCREEN: &str = "MainWindowFullScreen";

/// Restores the saved state, then keeps the frame saved on every move and resize.
/// `legacy` is the file of tauri-plugin-window-state, read only when `AppKit` has no frame yet.
#[cfg(target_os = "macos")]
pub fn restore(window: &WebviewWindow, legacy: &Path) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSString, NSUserDefaults};
    let Ok(pointer) = window.ns_window() else {
        return;
    };
    // Setup runs on the main thread, and the window outlives this call.
    let native = unsafe { &*pointer.cast::<NSWindow>() };
    let frame = NSString::from_str(FRAME);
    if !native.setFrameUsingName(&frame) {
        restore_legacy(window, legacy);
    }
    native.setFrameAutosaveName(&frame);
    if NSUserDefaults::standardUserDefaults().boolForKey(&NSString::from_str(FULL_SCREEN)) {
        let _ = window.set_fullscreen(true);
    }
}
#[cfg(target_os = "macos")]
pub fn save(window: &WebviewWindow) {
    use objc2_foundation::{NSString, NSUserDefaults};
    let Ok(full_screen) = window.is_fullscreen() else {
        return;
    };
    NSUserDefaults::standardUserDefaults()
        .setBool_forKey(full_screen, &NSString::from_str(FULL_SCREEN));
}
#[cfg(not(target_os = "macos"))]
pub fn restore(_: &WebviewWindow, _: &Path) {}
#[cfg(not(target_os = "macos"))]
pub fn save(_: &WebviewWindow) {}

#[derive(Deserialize)]
struct Legacy {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    prev_x: i32,
    prev_y: i32,
    maximized: bool,
    fullscreen: bool,
}
fn read_legacy(path: &Path) -> Option<Legacy> {
    let mut windows: BTreeMap<String, Legacy> =
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    windows.remove(FRAME)
}
fn restore_legacy(window: &WebviewWindow, path: &Path) {
    let Some(state) = read_legacy(path) else {
        return;
    };
    let size = PhysicalSize::new(state.width, state.height);
    let on_screen = window.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|monitor| {
            let (origin, extent) = (monitor.position(), monitor.size());
            let right = i64::from(origin.x) + i64::from(extent.width);
            let bottom = i64::from(origin.y) + i64::from(extent.height);
            i64::from(state.x) < right
                && i64::from(state.x) + i64::from(state.width) > i64::from(origin.x)
                && i64::from(state.y) < bottom
                && i64::from(state.y) + i64::from(state.height) > i64::from(origin.y)
        })
    });
    if on_screen {
        let _ = window.set_position(if state.maximized {
            PhysicalPosition::new(state.prev_x, state.prev_y)
        } else {
            PhysicalPosition::new(state.x, state.y)
        });
    }
    let _ = window.set_size(size);
    if state.maximized {
        let _ = window.maximize();
    }
    if state.fullscreen {
        let _ = window.set_fullscreen(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_state_reads_the_main_window_and_ignores_unreadable_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".window-state.json");
        assert!(read_legacy(&path).is_none());
        std::fs::write(&path, br#"{"main":{"width":2720,"height":1720,"x":2,"y":66,"prev_x":3022,"prev_y":1900,"maximized":true,"visible":true,"decorated":true,"fullscreen":false}}"#).unwrap();
        let state = read_legacy(&path).unwrap();
        assert_eq!(
            (state.width, state.height, state.x, state.y),
            (2720, 1720, 2, 66)
        );
        assert_eq!((state.prev_x, state.prev_y), (3022, 1900));
        assert!(state.maximized && !state.fullscreen);
        std::fs::write(&path, br#"{"other":{}}"#).unwrap();
        assert!(read_legacy(&path).is_none());
    }
}
