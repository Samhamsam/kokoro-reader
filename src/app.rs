use crate::pdf::{PageRender, PdfDoc};
use crate::tts::{TtsEngine, TtsState, VOICES};
use egui::{
    Color32, ColorImage, FontId, Pos2, Rect, RichText, Rounding, Stroke, TextureHandle, Vec2,
};
use std::sync::mpsc;
use std::path::PathBuf;

// -- Color Palette --
const BG_DARK: Color32 = Color32::from_rgb(24, 24, 32);
const SURFACE: Color32 = Color32::from_rgb(32, 33, 44);
const SURFACE_HOVER: Color32 = Color32::from_rgb(44, 46, 58);
const ACCENT: Color32 = Color32::from_rgb(99, 102, 241);    // indigo
const ACCENT_HOVER: Color32 = Color32::from_rgb(129, 131, 252);
const GREEN: Color32 = Color32::from_rgb(52, 211, 153);     // playing
const AMBER: Color32 = Color32::from_rgb(251, 191, 36);     // generating
const RED: Color32 = Color32::from_rgb(239, 68, 68);        // stop/error
const TEXT_PRIMARY: Color32 = Color32::from_rgb(226, 232, 240);
const TEXT_DIM: Color32 = Color32::from_rgb(120, 130, 150);
const HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(99, 102, 241, 50);

enum RenderResult {
    Opened {
        render: PageRender,
        filename: String,
        page_count: usize,
    },
    Rendered(PageRender),
    Error(String),
}

pub struct App {
    pdf: Option<PdfDoc>,
    pdf_path: Option<PathBuf>,
    current_page: usize,
    page_texture: Option<TextureHandle>,
    page_text: String,
    page_img_size: (usize, usize),
    zoom: f32,
    tts: TtsEngine,
    selected_voice: usize,
    speed: f32,
    status_msg: String,
    needs_render: bool,
    reading_active: bool,
    page_input: String,
    /// Receives render results from background thread
    render_rx: mpsc::Receiver<RenderResult>,
    render_tx: mpsc::Sender<RenderResult>,
    loading: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, initial_pdf: Option<String>) -> Self {
        // Apply dark theme
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG_DARK;
        visuals.window_fill = SURFACE;
        visuals.extreme_bg_color = Color32::from_rgb(16, 16, 22);
        visuals.widgets.noninteractive.bg_fill = SURFACE;
        visuals.widgets.inactive.bg_fill = SURFACE;
        visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
        visuals.widgets.active.bg_fill = ACCENT;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
        visuals.selection.bg_fill = ACCENT;
        cc.egui_ctx.set_visuals(visuals);

        // Slightly larger default font
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        cc.egui_ctx.set_style(style);

        let (render_tx, render_rx) = mpsc::channel();

        let mut result = Self {
            pdf: None,
            pdf_path: None,
            current_page: 0,
            page_texture: None,
            page_text: String::new(),
            page_img_size: (0, 0),
            zoom: 1.0,
            tts: TtsEngine::new(),
            selected_voice: 0,
            speed: 1.0,
            status_msg: String::new(),
            needs_render: false,
            reading_active: false,
            page_input: String::new(),
            render_rx,
            render_tx,
            loading: false,
        };

        if let Some(path) = initial_pdf {
            result.open_pdf(PathBuf::from(path));
        }

        result
    }

    fn open_pdf(&mut self, path: PathBuf) {
        self.tts.stop();
        self.reading_active = false;
        self.pdf = None;
        self.page_texture = None;
        self.loading = true;
        self.status_msg = "Loading...".into();

        let tx = self.render_tx.clone();
        let path_clone = path.clone();
        self.pdf_path = Some(path);

        // Open and render first page in background
        std::thread::spawn(move || {
            match PdfDoc::open(&path_clone) {
                Ok(pdf) => {
                    let page_count = pdf.page_count();
                    let filename = path_clone.file_name()
                        .unwrap_or_default().to_string_lossy().to_string();
                    match pdf.render_page(0, 1200) {
                        Ok(render) => {
                            let _ = tx.send(RenderResult::Opened { render, filename, page_count });
                        }
                        Err(e) => { let _ = tx.send(RenderResult::Error(format!("{}", e))); }
                    }
                }
                Err(e) => { let _ = tx.send(RenderResult::Error(format!("{}", e))); }
            }
        });
    }

    fn request_render(&self, page: usize) {
        if let Some(ref path) = self.pdf_path {
            let tx = self.render_tx.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                match PdfDoc::open(&path) {
                    Ok(pdf) => match pdf.render_page(page, 1200) {
                        Ok(render) => { let _ = tx.send(RenderResult::Rendered(render)); }
                        Err(e) => { let _ = tx.send(RenderResult::Error(format!("{}", e))); }
                    }
                    Err(e) => { let _ = tx.send(RenderResult::Error(format!("{}", e))); }
                }
            });
        }
    }

    fn apply_render(&mut self, render: PageRender, ctx: &egui::Context) {
        let PageRender { rgba, width, height, text } = render;
        let image = ColorImage::from_rgba_unmultiplied([width, height], &rgba);
        self.page_texture = Some(ctx.load_texture(
            format!("page-{}", self.current_page),
            image,
            Default::default(),
        ));
        self.page_text = text;
        self.page_img_size = (width, height);
        self.loading = false;
        // Pre-synthesize first sentences
        let voice = VOICES[self.selected_voice].0;
        self.tts.precache_page(&self.page_text, voice);
    }

    fn go_to_page(&mut self, page: usize) {
        if let Some(ref pdf) = self.pdf {
            if page < pdf.page_count() {
                self.current_page = page;
                self.page_texture = None;
                self.loading = true;
                self.request_render(page);
            }
        }
    }

    fn start_reading(&mut self) {
        let voice = VOICES[self.selected_voice].0.to_string();
        self.tts.speak(self.page_text.clone(), voice, self.speed);
        self.reading_active = true;
        // Pre-cache the NEXT page so the transition is instant
        self.precache_next_page();
    }

    fn precache_next_page(&self) {
        let next_page = self.current_page + 1;
        if let Some(ref pdf) = self.pdf {
            if next_page < pdf.page_count() {
                if let Ok(text) = pdf.page_text(next_page) {
                    let voice = VOICES[self.selected_voice].0;
                    self.tts.precache_page(&text, voice);
                }
            }
        }
    }
}

