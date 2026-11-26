use crate::pdf_engine::{
    export_pdfs_to_images, inspect_input_files, merge_and_save_pdf, ExportFormat, FileKind,
    InputFile, ProcessSettings, ProgressUpdate, TaskOutcome,
};
use crate::theme::{apply_theme, configure_fonts, ThemePalette};
use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, FontId, Frame, Layout, Margin, Pos2,
    RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

enum WorkerMsg {
    Progress(ProgressUpdate),
    Finished(TaskOutcome),
}

struct RunningJob {
    rx: Receiver<WorkerMsg>,
    cancel: Arc<AtomicBool>,
    progress: ProgressUpdate,
}

pub struct PdfAraciApp {
    files: Vec<InputFile>,
    settings: ProcessSettings,
    dark_mode: bool,
    theme_applied: Option<bool>,
    icon_texture: Option<TextureHandle>,
    running: Option<RunningJob>,
    last_outcome: Option<TaskOutcome>,
    ui_notice: Option<String>,
}

impl PdfAraciApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        let icon_texture = load_icon_texture(&cc.egui_ctx);

        let palette = ThemePalette::light();
        apply_theme(&cc.egui_ctx, &palette);

        Self {
            files: Vec::new(),
            settings: ProcessSettings::default(),
            dark_mode: false,
            theme_applied: Some(false),
            icon_texture,
            running: None,
            last_outcome: None,
            ui_notice: None,
        }
    }

    fn palette(&self) -> ThemePalette {
        if self.dark_mode {
            ThemePalette::dark()
        } else {
            ThemePalette::light()
        }
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let inspected = inspect_input_files(&paths);
        let skipped = paths.len().saturating_sub(inspected.len());
        let mut added = 0usize;
        for item in inspected {
            if !self.files.iter().any(|existing| existing.path == item.path) {
                self.files.push(item);
                added += 1;
            }
        }
        if added > 0 {
            self.last_outcome = None;
            self.ui_notice = None;
        } else if skipped > 0 {
            self.ui_notice = Some("Yalnızca .pdf, .png, .jpg ve .jpeg dosyaları desteklenir.".to_string());
        }
    }

    fn pick_files_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("PDF veya Görsel Dosyaları Seçin")
            .add_filter("Desteklenen Dosyalar", &["pdf", "png", "jpg", "jpeg"])
            .add_filter("PDF Dosyaları", &["pdf"])
            .add_filter("Görseller", &["png", "jpg", "jpeg"])
            .pick_files()
        {
            self.add_paths(paths);
        }
    }

    fn start_merge_pdf(&mut self, ctx: &egui::Context) {
        if self.files.is_empty() {
            self.ui_notice = Some("Lütfen önce en az bir PDF veya görsel dosyası ekleyin.".to_string());
            return;
        }

        let default_name = self
            .files
            .first()
            .and_then(|f| f.path.file_stem())
            .map(|s| format!("{}_optimize.pdf", s.to_string_lossy()))
            .unwrap_or_else(|| "birlesik_belge.pdf".to_string());

        let mut dialog = rfd::FileDialog::new()
            .set_title("PDF Olarak Kaydet")
            .set_file_name(&default_name)
            .add_filter("PDF Dosyası", &["pdf"]);

        if let Some(parent) = self.files.first().and_then(|f| f.path.parent()) {
            dialog = dialog.set_directory(parent);
        }

        let Some(mut out_path) = dialog.save_file() else {
            return;
        };
        if out_path.extension().is_none() {
            out_path.set_extension("pdf");
        }

        let files = self.files.clone();
        let settings = self.settings;
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let ctx_clone = ctx.clone();

        self.last_outcome = None;
        self.ui_notice = None;
        self.running = Some(RunningJob {
            rx,
            cancel,
            progress: ProgressUpdate {
                current: 0,
                total: 1,
                message: "İşlem hazırlanıyor...".to_string(),
            },
        });

        std::thread::spawn(move || {
            let outcome = merge_and_save_pdf(&files, &out_path, settings, cancel_worker, |upd| {
                let _ = tx.send(WorkerMsg::Progress(upd));
                ctx_clone.request_repaint();
            });
            let _ = tx.send(WorkerMsg::Finished(outcome));
            ctx_clone.request_repaint();
        });
    }

    fn start_export_images(&mut self, format: ExportFormat, ctx: &egui::Context) {
        let pdf_count = self.files.iter().filter(|f| f.kind == FileKind::Pdf).count();
        if pdf_count == 0 {
            self.ui_notice = Some("Görsele çevirmek için listede en az bir PDF dosyası olmalı.".to_string());
            return;
        }

        let mut dialog = rfd::FileDialog::new().set_title(format!(
            "Görsellerin ({}) Kaydedileceği Klasörü Seçin",
            format.label()
        ));
        if let Some(parent) = self
            .files
            .iter()
            .find(|f| f.kind == FileKind::Pdf)
            .and_then(|f| f.path.parent())
        {
            dialog = dialog.set_directory(parent);
        }

        let Some(out_dir) = dialog.pick_folder() else {
            return;
        };

        let files = self.files.clone();
        let settings = self.settings;
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let ctx_clone = ctx.clone();

        self.last_outcome = None;
        self.ui_notice = None;
        self.running = Some(RunningJob {
            rx,
            cancel,
            progress: ProgressUpdate {
                current: 0,
                total: 1,
                message: format!("PDF sayfaları {} formatına hazırlanıyor...", format.label()),
            },
        });

        std::thread::spawn(move || {
            let outcome =
                export_pdfs_to_images(&files, &out_dir, format, settings, cancel_worker, |upd| {
                    let _ = tx.send(WorkerMsg::Progress(upd));
                    ctx_clone.request_repaint();
                });
            let _ = tx.send(WorkerMsg::Finished(outcome));
            ctx_clone.request_repaint();
        });
    }

    fn poll_worker(&mut self) {
        let mut finished: Option<TaskOutcome> = None;
        if let Some(job) = &mut self.running {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    WorkerMsg::Progress(p) => job.progress = p,
                    WorkerMsg::Finished(out) => {
                        finished = Some(out);
                        break;
                    }
                }
            }
        }
        if let Some(out) = finished {
            self.running = None;
            self.last_outcome = Some(out);
        }
    }

    fn handle_drag_drop(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        if self.running.is_some() {
            return;
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }

        let is_hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if is_hovering {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(15, 23, 42, 165));
            let card = screen.shrink(36.0);
            painter.rect(
                card,
                CornerRadius::same(18),
                Color32::from_rgba_unmultiplied(79, 70, 229, 36),
                Stroke::new(2.5_f32, p.border_accent),
                StrokeKind::Inside,
            );
            painter.text(
                screen.center() - Vec2::new(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                "Dosyaları Buraya Bırakın",
                FontId::proportional(24.0),
                Color32::WHITE,
            );
            painter.text(
                screen.center() + Vec2::new(0.0, 22.0),
                egui::Align2::CENTER_CENTER,
                "PDF, PNG, JPG ve JPEG dosyaları otomatik olarak kuyruğa eklenir",
                FontId::proportional(14.0),
                Color32::from_rgb(203, 213, 225),
            );
        }
    }
}

