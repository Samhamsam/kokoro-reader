use crate::pdf::{PageRender, PdfDoc};
use crate::tts::{TtsEngine, TtsState, VOICES};
use egui::{Color32, ColorImage, Rect, TextureHandle, Vec2, Pos2};

pub struct App {
    pdf: Option<PdfDoc>,
    current_page: usize,
    page_texture: Option<TextureHandle>,
    page_text: String,
    page_img_size: (usize, usize), // rendered image dimensions
    tts: TtsEngine,
    selected_voice: usize,
    speed: f32,
    status_msg: String,
    needs_render: bool,
    reading_active: bool, // true after user clicks Play, stays true for auto-advance
}

impl App {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            pdf: None,
            current_page: 0,
            page_texture: None,
            page_text: String::new(),
            page_img_size: (0, 0),
            tts: TtsEngine::new(),
            selected_voice: 0,
            speed: 1.0,
            status_msg: "Open a PDF to start".into(),
            needs_render: false,

            reading_active: false,
        }
    }

    fn open_pdf(&mut self, path: std::path::PathBuf) {
        self.tts.stop();
        self.reading_active = false;
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
                Ok(PageRender { rgba, width, height, text }) => {
                    let image = ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                    self.page_texture = Some(ctx.load_texture(
                        format!("page-{}", self.current_page),
                        image,
                        Default::default(),
                    ));
                    self.page_text = text;
                    self.page_img_size = (width, height);
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

    fn start_reading(&mut self) {
        let voice = VOICES[self.selected_voice].0.to_string();
        self.tts.speak(self.page_text.clone(), voice, self.speed);
        self.reading_active = true;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Auto-advance when page finished
        if self.tts.state() == TtsState::Finished {
            self.tts.clear_finished();
            if self.reading_active {
                let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
                if self.current_page + 1 < page_count {
                    self.go_to_page(self.current_page + 1);
                } else {
                    self.reading_active = false;
                }
            }
        }

        if self.needs_render {
            self.render_current_page(ctx);
            // Auto-start reading on new page if we were already reading
            if self.reading_active {
                if !self.page_text.trim().is_empty() {
                    self.start_reading();
                } else {
                    // Empty page — skip to next
                    let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
                    if self.current_page + 1 < page_count {
                        self.go_to_page(self.current_page + 1);
                    } else {
                        self.reading_active = false;
                    }
                }
            }
        }

        // Top panel: controls
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open PDF").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .pick_file()
                    {
                        self.open_pdf(path);
                    }
                }

                ui.separator();

                let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
                if ui.button("<").clicked() && self.current_page > 0 {
                    self.tts.stop();
                    self.reading_active = false;
                    self.go_to_page(self.current_page - 1);
                }
                ui.label(format!("{} / {}", self.current_page + 1, page_count));
                if ui.button(">").clicked() && self.current_page + 1 < page_count {
                    self.tts.stop();
                    self.reading_active = false;
                    self.go_to_page(self.current_page + 1);
                }

            });

            ui.horizontal(|ui| {
                let tts_state = self.tts.state();

                match tts_state {
                    TtsState::Loading => {
                        ui.spinner();
                        ui.label("Loading Kokoro model...");
                        ctx.request_repaint();
                    }
                    TtsState::Idle | TtsState::Finished | TtsState::Error(_) => {
                        let can_play = self.pdf.is_some() && !self.page_text.is_empty();
                        if ui.add_enabled(can_play, egui::Button::new("Play")).clicked() {
                            self.start_reading();
                        }
                    }
                    TtsState::Generating => {
                        ui.spinner();
                        let (gen_idx, _play, total) = self.tts.progress();
                        ui.label(format!("Generating {}/{}...", gen_idx, total));
                        ctx.request_repaint();
                    }
                    TtsState::Playing => {
                        if ui.button("Pause").clicked() {
                            self.tts.pause();
                        }
                        if ui.button("Stop").clicked() {
                            self.tts.stop();
                            self.reading_active = false;
                        }
                        ctx.request_repaint();
                    }
                    TtsState::Paused => {
                        if ui.button("Resume").clicked() {
                            self.tts.resume();
                        }
                        if ui.button("Stop").clicked() {
                            self.tts.stop();
                            self.reading_active = false;
                        }
                    }
                }

                ui.separator();

                ui.label("Voice:");
                egui::ComboBox::from_id_salt("voice")
                    .selected_text(VOICES[self.selected_voice].1)
                    .show_ui(ui, |ui| {
                        for (i, (_, label)) in VOICES.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_voice, i, *label);
                        }
                    });

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
                    ui.colored_label(Color32::RED, format!("TTS: {}", e));
                }
            });
        });

        // Central panel: PDF page with sentence highlight overlay
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref texture) = self.page_texture {
                egui::ScrollArea::both().show(ui, |ui| {
                    let available_width = ui.available_width();
                    let tex_size = texture.size_vec2();
                    let scale = available_width / tex_size.x;
                    let display_size = Vec2::new(available_width, tex_size.y * scale);

                    // Reserve space and get the rect where the image is drawn
                    let (response, painter) =
                        ui.allocate_painter(display_size, egui::Sense::hover());
                    let image_rect = response.rect;

                    // Draw the PDF image
                    painter.image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    // Draw highlight rectangles for current sentence
                    let is_speaking = matches!(
                        self.tts.state(),
                        TtsState::Playing | TtsState::Generating | TtsState::Paused
                    );
                    if is_speaking {
                        let (sentences, playing_idx) = self.tts.current_sentences();
                        if let Some(sentence) = sentences.get(playing_idx) {
                            if let Some(ref pdf) = self.pdf {
                                let rects = pdf.find_sentence_rects(
                                    self.current_page,
                                    sentence,
                                    self.page_img_size.0,
                                    self.page_img_size.1,
                                );

                                let img_to_screen_x = display_size.x / tex_size.x;
                                let img_to_screen_y = display_size.y / tex_size.y;

                                let highlight_color = Color32::from_rgba_unmultiplied(255, 220, 50, 60);

                                for r in &rects {
                                    let min = Pos2::new(
                                        image_rect.min.x + r.x * img_to_screen_x,
                                        image_rect.min.y + r.y * img_to_screen_y,
                                    );
                                    let max = Pos2::new(
                                        min.x + r.w * img_to_screen_x,
                                        min.y + r.h * img_to_screen_y,
                                    );
                                    painter.rect_filled(
                                        Rect::from_min_max(min, max),
                                        0.0,
                                        highlight_color,
                                    );
                                }
                            }
                        }
                    }
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
