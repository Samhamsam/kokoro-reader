mod app;
mod kokoro_engine;
mod pdf;
mod tts;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 750.0])
            .with_title("Kokoro Reader"),
        ..Default::default()
    };

    eframe::run_native(
        "Kokoro Reader",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