impl eframe::App for PdfAraciApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        if self.theme_applied != Some(self.dark_mode) {
            let p = self.palette();
            apply_theme(ctx, &p);
            self.theme_applied = Some(self.dark_mode);
        }

        let p = self.palette();
        let is_busy = self.running.is_some();

        // Top Header Bar
        egui::TopBottomPanel::top("header_bar")
            .frame(
                Frame::new()
                    .fill(p.bg_card)
                    .inner_margin(Margin::symmetric(20, 14))
                    .stroke(Stroke::new(1.0_f32, p.border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(tex) = &self.icon_texture {
                        ui.add(
                            egui::Image::new(tex)
                                .fit_to_exact_size(Vec2::splat(38.0))
                                .corner_radius(CornerRadius::same(9)),
                        );
                        ui.add_space(8.0);
                    }

                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new("PDF Optimize & Dönüştürme Aracı")
                                .size(18.0)
                                .strong()
                                .color(p.text_primary),
                        );
                        ui.label(
                            RichText::new(
                                "PDF ve görselleri sıralayın, sıkıştırın veya yüksek kalitede dışa aktarın",
                            )
                            .size(12.0)
                            .color(p.text_muted),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let theme_label = if self.dark_mode { "☀ Açık Tema" } else { "☾ Koyu Tema" };
                        if ui
                            .add(
                                egui::Button::new(RichText::new(theme_label).size(12.0))
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                        }

                        if !self.files.is_empty() {
                            let total_bytes: u64 = self.files.iter().map(|f| f.size_bytes).sum();
                            let total_pages: usize =
                                self.files.iter().filter_map(|f| f.page_count).sum();
                            let summary = format!(
                                "{} dosya  •  {} sayfa  •  {}",
                                self.files.len(),
                                total_pages,
                                format_bytes(total_bytes)
                            );
                            Frame::new()
                                .fill(p.bg_card_alt)
                                .stroke(Stroke::new(1.0_f32, p.border))
                                .corner_radius(CornerRadius::same(16))
                                .inner_margin(Margin::symmetric(12, 5))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(summary).size(12.0).color(p.text_muted),
                                    );
                                });
                        }
                    });
                });
            });

        // Main Bento Workspace
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg_canvas).inner_margin(Margin::same(18)))
            .show(ctx, |ui| {
                let avail_w = ui.available_width();
                let gap = 16.0;
                let left_w = ((avail_w - gap) * 0.57).max(340.0);
                let right_w = (avail_w - gap - left_w).max(280.0);
                let panel_h = ui.available_height();

                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;

                    // LEFT COLUMN: File Queue Card
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, panel_h),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            render_card(ui, &p, Vec2::new(left_w, panel_h), |ui| {
                                self.render_queue_panel(ui, &p, is_busy);
                            });
                        },
                    );

                    // RIGHT COLUMN: Settings + Actions + Status
                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, panel_h),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 14.0;

                            // Settings Card
                            render_card(ui, &p, Vec2::new(right_w, 0.0), |ui| {
                                self.render_settings_panel(ui, &p, is_busy);
                            });

                            // Action Card
                            render_card(ui, &p, Vec2::new(right_w, 0.0), |ui| {
                                self.render_actions_panel(ui, ctx, &p, is_busy);
                            });

                            // Status / Outcome Card
                            self.render_status_panel(ui, &p, right_w);
                        },
                    );
                });
            });

        self.handle_drag_drop(ctx, &p);
    }
}

