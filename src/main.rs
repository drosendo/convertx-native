use convertx::{app, cli};
use eframe::egui;

fn main() -> eframe::Result {
    // `--to` / `--optimize` run without opening a window.
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
