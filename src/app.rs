use crate::library::Library;
use crate::pdf::{PageRender, PdfDoc};
use crate::tts::{TtsEngine, TtsState, VOICES};
use egui::{
    Color32, ColorImage, FontId, Pos2, Rect, RichText, Stroke, TextureHandle, Vec2,
};
use std::path::PathBuf;
use std::sync::mpsc;

// -- Color Palette --
const BG_DARK: Color32 = Color32::from_rgb(24, 24, 32);
const SURFACE: Color32 = Color32::from_rgb(32, 33, 44);
const SURFACE_HOVER: Color32 = Color32::from_rgb(44, 46, 58);
const ACCENT: Color32 = Color32::from_rgb(99, 102, 241);
const GREEN: Color32 = Color32::from_rgb(52, 211, 153);
const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
const RED: Color32 = Color32::from_rgb(239, 68, 68);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(226, 232, 240);
const TEXT_DIM: Color32 = Color32::from_rgb(120, 130, 150);
const HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(99, 102, 241, 50);
const PROGRESS_BG: Color32 = Color32::from_rgb(40, 42, 56);

enum RenderResult {
    Opened { render: PageRender, page_count: usize },
    Rendered(PageRender),
    Error(String),
}

enum AppMode {
    Library,
    Reader { book_id: String },
}

pub struct App {
    mode: AppMode,
    library: Library,
    search_query: String,
    // Reader state
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
    reading_active: bool,
    page_input: String,
    render_rx: mpsc::Receiver<RenderResult>,
    render_tx: mpsc::Sender<RenderResult>,
    loading: bool,
    needs_render_data: Option<PageRender>,
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

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        cc.egui_ctx.set_style(style);

        let (render_tx, render_rx) = mpsc::channel();
        let mut library = Library::load();

        let mut app = Self {
            mode: AppMode::Library,
            library,
            search_query: String::new(),
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
            reading_active: false,
            page_input: String::new(),
            render_rx,
            render_tx,
            loading: false,
            needs_render_data: None,
        };

        // If launched with a PDF argument, import and open it
        if let Some(path) = initial_pdf {
            let pb = PathBuf::from(&path);
            if let Ok(id) = app.library.import(&pb) {
                app.open_book(&id);
            }
        }