impl PdfAraciApp {
    fn render_queue_panel(&mut self, ui: &mut egui::Ui, p: &ThemePalette, is_busy: bool) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Dosya Kuyruğu")
                    .size(15.0)
                    .strong()
                    .color(p.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.files.is_empty() {
                    if ui
                        .add_enabled(
                            !is_busy,
                            egui::Button::new(
                                RichText::new("Temizle").size(12.0).color(p.danger),
                            )
                            .fill(p.bg_card_alt),
                        )
                        .clicked()
                    {
                        self.files.clear();
                        self.last_outcome = None;
                        self.ui_notice = None;
                    }
                }
                if ui
                    .add_enabled(
                        !is_busy,
                        egui::Button::new(
                            RichText::new("+ Dosya Ekle")
                                .size(12.5)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(p.primary)
                        .stroke(Stroke::NONE),
                    )
                    .clicked()
                {
                    self.pick_files_dialog();
                }
            });
        });

        ui.add_space(6.0);

        if self.files.is_empty() {
            let drop_h = (ui.available_height() - 8.0).max(200.0);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), drop_h), Sense::click());

            let hovered = response.hovered() && !is_busy;
            let fill = if hovered { p.bg_hover } else { p.bg_card_alt };
            let stroke_col = if hovered { p.primary } else { p.border };

            ui.painter().rect(
                rect,
                CornerRadius::same(12),
                fill,
                Stroke::new(1.5_f32, stroke_col),
                StrokeKind::Inside,
            );

            // Vector document/upload emblem
            let center = rect.center() - Vec2::new(0.0, 26.0);
            draw_upload_emblem(ui.painter(), center, p.primary);

            ui.painter().text(
                rect.center() + Vec2::new(0.0, 22.0),
                egui::Align2::CENTER_CENTER,
                "PDF, PNG veya JPG dosyalarını buraya sürükleyin",
                FontId::proportional(14.5),
                p.text_primary,
            );
            ui.painter().text(
                rect.center() + Vec2::new(0.0, 46.0),
                egui::Align2::CENTER_CENTER,
                "veya dosya seçmek için bu alana tıklayın",
                FontId::proportional(12.5),
                p.text_muted,
            );

            if response.clicked() && !is_busy {
                self.pick_files_dialog();
            }
            return;
        }

        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let mut remove_idx: Option<usize> = None;
        let total_files = self.files.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 7.0;
                for (idx, item) in self.files.iter().enumerate() {
                    Frame::new()
                        .fill(p.bg_card_alt)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Index badge
                                ui.label(
                                    RichText::new(format!("{}.", idx + 1))
                                        .size(12.5)
                                        .strong()
                                        .color(p.text_muted),
                                );

                                // File type pill
                                let (pill_bg, pill_fg) = match item.kind {
                                    FileKind::Pdf => (p.pdf_pill_bg, p.pdf_pill_fg),
                                    FileKind::Png | FileKind::Jpeg => (p.img_pill_bg, p.img_pill_fg),
                                };
                                Frame::new()
                                    .fill(pill_bg)
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(Margin::symmetric(7, 2))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(item.kind.label())
                                                .size(10.5)
                                                .strong()
                                                .color(pill_fg),
                                        );
                                    });

                                ui.add_space(2.0);

                                // Controls first on right so name truncates cleanly
                                let ctrl_w = 96.0;
                                let name_max_w = (ui.available_width() - ctrl_w).max(80.0);

                                ui.allocate_ui_with_layout(
                                    Vec2::new(name_max_w, 32.0),
                                    Layout::top_down(Align::LEFT),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&item.name)
                                                    .size(13.0)
                                                    .strong()
                                                    .color(p.text_primary),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(item.path.display().to_string());

                                        let meta = match item.page_count {
                                            Some(pages) if item.kind == FileKind::Pdf => format!(
                                                "{pages} sayfa  •  {}",
                                                format_bytes(item.size_bytes)
                                            ),
                                            _ => format_bytes(item.size_bytes),
                                        };
                                        ui.label(
                                            RichText::new(meta).size(11.0).color(p.text_muted),
                                        );
                                    },
                                );

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if ui
                                        .add_enabled(
                                            !is_busy,
                                            egui::Button::new(
                                                RichText::new("✕").size(12.0).color(p.danger),
                                            )
                                            .min_size(Vec2::new(26.0, 26.0))
                                            .fill(p.bg_card),
                                        )
                                        .on_hover_text("Listeden çıkar")
                                        .clicked()
                                    {
                                        remove_idx = Some(idx);
                                    }

                                    if ui
                                        .add_enabled(
                                            !is_busy && idx + 1 < total_files,
                                            egui::Button::new(RichText::new("▼").size(11.0))
                                                .min_size(Vec2::new(26.0, 26.0))
                                                .fill(p.bg_card),
                                        )
                                        .on_hover_text("Aşağı taşı")
                                        .clicked()
                                    {
                                        move_down = Some(idx);
                                    }

                                    if ui
                                        .add_enabled(
                                            !is_busy && idx > 0,
                                            egui::Button::new(RichText::new("▲").size(11.0))
                                                .min_size(Vec2::new(26.0, 26.0))
                                                .fill(p.bg_card),
                                        )
                                        .on_hover_text("Yukarı taşı")
                                        .clicked()
                                    {
                                        move_up = Some(idx);
                                    }
                                });
                            });
                        });
                }
            });

        if let Some(i) = move_up {
            self.files.swap(i, i - 1);
        } else if let Some(i) = move_down {
            self.files.swap(i, i + 1);
        } else if let Some(i) = remove_idx {
            self.files.remove(i);
        }
    }

    fn render_settings_panel(&mut self, ui: &mut egui::Ui, p: &ThemePalette, is_busy: bool) {
        ui.label(
            RichText::new("Sıkıştırma & Kalite Ayarları")
                .size(15.0)
                .strong()
                .color(p.text_primary),
        );
        ui.add_space(6.0);

        // Rasterize toggle box
        Frame::new()
            .fill(p.bg_card_alt)
            .stroke(Stroke::new(
                1.0_f32,
                if self.settings.rasterize {
                    p.border_accent
                } else {
                    p.border
                },
            ))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.add_enabled_ui(!is_busy, |ui| {
                    ui.checkbox(
                        &mut self.settings.rasterize,
                        RichText::new("Vektörleri Görsele Çevirerek Sıkıştır")
                            .size(13.0)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.label(
                        RichText::new(
                            "Mimari paftalar (AutoCAD/CAD) ve ağır vektörlü PDF'lerde maksimum küçültme sağlar.",
                        )
                        .size(11.5)
                        .color(p.text_muted),
                    );
                });
            });

        ui.add_space(8.0);

        // DPI Segmented Pills
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Çözünürlük (DPI)")
                    .size(12.5)
                    .strong()
                    .color(p.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(dpi_description(self.settings.dpi))
                        .size(11.5)
                        .color(p.text_muted),
                );
            });
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let dpis = [72u16, 96, 150, 200, 300];
            let btn_w = ((ui.available_width() - 24.0) / dpis.len() as f32).max(46.0);
            for dpi in dpis {
                let selected = self.settings.dpi == dpi;
                let (fill, text_col, stroke) = if selected {
                    (p.primary, Color32::WHITE, Stroke::NONE)
                } else {
                    (p.bg_card_alt, p.text_primary, Stroke::new(1.0_f32, p.border))
                };
                if ui
                    .add_enabled(
                        !is_busy,
                        egui::Button::new(RichText::new(format!("{dpi}")).size(12.5).strong().color(text_col))
                            .min_size(Vec2::new(btn_w, 30.0))
                            .fill(fill)
                            .stroke(stroke),
                    )
                    .clicked()
                {
                    self.settings.dpi = dpi;
                }
            }
        });

        ui.add_space(8.0);

        // Quality Slider
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Görsel Kalitesi")
                    .size(12.5)
                    .strong()
                    .color(p.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                Frame::new()
                    .fill(p.bg_card_alt)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("%{}", self.settings.quality))
                                .size(12.0)
                                .strong()
                                .color(p.primary),
                        );
                    });
            });
        });

        ui.add_enabled_ui(!is_busy, |ui| {
            ui.spacing_mut().slider_width = ui.available_width();
            ui.add(
                egui::Slider::new(&mut self.settings.quality, 10..=100)
                    .show_value(false)
                    .step_by(1.0),
            );
        });
    }

    fn render_actions_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        p: &ThemePalette,
        is_busy: bool,
    ) {
        ui.label(
            RichText::new("İşlem Merkezi")
                .size(15.0)
                .strong()
                .color(p.text_primary),
        );
        ui.add_space(6.0);

        let full_w = ui.available_width();
        let merge_btn = egui::Button::new(
            RichText::new("Birleştir ve PDF Olarak Kaydet")
                .size(14.0)
                .strong()
                .color(Color32::WHITE),
        )
        .min_size(Vec2::new(full_w, 42.0))
        .fill(p.emerald)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(10));

        if ui
            .add_enabled(!is_busy && !self.files.is_empty(), merge_btn)
            .clicked()
        {
            self.start_merge_pdf(ctx);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("PDF sayfalarını görsel olarak dışa aktar:")
                    .size(11.5)
                    .color(p.text_muted),
            );
        });
        ui.add_space(2.0);

        let has_pdf = self.files.iter().any(|f| f.kind == FileKind::Pdf);
        let half_w = ((full_w - 8.0) * 0.5).max(100.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let jpg_btn = egui::Button::new(
                RichText::new("PDF → JPG")
                    .size(13.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .min_size(Vec2::new(half_w, 36.0))
            .fill(p.blue)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(9));

            if ui.add_enabled(!is_busy && has_pdf, jpg_btn).clicked() {
                self.start_export_images(ExportFormat::Jpg, ctx);
            }

            let png_btn = egui::Button::new(
                RichText::new("PDF → PNG")
                    .size(13.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .min_size(Vec2::new(half_w, 36.0))
            .fill(p.blue)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(9));

            if ui.add_enabled(!is_busy && has_pdf, png_btn).clicked() {
                self.start_export_images(ExportFormat::Png, ctx);
            }
        });
    }

    fn render_status_panel(&mut self, ui: &mut egui::Ui, p: &ThemePalette, width: f32) {
        if let Some(job) = &self.running {
            render_card(ui, p, Vec2::new(width, 0.0), |ui| {
                let frac = if job.progress.total > 0 {
                    (job.progress.current as f32 / job.progress.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("İşlem Sürüyor...")
                            .size(13.5)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("İptal").size(11.5).color(p.danger),
                                )
                                .fill(p.danger_bg)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                });
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(frac)
                        .show_percentage()
                        .animate(true)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&job.progress.message)
                        .size(11.5)
                        .color(p.text_muted),
                );
            });
            return;
        }

        if let Some(notice) = &self.ui_notice {
            Frame::new()
                .fill(p.error_bg)
                .stroke(Stroke::new(1.0_f32, p.error_border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_min_width(width - 28.0);
                    ui.label(RichText::new(notice).size(12.5).color(p.error_fg));
                });
            return;
        }

        match &self.last_outcome {
            Some(TaskOutcome::MergedPdf {
                output_path,
                initial_bytes,
                final_bytes,
                total_pages,
            }) => {
                Frame::new()
                    .fill(p.success_bg)
                    .stroke(Stroke::new(1.0_f32, p.success_border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_min_width(width - 28.0);
                        ui.label(
                            RichText::new("✓ PDF Başarıyla Oluşturuldu!")
                                .size(13.5)
                                .strong()
                                .color(p.success_fg),
                        );
                        ui.add_space(4.0);

                        let delta = if *initial_bytes > 0 && final_bytes <= initial_bytes {
                            let pct = ((*initial_bytes - *final_bytes) as f64
                                / *initial_bytes as f64)
                                * 100.0;
                            format!(" (-%{pct:.0})")
                        } else {
                            String::new()
                        };

                        ui.label(
                            RichText::new(format!(
                                "{total_pages} sayfa  •  {} → {}{delta}",
                                format_bytes(*initial_bytes),
                                format_bytes(*final_bytes)
                            ))
                            .size(12.5)
                            .strong()
                            .color(p.success_fg),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(output_path.display().to_string())
                                    .size(11.0)
                                    .color(p.success_fg),
                            )
                            .truncate(),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Klasörü Aç").clicked() {
                                open_in_explorer_select(output_path);
                            }
                            if ui.button("PDF'i Aç").clicked() {
                                open_path_default(output_path);
                            }
                        });
                    });
            }
            Some(TaskOutcome::ExportedImages {
                output_dir,
                total_pages,
                format,
            }) => {
                Frame::new()
                    .fill(p.success_bg)
                    .stroke(Stroke::new(1.0_f32, p.success_border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_min_width(width - 28.0);
                        ui.label(
                            RichText::new(format!(
                                "✓ {total_pages} Sayfa {} Olarak Kaydedildi!",
                                format.label()
                            ))
                            .size(13.5)
                            .strong()
                            .color(p.success_fg),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(output_dir.display().to_string())
                                    .size(11.0)
                                    .color(p.success_fg),
                            )
                            .truncate(),
                        );
                        ui.add_space(6.0);
                        if ui.button("Klasörü Aç").clicked() {
                            open_path_default(output_dir);
                        }
                    });
            }
            Some(TaskOutcome::Cancelled) => {
                render_card(ui, p, Vec2::new(width, 0.0), |ui| {
                    ui.label(
                        RichText::new("İşlem kullanıcı tarafından iptal edildi.")
                            .size(12.5)
                            .color(p.text_muted),
                    );
                });
            }
            Some(TaskOutcome::Error(err)) => {
                Frame::new()
                    .fill(p.error_bg)
                    .stroke(Stroke::new(1.0_f32, p.error_border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_min_width(width - 28.0);
                        ui.label(
                            RichText::new("Hata Oluştu")
                                .size(13.0)
                                .strong()
                                .color(p.error_fg),
                        );
                        ui.label(RichText::new(err).size(12.0).color(p.error_fg));
                    });
            }
            None => {
                render_card(ui, p, Vec2::new(width, 0.0), |ui| {
                    ui.label(
                        RichText::new("Hazır  •  Dosya ekleyip istediğiniz işlemi başlatın.")
                            .size(12.0)
                            .color(p.text_muted),
                    );
                });
            }
        }
    }
}

