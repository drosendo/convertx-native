use std::process::Command;

mod app;
mod cli;
mod convert;
mod history;
mod native;
mod optimize;
mod registry;
mod settings;
mod theme;
mod tools;
mod vcf;

fn main() -> eframe::Result {
    if let Some(code) = cli::maybe_run() {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([920.0, 600.0])
            .with_title("ConvertX"),
        ..Default::default()
    };

    eframe::run_native(
        "ConvertX",
        options,
        Box::new(|cc| Ok(Box::new(app::ConvertXApp::new(cc)))),
    )
}

pub fn open_in_file_manager(path: &std::path::Path) {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
    } else {
        Command::new("xdg-open")
    };
    let _ = cmd.arg(path).spawn();
}
