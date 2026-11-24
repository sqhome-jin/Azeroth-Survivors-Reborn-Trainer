use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

unsafe extern "C" {
    fn trainer_install_macos_chrome(view: *mut std::ffi::c_void) -> i32;
    fn trainer_macos_preferences() -> i32;
}

pub fn install(frame: &eframe::Frame) -> bool {
    let Ok(handle) = frame.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // eframe calls update on the AppKit main thread with a live NSView.
    unsafe { trainer_install_macos_chrome(handle.ns_view.as_ptr()) != 0 }
}

pub fn preferences() -> (bool, egui::Theme) {
    // The native preference and appearance objects are accessed on the UI thread.
    let settings = unsafe { trainer_macos_preferences() };
    let theme = if settings & 2 != 0 {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    (settings & 1 != 0, theme)
}

pub fn install_chinese_font(context: &egui::Context) -> bool {
    let Ok(bytes) = std::fs::read("/System/Library/Fonts/STHeiti Light.ttc") else {
        return false;
    };
    let mut fonts = egui::FontDefinitions::default();
    let family = "macos-chinese".to_owned();
    fonts
        .font_data
        .insert(family.clone(), egui::FontData::from_owned(bytes).into());
    for fallback in fonts.families.values_mut() {
        fallback.push(family.clone());
    }
    context.set_fonts(fonts);
    true
}