fn render_card(
    ui: &mut egui::Ui,
    p: &ThemePalette,
    size: Vec2,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let frame = Frame::new()
        .fill(p.bg_card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(16));

    if size.y > 0.0 {
        frame.show(ui, |ui| {
            ui.set_min_size(Vec2::new((size.x - 32.0).max(10.0), (size.y - 32.0).max(10.0)));
            ui.set_max_size(Vec2::new((size.x - 32.0).max(10.0), (size.y - 32.0).max(10.0)));
            add_contents(ui);
        });
    } else {
        frame.show(ui, |ui| {
            ui.set_min_width((size.x - 32.0).max(10.0));
            ui.set_max_width((size.x - 32.0).max(10.0));
            add_contents(ui);
        });
    }
}

fn draw_upload_emblem(painter: &egui::Painter, center: Pos2, accent: Color32) {
    let circle_bg = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
    painter.circle_filled(center, 26.0, circle_bg);
    // Clean up-arrow inside circle
    let stroke = Stroke::new(2.4_f32, accent);
    painter.line_segment(
        [center + Vec2::new(0.0, 9.0), center + Vec2::new(0.0, -9.0)],
        stroke,
    );
    painter.line_segment(
        [center + Vec2::new(-7.0, -2.0), center + Vec2::new(0.0, -9.0)],
        stroke,
    );
    painter.line_segment(
        [center + Vec2::new(7.0, -2.0), center + Vec2::new(0.0, -9.0)],
        stroke,
    );
    painter.line_segment(
        [center + Vec2::new(-9.0, 12.0), center + Vec2::new(9.0, 12.0)],
        stroke,
    );
}

fn dpi_description(dpi: u16) -> &'static str {
    match dpi {
        72 => "Minimum boyut",
        96 => "Web için ideal",
        150 => "Dengeli kalite",
        200 => "Yüksek kalite",
        300 => "Baskı kalitesi",
        _ => "",
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn load_icon_texture(ctx: &egui::Context) -> Option<TextureHandle> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color_img = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("app_icon", color_img, TextureOptions::LINEAR))
}

fn open_in_explorer_select(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(parent) = path.parent() {
            open_path_default(parent);
        }
    }
}

fn open_path_default(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
    }
}
