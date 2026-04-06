use crate::pdf::{PageRender, PdfDoc};
use crate::tts::{TtsEngine, TtsState, VOICES};
use egui::{ColorImage, TextureHandle};

pub struct App {
    pdf: Option<PdfDoc>,
    current_page: usize,
    page_texture: Option<TextureHandle>,
    page_text: String,
    tts: TtsEngine,
    selected_voice: usize,
    speed: f32,
    status_msg: String,
    needs_render: bool,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            pdf: None,
            current_page: 0,
            page_texture: None,
            page_text: String::new(),
            tts: TtsEngine::new(),
            selected_voice: 0,
            speed: 1.0,
            status_msg: "Open a PDF to start".into(),
            needs_render: false,
        }
    }

    fn open_pdf(&mut self, path: std::path::PathBuf) {
        match PdfDoc::open(&path) {
            Ok(doc) => {
                self.status_msg = format!(
                    "Opened: {} ({} pages)",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    doc.page_count()
                );
                self.pdf = Some(doc);
                self.current_page = 0;
                self.page_texture = None;
                self.needs_render = true;
            }
            Err(e) => {
                self.status_msg = format!("Error: {}", e);
            }
        }
    }

    fn render_current_page(&mut self, ctx: &egui::Context) {
        if let Some(ref pdf) = self.pdf {
            match pdf.render_page(self.current_page, 1200) {
                Ok(PageRender {
                    rgba,
                    width,
                    height,
                    text,
                }) => {
                    let image =
                        ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    self.page_texture = Some(ctx.load_texture(
                        format!("page-{}", self.current_page),
                        image,
                        Default::default(),
                    ));
                    self.page_text = text;
                }
                Err(e) => {
                    self.status_msg = format!("Render error: {}", e);
                }
            }
        }
        self.needs_render = false;
    }

    fn go_to_page(&mut self, page: usize) {
        if let Some(ref pdf) = self.pdf {
            if page < pdf.page_count() {
                self.current_page = page;
                self.page_texture = None;
                self.needs_render = true;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.needs_render {
            self.render_current_page(ctx);
        }

        // Top panel: controls
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // File open
                if ui.button("Open PDF").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .pick_file()
                    {
                        self.open_pdf(path);
                    }
                }

                ui.separator();

                // Page navigation
                let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
                if ui.button("<").clicked() && self.current_page > 0 {
                    self.go_to_page(self.current_page - 1);
                }
                ui.label(format!("{} / {}", self.current_page + 1, page_count));
                if ui.button(">").clicked() && self.current_page + 1 < page_count {
                    self.go_to_page(self.current_page + 1);
                }
            });

            ui.horizontal(|ui| {
                // TTS controls
                let tts_state = self.tts.state();

                match tts_state {
                    TtsState::Idle | TtsState::Error(_) => {
                        let can_play = self.pdf.is_some() && !self.page_text.is_empty();
                        if ui.add_enabled(can_play, egui::Button::new("Play")).clicked() {
                            let voice = VOICES[self.selected_voice].0.to_string();
                            self.tts.speak(self.page_text.clone(), voice, self.speed);
                        }
                    }
                    TtsState::Generating => {
                        ui.spinner();
                        let (cur, total) = self.tts.progress();
                        ui.label(format!("Generating {}/{}...", cur, total));
                        ctx.request_repaint();
                    }
                    TtsState::Playing => {
                        let (cur, total) = self.tts.progress();
                        ui.label(format!("[{}/{}]", cur, total));
                        if ui.button("Pause").clicked() {
                            self.tts.pause();
                        }
                        if ui.button("Stop").clicked() {
                            self.tts.stop();
                        }
                        ctx.request_repaint();
                    }
                    TtsState::Paused => {
                        if ui.button("Resume").clicked() {
                            self.tts.resume();
                        }
                        if ui.button("Stop").clicked() {
                            self.tts.stop();
                        }
                    }
                }

                ui.separator();

                // Voice selection
                ui.label("Voice:");
                egui::ComboBox::from_id_salt("voice")
                    .selected_text(VOICES[self.selected_voice].1)
                    .show_ui(ui, |ui| {
                        for (i, (_, label)) in VOICES.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_voice, i, *label);
                        }
                    });

                // Speed slider
                ui.label("Speed:");
                let old_speed = self.speed;
                ui.add(egui::Slider::new(&mut self.speed, 0.5..=2.0).step_by(0.1));
                if (self.speed - old_speed).abs() > 0.01 {
                    self.tts.set_speed(self.speed);
                }
            });
        });

        // Bottom panel: status
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status_msg);
                if let TtsState::Error(ref e) = self.tts.state() {
                    ui.colored_label(egui::Color32::RED, format!("TTS Error: {}", e));
                }
            });
        });

        // Central panel: PDF page
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref texture) = self.page_texture {
                egui::ScrollArea::both().show(ui, |ui| {
                    let available_width = ui.available_width();
                    let tex_size = texture.size_vec2();
                    let scale = available_width / tex_size.x;
                    let display_size =
                        egui::Vec2::new(available_width, tex_size.y * scale);
                    ui.image(egui::load::SizedTexture::new(texture.id(), display_size));
                });
            } else if self.pdf.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.heading("Drag & drop a PDF or click 'Open PDF'");
                });
            }
        });

        // Handle drag & drop
        ctx.input(|i| {
            if let Some(dropped) = i.raw.dropped_files.first() {
                if let Some(path) = &dropped.path {
                    if path.extension().is_some_and(|e| e == "pdf") {
                        self.open_pdf(path.clone());
                    }
                }
            }
        });
    }
}