        app
    }

    // ── Library actions ──

    fn import_book(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        {
            match self.library.import(&path) {
                Ok(id) => self.open_book(&id),
                Err(e) => self.status_msg = format!("Import error: {}", e),
            }
        }
    }

    fn open_book(&mut self, book_id: &str) {
        if let Some(book) = self.library.get(book_id) {
            let path = book.book_path();
            let start_page = book.last_page;
            self.selected_voice = book.selected_voice;
            self.mode = AppMode::Reader {
                book_id: book_id.to_string(),
            };
            self.open_pdf(path, start_page);
        }
    }

    fn back_to_library(&mut self) {
        self.tts.stop();
        self.reading_active = false;
        // Save current progress
        if let AppMode::Reader { ref book_id } = self.mode {
            self.library
                .update_progress(book_id, self.current_page, self.selected_voice);
        }
        self.pdf = None;
        self.page_texture = None;
        self.mode = AppMode::Library;
    }

    // ── Reader actions ──

    fn open_pdf(&mut self, path: PathBuf, start_page: usize) {
        self.tts.stop();
        self.reading_active = false;
        self.pdf = None;
        self.page_texture = None;
        self.loading = true;
        self.status_msg = "Loading...".into();
        self.current_page = start_page;

        let tx = self.render_tx.clone();
        let path_clone = path.clone();
        self.pdf_path = Some(path);

        std::thread::spawn(move || {
            match PdfDoc::open(&path_clone) {
                Ok(pdf) => {
                    let page_count = pdf.page_count();
                    let page = start_page.min(page_count.saturating_sub(1));
                    match pdf.render_page(page, 1200) {
                        Ok(render) => {
                            let _ = tx.send(RenderResult::Opened { render, page_count });
                        }
                        Err(e) => {
                            let _ = tx.send(RenderResult::Error(format!("{}", e)));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(RenderResult::Error(format!("{}", e)));
                }
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
                        Ok(render) => {
                            let _ = tx.send(RenderResult::Rendered(render));
                        }
                        Err(e) => {
                            let _ = tx.send(RenderResult::Error(format!("{}", e)));
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(RenderResult::Error(format!("{}", e)));
                    }
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
        let voice = VOICES[self.selected_voice].0;
        self.tts.precache_page(&self.page_text, voice);
    }

    fn go_to_page(&mut self, page: usize) {
        let page_count = self.pdf.as_ref().map_or(0, |p| p.page_count());
        if page < page_count {
            self.current_page = page;
            self.page_texture = None;
            // Render synchronously — fast enough for page switches (<100ms)
            if let Some(ref pdf) = self.pdf {
                if let Ok(render) = pdf.render_page(page, 1200) {
                    self.needs_render_data = Some(render);
                }
            }
            // Save progress
            if let AppMode::Reader { ref book_id } = self.mode {
                self.library.update_progress(book_id, page, self.selected_voice);
            }
        }
    }

    fn start_reading(&mut self) {
        let voice = VOICES[self.selected_voice].0.to_string();
        self.tts.speak(self.page_text.clone(), voice, self.speed);
        self.reading_active = true;
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

    // ── UI rendering ──

    fn show_library(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("lib_toolbar")
            .exact_height(52.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Kokoro Reader")
                                .font(FontId::proportional(20.0))
                                .color(TEXT_PRIMARY)
                                .strong(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if styled_button(ui, "Import Book", ACCENT).clicked() {
                                self.import_book();
                            }
                        });
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ctx, |ui| {
                // Search bar
                ui.add_space(4.0);
                let search = egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search books...")
                    .desired_width(ui.available_width())
                    .font(FontId::proportional(15.0));
                ui.add(search);
                ui.add_space(12.0);

                if self.library.books.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        ui.label(
                            RichText::new("No books yet")
                                .font(FontId::proportional(24.0))
                                .color(TEXT_DIM),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Click Import Book to add a PDF")
                                .font(FontId::proportional(14.0))
                                .color(Color32::from_rgb(80, 85, 100)),
                        );
                    });
                    return;
                }

                // Book list
                let query = self.search_query.to_lowercase();
                let mut open_id: Option<String> = None;
                let mut delete_id: Option<String> = None;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for book in &self.library.books {
                        if !query.is_empty() && !book.title.to_lowercase().contains(&query) {
                            continue;
                        }

                        let progress = book.progress();
                        let pct = book.progress_percent();

                        ui.push_id(&book.id, |ui| {
                            let frame = egui::Frame::new()
                                .fill(SURFACE)
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(14))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(45, 47, 60)));

                            frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Book info (clickable)
                                    let response = ui
                                        .vertical(|ui| {
                                            ui.set_min_width(ui.available_width() - 80.0);
                                            ui.label(
                                                RichText::new(&book.title)
                                                    .font(FontId::proportional(16.0))
                                                    .color(TEXT_PRIMARY)
                                                    .strong(),
                                            );
                                            ui.add_space(6.0);

                                            // Progress bar
                                            let bar_height = 6.0;
                                            let bar_width = ui.available_width();
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::new(bar_width, bar_height),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                egui::CornerRadius::same(3),
                                                PROGRESS_BG,
                                            );
                                            let filled = Rect::from_min_size(
                                                rect.min,
                                                Vec2::new(bar_width * progress, bar_height),
                                            );
                                            ui.painter().rect_filled(
                                                filled,
                                                egui::CornerRadius::same(3),
                                                ACCENT,
                                            );

                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new(format!(
                                                    "{}%  —  Page {} / {}",
                                                    pct,
                                                    book.last_page + 1,
                                                    book.total_pages
                                                ))
                                                .color(TEXT_DIM)
                                                .small(),
                                            );
                                        })
                                        .response;

                                    if response.interact(egui::Sense::click()).clicked() {
                                        open_id = Some(book.id.clone());
                                    }

                                    // Delete button
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("X")
                                                            .color(RED)
                                                            .strong(),
                                                    )
                                                    .fill(Color32::TRANSPARENT),
                                                )
                                                .clicked()
                                            {
                                                delete_id = Some(book.id.clone());
                                            }
                                        },
                                    );
                                });
                            });
                            ui.add_space(6.0);
                        });
                    }
                });

                // Process actions after iteration
                if let Some(id) = open_id {
                    self.open_book(&id);
                }
                if let Some(id) = delete_id {
                    self.library.delete(&id);
                }
            });
    }

    fn show_reader(&mut self, ctx: &egui::Context) {
        // Apply deferred page render (from synchronous go_to_page)
        if let Some(render) = self.needs_render_data.take() {
            self.apply_render(render, ctx);
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

        // Process background render results (initial PDF open)
        while let Ok(result) = self.render_rx.try_recv() {
            match result {
                RenderResult::Opened { render, page_count } => {
                    self.status_msg = format!("{} pages", page_count);
                    if let Some(ref path) = self.pdf_path {
                        if let Ok(pdf) = PdfDoc::open(path) {
                            self.pdf = Some(pdf);
                        }
                    }
                    self.apply_render(render, ctx);
                }
                RenderResult::Rendered(render) => {
                    self.apply_render(render, ctx);
                    if self.reading_active {
                        if !self.page_text.trim().is_empty() {
                            self.start_reading();
                        } else {
                            let page_count =
                                self.pdf.as_ref().map_or(0, |p| p.page_count());
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

        // Auto-advance
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

        if self.loading {
            ctx.request_repaint();
        }

        // ── Toolbar ──
        egui::TopBottomPanel::top("toolbar")
            .exact_height(44.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            // Back button
                            if ui
                                .add(egui::Button::new(
                                    RichText::new("< Library").color(TEXT_PRIMARY),
                                ))
                                .clicked()
                            {
                                self.back_to_library();
                                return;
                            }

                            ui.add_space(12.0);

                            // Page navigation
                            let page_count =
                                self.pdf.as_ref().map_or(0, |p| p.page_count());

                            if ui
                                .add_enabled(
                                    self.current_page > 0,
                                    egui::Button::new(RichText::new("<").strong()),
                                )
                                .clicked()
                            {
                                self.tts.stop();
                                self.reading_active = false;
                                self.go_to_page(self.current_page - 1);
                            }

                            let input = egui::TextEdit::singleline(&mut self.page_input)
                                .desired_width(36.0)
                                .horizontal_align(egui::Align::Center)
                                .font(FontId::proportional(14.0))
                                .return_key(egui::KeyboardShortcut::new(
                                    egui::Modifiers::NONE,
                                    egui::Key::Enter,
                                ));
                            let output = input.show(ui);
                            if output.response.lost_focus() {
                                if let Ok(n) = self.page_input.trim().parse::<usize>() {
                                    if n >= 1 && n <= page_count {
                                        self.tts.stop();
                                        self.reading_active = false;
                                        self.go_to_page(n - 1);
                                    }
                                }
                            }
                            if !output.response.has_focus() {
                                self.page_input =
                                    format!("{}", self.current_page + 1);
                            }

                            ui.label(
                                RichText::new(format!("/ {}", page_count))
                                    .color(TEXT_DIM)
                                    .font(FontId::proportional(14.0)),
                            );

                            if ui
                                .add_enabled(
                                    self.current_page + 1 < page_count,
                                    egui::Button::new(RichText::new(">").strong()),
                                )
                                .clicked()
                            {
                                self.tts.stop();
                                self.reading_active = false;
                                self.go_to_page(self.current_page + 1);
                            }

                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // TTS controls
                            let tts_state = self.tts.state();
                            match tts_state {
                                TtsState::Loading => {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("Loading model...").color(AMBER),
                                    );
                                    ctx.request_repaint();
                                }
                                TtsState::Idle | TtsState::Finished | TtsState::Error(_) => {
                                    let can_play =
                                        self.pdf.is_some() && !self.page_text.is_empty();
                                    let btn = egui::Button::new(
                                        RichText::new("Play")
                                            .color(Color32::WHITE)
                                            .strong(),
                                    )
                                    .fill(GREEN)
                                    .corner_radius(egui::CornerRadius::same(6));
                                    if ui.add_enabled(can_play, btn).clicked() {
                                        self.start_reading();
                                    }
                                }
                                TtsState::Generating => {
                                    ui.spinner();
                                    let (g, _, t) = self.tts.progress();
                                    ui.label(
                                        RichText::new(format!("{}/{}...", g, t))
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

                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // Voice
                            egui::menu::menu_button(
                                ui,
                                RichText::new(format!(
                                    "Voice: {}",
                                    VOICES[self.selected_voice].1
                                ))
                                .color(TEXT_PRIMARY),
                                |ui| {
                                    for (i, (_, label)) in VOICES.iter().enumerate() {
                                        if ui
                                            .selectable_label(
                                                self.selected_voice == i,
                                                *label,
                                            )
                                            .clicked()
                                        {
                                            self.selected_voice = i;
                                            ui.close_menu();
                                        }
                                    }
                                },
                            );

                            ui.add_space(8.0);

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
                        },
                    );
                });
            });

        // ── Status bar ──
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if !self.status_msg.is_empty() {
                        ui.label(RichText::new(&self.status_msg).color(TEXT_DIM).small());
                    }
                    if let TtsState::Error(ref e) = self.tts.state() {
                        ui.label(
                            RichText::new(format!("Error: {}", e)).color(RED).small(),
                        );
                    }
                });
            });

        // ── PDF viewer ──
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_DARK).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                // Ctrl+scroll zoom
                let zoom_ratio = if ui.input(|i| i.modifiers.ctrl) {
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        let old = self.zoom;
                        self.zoom = (self.zoom + scroll * 0.001).clamp(0.3, 5.0);
                        self.zoom / old
                    } else {
                        1.0
                    }
                } else {
                    1.0
                };

                if let Some(ref texture) = self.page_texture {
                    let scroll_area = egui::ScrollArea::both()
                        .id_salt("pdf_scroll")
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        );
                    let scroll_output = scroll_area.show(ui, |ui| {
                        let available_size = ui.available_size();
                        let tex_size = texture.size_vec2();
                        let scale = (available_size.x / tex_size.x) * self.zoom;
                        let display_size =
                            Vec2::new(tex_size.x * scale, tex_size.y * scale);

                        let pad_x =
                            ((available_size.x - display_size.x) / 2.0).max(0.0);
                        let pad_y =
                            ((available_size.y - display_size.y) / 2.0).max(0.0);

                        let total = Vec2::new(
                            display_size.x + pad_x * 2.0,
                            display_size.y + pad_y * 2.0,
                        );
                        let (response, painter) =
                            ui.allocate_painter(total, egui::Sense::hover());
                        let image_rect = Rect::from_min_size(
                            response.rect.min + Vec2::new(pad_x, pad_y),
                            display_size,
                        );

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
                            let (sentences, idx) = self.tts.current_sentences();
                            if let Some(sentence) = sentences.get(idx) {
                                if let Some(ref pdf) = self.pdf {
                                    let next =
                                        sentences.get(idx + 1).map(|s| s.as_str());
                                    let rects = pdf.find_sentence_rects(
                                        self.current_page,
                                        sentence,
                                        next,
                                        self.page_img_size.0,
                                        self.page_img_size.1,
                                    );
                                    let sx = display_size.x / tex_size.x;
                                    let sy = display_size.y / tex_size.y;
                                    for r in &rects {
                                        let min = Pos2::new(
                                            image_rect.min.x + r.x * sx,
                                            image_rect.min.y + r.y * sy,
                                        );
                                        let max = Pos2::new(
                                            min.x + r.w * sx,
                                            min.y + r.h * sy,
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

                    if zoom_ratio != 1.0 {
                        let vp = scroll_output.inner_rect.size();
                        let mut off = scroll_output.state.offset;
                        let cx = off.x + vp.x / 2.0;
                        let cy = off.y + vp.y / 2.0;
                        off.x = (cx * zoom_ratio - vp.x / 2.0).max(0.0);
                        off.y = (cy * zoom_ratio - vp.y / 2.0).max(0.0);
                        let mut state = scroll_output.state;
                        state.offset = off;
                        state.store(ui.ctx(), scroll_output.id);
                    }
                } else if self.loading {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                    });
                }
            });
    }
}

fn styled_button(ui: &mut egui::Ui, label: &str, color: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
            .fill(color)
            .corner_radius(egui::CornerRadius::same(6)),
    )
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        match self.mode {
            AppMode::Library => self.show_library(ctx),
            AppMode::Reader { .. } => self.show_reader(ctx),
        }
    }
}