fn styled_button(ui: &mut egui::Ui, label: &str, color: Color32) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
        .fill(color)
        .rounding(egui::CornerRadius::same(6));
    ui.add(btn)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process background render results
        while let Ok(result) = self.render_rx.try_recv() {
            match result {
                RenderResult::Opened { render, filename, page_count } => {
                    self.status_msg = format!("{} -- {} pages", filename, page_count);
                    // Open PdfDoc on main thread (needed for highlighting, not Send)
                    if let Some(ref path) = self.pdf_path {
                        if let Ok(pdf) = PdfDoc::open(path) {
                            self.pdf = Some(pdf);
                        }
                    }
                    self.current_page = 0;
                    self.apply_render(render, ctx);
                }
                RenderResult::Rendered(render) => {
                    self.apply_render(render, ctx);
                    // Auto-start reading if we were reading
                    if self.reading_active {
                        if !self.page_text.trim().is_empty() {
                            self.start_reading();
                        } else {
                            let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
                            if self.current_page + 1 < page_count {
                                self.go_to_page(self.current_page + 1);
                            } else {
                                self.reading_active = false;
                            }
                        }
                    }
                }
                RenderResult::Error(e) => {
                    self.status_msg = format!("Error: {}", e);
                    self.loading = false;
                }
            }
        }

        // Auto-advance when TTS finished
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

        // Keep repainting while loading
        if self.loading {
            ctx.request_repaint();
        }

        // ── Top toolbar ──
        egui::TopBottomPanel::top("toolbar")
            .exact_height(44.0)
            .frame(egui::Frame::new().fill(SURFACE).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ctx, |ui| {
                let toolbar_rect = ui.available_rect_before_wrap();
                ui.allocate_ui_at_rect(toolbar_rect, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // Open button
                    if styled_button(ui, "Open PDF", ACCENT).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file()
                        {
                            self.open_pdf(path);
                        }
                    }

                    ui.add_space(16.0);

                    // Page navigation
                    let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());

                    if ui.add_enabled(
                        self.current_page > 0,
                        egui::Button::new(RichText::new("<").strong()),
                    ).clicked() {
                        self.tts.stop();
                        self.reading_active = false;
                        self.go_to_page(self.current_page - 1);
                        self.page_input = format!("{}", self.current_page + 1);
                    }

                    // Editable page number field
                    let input = egui::TextEdit::singleline(&mut self.page_input)
                        .desired_width(36.0)
                        .horizontal_align(egui::Align::Center)
                        .font(FontId::proportional(14.0))
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Enter));
                    let output = input.show(ui);
                    // Navigate FIRST (before sync overwrites the input)
                    if output.response.lost_focus() {
                        if let Ok(page_num) = self.page_input.trim().parse::<usize>() {
                            if page_num >= 1 && page_num <= page_count {
                                self.tts.stop();
                                self.reading_active = false;
                                self.go_to_page(page_num - 1);
                            }
                        }
                    }
                    // Sync display when not editing
                    if !output.response.has_focus() {
                        self.page_input = format!("{}", self.current_page + 1);
                    }

                    ui.label(
                        RichText::new(format!("/ {}", page_count))
                            .color(TEXT_DIM)
                            .font(FontId::proportional(14.0)),
                    );

                    if ui.add_enabled(
                        self.current_page + 1 < page_count,
                        egui::Button::new(RichText::new(">").strong()),
                    ).clicked() {
                        self.tts.stop();
                        self.reading_active = false;
                        self.go_to_page(self.current_page + 1);
                        self.page_input = format!("{}", self.current_page + 1);
                    }

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // TTS controls
                    let tts_state = self.tts.state();

                    match tts_state {
                        TtsState::Loading => {
                            ui.spinner();
                            ui.label(RichText::new("Loading model...").color(AMBER));
                            ctx.request_repaint();
                        }
                        TtsState::Idle | TtsState::Finished | TtsState::Error(_) => {
                            let can_play = self.pdf.is_some() && !self.page_text.is_empty();
                            let btn = egui::Button::new(
                                RichText::new("Play").color(Color32::WHITE).strong()
                            ).fill(GREEN).rounding(egui::CornerRadius::same(6));
                            if ui.add_enabled(can_play, btn).clicked() {
                                self.start_reading();
                            }
                        }
                        TtsState::Generating => {
                            ui.spinner();
                            let (gen_idx, _play, total) = self.tts.progress();
                            ui.label(
                                RichText::new(format!("Generating {}/{}...", gen_idx, total))
                                    .color(AMBER),
                            );
                            ctx.request_repaint();
                        }
                        TtsState::Playing => {
                            if styled_button(ui, "Pause", AMBER).clicked() {
                                self.tts.pause();
                            }
                            if styled_button(ui, "Stop", RED).clicked() {
                                self.tts.stop();
                                self.reading_active = false;
                            }
                            ctx.request_repaint();
                        }
                        TtsState::Paused => {
                            if styled_button(ui, "Resume", GREEN).clicked() {
                                self.tts.resume();
                            }
                            if styled_button(ui, "Stop", RED).clicked() {
                                self.tts.stop();
                                self.reading_active = false;
                            }
                        }
                    }

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Voice
                    egui::menu::menu_button(
                        ui,
                        RichText::new(format!("Voice: {}", VOICES[self.selected_voice].1))
                            .color(TEXT_PRIMARY),
                        |ui| {
                            for (i, (_, label)) in VOICES.iter().enumerate() {
                                if ui.selectable_label(self.selected_voice == i, *label).clicked() {
                                    self.selected_voice = i;
                                    ui.close_menu();
                                }
                            }
                        },
                    );

                    ui.add_space(8.0);

                    // Speed
                    ui.label(
                        RichText::new(format!("Speed: {:.1}x", self.speed))
                            .color(TEXT_PRIMARY),
                    );
                    let old_speed = self.speed;
                    ui.add(
                        egui::Slider::new(&mut self.speed, 0.5..=2.0)
                            .step_by(0.1)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if (self.speed - old_speed).abs() > 0.01 {
                        self.tts.set_speed(self.speed);
                    }
                    });
                });
            });

        // ── Bottom status bar ──
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(SURFACE).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if !self.status_msg.is_empty() {
                        ui.label(RichText::new(&self.status_msg).color(TEXT_DIM).small());
                    }
                    if let TtsState::Error(ref e) = self.tts.state() {
                        ui.label(RichText::new(format!("Error: {}", e)).color(RED).small());
                    }
                });
            });

        // ── Central panel: PDF ──
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_DARK).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                // Ctrl+scroll = zoom toward center
                let zoom_ratio = if ui.input(|i| i.modifiers.ctrl) {
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        let old_zoom = self.zoom;
                        self.zoom = (self.zoom + scroll * 0.001).clamp(0.3, 5.0);
                        self.zoom / old_zoom
                    } else {
                        1.0
                    }
                } else {
                    1.0
                };

                if let Some(ref texture) = self.page_texture {
                    let scroll_id = egui::Id::new("pdf_scroll");
                    let scroll_area = egui::ScrollArea::both()
                        .id_salt(scroll_id)
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
                    let scroll_output = scroll_area.show(ui, |ui| {
                        let available_size = ui.available_size();
                        let tex_size = texture.size_vec2();
                        let base_scale = available_size.x / tex_size.x;
                        let scale = base_scale * self.zoom;
                        let display_size = Vec2::new(tex_size.x * scale, tex_size.y * scale);

                        // Center the image when it's smaller than the viewport
                        let pad_x = ((available_size.x - display_size.x) / 2.0).max(0.0);
                        let pad_y = ((available_size.y - display_size.y) / 2.0).max(0.0);
                        if pad_x > 0.0 || pad_y > 0.0 {
                            ui.add_space(0.0); // ensure layout is active
                        }

                        // Allocate centering space + image
                        let total_size = Vec2::new(
                            display_size.x + pad_x * 2.0,
                            display_size.y + pad_y * 2.0,
                        );
                        let (response, painter) =
                            ui.allocate_painter(total_size, egui::Sense::hover());
                        let image_rect = Rect::from_min_size(
                            response.rect.min + Vec2::new(pad_x, pad_y),
                            display_size,
                        );

                        // PDF page with subtle shadow effect
                        painter.rect_filled(
                            image_rect.expand(1.0),
                            egui::CornerRadius::same(2),
                            Color32::from_rgb(40, 42, 54),
                        );
                        painter.image(
                            texture.id(),
                            image_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );

                        // Sentence highlight
                        let is_speaking = matches!(
                            self.tts.state(),
                            TtsState::Playing | TtsState::Generating | TtsState::Paused
                        );
                        if is_speaking {
                            let (sentences, playing_idx) = self.tts.current_sentences();
                            if let Some(sentence) = sentences.get(playing_idx) {
                                if let Some(ref pdf) = self.pdf {
                                    let next_sentence =
                                        sentences.get(playing_idx + 1).map(|s| s.as_str());
                                    let rects = pdf.find_sentence_rects(
                                        self.current_page,
                                        sentence,
                                        next_sentence,
                                        self.page_img_size.0,
                                        self.page_img_size.1,
                                    );

                                    let img_to_screen_x = display_size.x / tex_size.x;
                                    let img_to_screen_y = display_size.y / tex_size.y;

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
                                            egui::CornerRadius::same(2),
                                            HIGHLIGHT,
                                        );
                                    }
                                }
                            }
                        }
                    });

                    // After zoom, adjust scroll offset to keep center stable
                    if zoom_ratio != 1.0 {
                        let viewport_size = scroll_output.inner_rect.size();
                        let mut offset = scroll_output.state.offset;
                        // Center in content space before zoom
                        let center_x = offset.x + viewport_size.x / 2.0;
                        let center_y = offset.y + viewport_size.y / 2.0;
                        // New offset to keep same center after zoom
                        offset.x = center_x * zoom_ratio - viewport_size.x / 2.0;
                        offset.y = center_y * zoom_ratio - viewport_size.y / 2.0;
                        offset.x = offset.x.max(0.0);
                        offset.y = offset.y.max(0.0);
                        // Write back
                        let mut state = scroll_output.state;
                        state.offset = offset;
                        state.store(ui.ctx(), scroll_output.id);
                    }
                } else if self.pdf.is_none() {
                    // Empty state — welcoming drop zone
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            ui.label(
                                RichText::new("Drop a PDF here")
                                    .font(FontId::proportional(28.0))
                                    .color(TEXT_DIM),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("or click Open PDF to get started")
                                    .font(FontId::proportional(16.0))
                                    .color(Color32::from_rgb(80, 85, 100)),
                            );
                        });
                    });
                }
            });

        // Drag & drop
        let dropped_path = ctx.input(|i| {
            i.raw.dropped_files.first().and_then(|f| {
                f.path.as_ref().and_then(|p| {
                    if p.extension().is_some_and(|e| e == "pdf") {
                        Some(p.clone())
                    } else {
                        None
                    }
                })
            })
        });
        if let Some(path) = dropped_path {
            self.open_pdf(path);
        }
    }
}
