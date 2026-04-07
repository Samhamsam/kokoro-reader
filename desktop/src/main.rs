mod app;
mod library;
mod pdf;
mod tts;

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

fn main() -> eframe::Result {
    let pdf_path = std::env::args().nth(1);

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1000.0, 750.0])
        .with_title("Kokoro Reader");

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Kokoro Reader",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, pdf_path)))),
    )
}
