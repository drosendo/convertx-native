//! ConvertX library root. The desktop binary and the Android shared library are
//! both thin shells around this crate.

pub mod app;
pub mod cli;
pub mod convert;
pub mod dialogs;
pub mod history;
pub mod native;
pub mod optimize;
pub mod registry;
pub mod settings;
pub mod theme;
pub mod tools;
pub mod vcf;

use std::process::Command;

pub fn open_in_file_manager(path: &std::path::Path) {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
    } else if cfg!(target_os = "android") {
        return;
    } else {
        Command::new("xdg-open")
    };
    let _ = cmd.arg(path).spawn();
}

/// Android entry point. `android-activity` calls this once the native activity
/// is ready; there is no `main()` on Android.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // Android gives each app a private data folder; that is the only place a
    // conversion may be written without asking for storage permissions.
    if let Some(base) = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .map(|path| path.to_path_buf())
    {
        settings::set_base_dir(base);
    }

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "ConvertX",
        options,
        Box::new(|cc| Ok(Box::new(app::ConvertXApp::new(cc)))),
    );
}
