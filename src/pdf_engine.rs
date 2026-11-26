use crate::pdf_writer::{build_jpeg_pdf_bytes, write_jpeg_pdf, JpegPage};
use image::{DynamicImage, ImageFormat, RgbImage};
use jpeg_encoder::{ColorType, Encoder};
use pdfium_render::prelude::*;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

static PDFIUM_SINGLETON: OnceLock<Result<Mutex<Pdfium>, String>> = OnceLock::new();

const MAX_RENDER_PIXEL_DIM: f32 = 12_000.0;
const RENDER_BATCH_SIZE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Png,
    Jpeg,
}

impl FileKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => Some(Self::Pdf),
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Png => "PNG",
            Self::Jpeg => "JPG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InputFile {
    pub path: PathBuf,
    pub name: String,
    pub size_bytes: u64,
    pub kind: FileKind,
    pub page_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessSettings {
    pub rasterize: bool,
    pub quality: u8,
    pub dpi: u16,
}

impl Default for ProcessSettings {
    fn default() -> Self {
        Self {
            rasterize: true,
            quality: 75,
            dpi: 150,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Jpg,
    Png,
}

impl ExportFormat {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Png => "png",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jpg => "JPG",
            Self::Png => "PNG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub current: usize,
    pub total: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum TaskOutcome {
    MergedPdf {
        output_path: PathBuf,
        initial_bytes: u64,
        final_bytes: u64,
        total_pages: usize,
    },
    ExportedImages {
        output_dir: PathBuf,
        total_pages: usize,
        format: ExportFormat,
    },
    Cancelled,
    Error(String),
}

fn bind_pdfium_once() -> Result<Pdfium, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if !candidates.contains(&cwd) {
            candidates.push(cwd);
        }
    }

    for dir in &candidates {
        let lib_path = Pdfium::pdfium_platform_library_name_at_path(dir);
        if let Ok(bindings) = Pdfium::bind_to_library(&lib_path) {
            return Ok(Pdfium::new(bindings));
        }
    }

    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| {
            format!(
                "pdfium.dll bulunamadı veya yüklenemedi. Lütfen pdfium.dll dosyasının uygulama ile aynı klasörde olduğundan emin olun. ({e})"
            )
        })
}

pub fn lock_pdfium() -> Result<MutexGuard<'static, Pdfium>, String> {
    let res = PDFIUM_SINGLETON.get_or_init(|| bind_pdfium_once().map(Mutex::new));
    match res {
        Ok(mutex) => Ok(mutex.lock().unwrap_or_else(|poison| poison.into_inner())),
        Err(e) => Err(e.clone()),
    }
}

pub fn inspect_input_files(paths: &[PathBuf]) -> Vec<InputFile> {
    let pdfium = lock_pdfium().ok();
    paths
        .iter()
        .filter_map(|path| inspect_single_file(path, pdfium.as_deref()))
        .collect()
}

fn inspect_single_file(path: &Path, pdfium: Option<&Pdfium>) -> Option<InputFile> {
    let kind = FileKind::from_path(path)?;
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let page_count = match kind {
        FileKind::Pdf => pdfium
            .and_then(|p| p.load_pdf_from_file(path, None).ok())
            .map(|doc| doc.pages().len() as usize),
        FileKind::Png | FileKind::Jpeg => Some(1),
    };

    Some(InputFile {
        path: path.to_path_buf(),
        name,
        size_bytes: meta.len(),
        kind,
        page_count,
    })
}

fn bitmap_to_rgb(bitmap: &PdfBitmap<'_>) -> Result<(u32, u32, Vec<u8>), String> {
    let width = bitmap.width().max(0) as u32;
    let height = bitmap.height().max(0) as u32;
    if width == 0 || height == 0 {
        return Err("Render edilen sayfa boş boyut döndürdü.".to_string());
    }

    let format = bitmap
        .format()
        .map_err(|e| format!("Bitmap formatı okunamadı: {e}"))?;
    let pixel_count = (width as usize) * (height as usize);
    let mut rgb = Vec::with_capacity(pixel_count * 3);

    if format == PdfBitmapFormat::Gray {
        let raw = bitmap.as_raw_bytes();
        let stride = (raw.len() / (height as usize)).max(width as usize);
        for row in 0..(height as usize) {
            let start = row * stride;
            for &y in &raw[start..start + (width as usize)] {
                rgb.push(y);
                rgb.push(y);
                rgb.push(y);
            }
        }
    } else {
        // `as_rgba_bytes()` handles `PdfRenderConfig`'s byte-order reversal and row stride.
        let rgba = bitmap.as_rgba_bytes();
        for px in rgba.chunks_exact(4).take(pixel_count) {
            rgb.push(px[0]);
            rgb.push(px[1]);
            rgb.push(px[2]);
        }
    }

    if rgb.len() != pixel_count * 3 {
        return Err("Bitmap piksel boyutu beklenenden farklı.".to_string());
    }

    Ok((width, height, rgb))
}

fn encode_rgb_to_jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let w = u16::try_from(width).map_err(|_| "Görsel genişliği JPEG sınırını aşıyor.".to_string())?;
    let h = u16::try_from(height).map_err(|_| "Görsel yüksekliği JPEG sınırını aşıyor.".to_string())?;
    let mut out = Vec::with_capacity((width as usize * height as usize) / 6);
    let encoder = Encoder::new(&mut out, quality.clamp(10, 100));
    encoder
        .encode(rgb, w, h, ColorType::Rgb)
        .map_err(|e| format!("JPEG kodlama hatası: {e}"))?;
    Ok(out)
}

fn encode_rgb_to_png(rgb: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let img = RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| "PNG görsel tamponu oluşturulamadı.".to_string())?;
    let mut cursor = std::io::Cursor::new(Vec::with_capacity((width as usize * height as usize) / 4));
    DynamicImage::ImageRgb8(img)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("PNG kodlama hatası: {e}"))?;
    Ok(cursor.into_inner())
}

/// Decodes an image file and composites alpha against pure white so PNG transparency
/// never becomes black when converted to RGB/JPEG.
fn load_image_as_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let dyn_img = image::open(path)
        .map_err(|e| format!("Görsel açılamadı ({}): {e}", path.display()))?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);

    for px in rgba.pixels() {
        let a = px[3] as u16;
        if a == 255 {
            rgb.push(px[0]);
            rgb.push(px[1]);
            rgb.push(px[2]);
        } else if a == 0 {
            rgb.push(255);
            rgb.push(255);
            rgb.push(255);
        } else {
            let inv = 255 - a;
            rgb.push(((px[0] as u16 * a + 255 * inv) / 255) as u8);
            rgb.push(((px[1] as u16 * a + 255 * inv) / 255) as u8);
            rgb.push(((px[2] as u16 * a + 255 * inv) / 255) as u8);
        }
    }

    Ok((w, h, rgb))
}

fn render_page_to_rgb(page: &PdfPage<'_>, dpi: u16) -> Result<(f32, f32, u32, u32, Vec<u8>), String> {
    let width_pt = page.width().value.max(1.0);
    let height_pt = page.height().value.max(1.0);
    let scale = (dpi as f32) / 72.0;

    let mut target_w = (width_pt * scale).round().max(1.0);
    let mut target_h = (height_pt * scale).round().max(1.0);
    let max_dim = target_w.max(target_h);
    if max_dim > MAX_RENDER_PIXEL_DIM {
        let clamp_ratio = MAX_RENDER_PIXEL_DIM / max_dim;
        target_w = (target_w * clamp_ratio).round().max(1.0);
        target_h = (target_h * clamp_ratio).round().max(1.0);
    }

    let config = PdfRenderConfig::new()
        .set_target_width(target_w as Pixels)
        .set_target_height(target_h as Pixels)
        .render_form_data(true)
        .set_clear_color(PdfColor::WHITE);

    let bitmap = page
        .render_with_config(&config)
        .map_err(|e| format!("Sayfa render hatası: {e}"))?;
    let (pw, ph, rgb) = bitmap_to_rgb(&bitmap)?;
    Ok((width_pt, height_pt, pw, ph, rgb))
}

fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

pub fn merge_and_save_pdf<F>(
    files: &[InputFile],
    output_path: &Path,
    settings: ProcessSettings,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> TaskOutcome
where
    F: FnMut(ProgressUpdate),
{
    if files.is_empty() {
        return TaskOutcome::Error("Lütfen en az bir dosya ekleyin.".to_string());
    }

    let pdfium = match lock_pdfium() {
        Ok(p) => p,
        Err(e) => return TaskOutcome::Error(e),
    };

    let initial_bytes: u64 = files.iter().map(|f| f.size_bytes).sum();

    // Compute exact total units for smooth progress bar
    let mut total_units = 0usize;
    for f in files {
        match f.kind {
            FileKind::Pdf => {
                if let Some(cnt) = f.page_count {
                    total_units += cnt.max(1);
                } else {
                    match pdfium.load_pdf_from_file(&f.path, None) {
                        Ok(doc) => total_units += (doc.pages().len() as usize).max(1),
                        Err(e) => {
                            return TaskOutcome::Error(format!(
                                "'{}' dosyası açılamadı: {e}",
                                f.name
                            ))
                        }
                    }
                }
            }
            FileKind::Png | FileKind::Jpeg => total_units += 1,
        }
    }
    let total_units = total_units.max(1);

    if settings.rasterize {
        let mut jpeg_pages: Vec<JpegPage> = Vec::with_capacity(total_units);
        let mut completed = 0usize;

        struct RawRenderedPage {
            width_pt: f32,
            height_pt: f32,
            pw: u32,
            ph: u32,
            rgb: Vec<u8>,
        }

        for file in files {
            if is_cancelled(&cancel) {
                return TaskOutcome::Cancelled;
            }

            match file.kind {
                FileKind::Pdf => {
                    let doc = match pdfium.load_pdf_from_file(&file.path, None) {
                        Ok(d) => d,
                        Err(e) => {
                            return TaskOutcome::Error(format!(
                                "'{}' yüklenemedi: {e}",
                                file.name
                            ))
                        }
                    };

                    let pages = doc.pages();
                    let page_count = pages.len();
                    let mut idx: PdfPageIndex = 0;

                    while idx < page_count {
                        if is_cancelled(&cancel) {
                            return TaskOutcome::Cancelled;
                        }

                        let mut batch: Vec<RawRenderedPage> = Vec::with_capacity(RENDER_BATCH_SIZE);
                        for _ in 0..RENDER_BATCH_SIZE {
                            if idx >= page_count || is_cancelled(&cancel) {
                                break;
                            }
                            let page = match pages.get(idx) {
                                Ok(p) => p,
                                Err(e) => {
                                    return TaskOutcome::Error(format!(
                                        "'{}' sayfa {} okunamadı: {e}",
                                        file.name,
                                        idx + 1
                                    ))
                                }
                            };
                            on_progress(ProgressUpdate {
                                current: completed,
                                total: total_units,
                                message: format!(
                                    "Render ediliyor: {} (Sayfa {}/{})",
                                    file.name,
                                    idx + 1,
                                    page_count
                                ),
                            });
                            match render_page_to_rgb(&page, settings.dpi) {
                                Ok((width_pt, height_pt, pw, ph, rgb)) => {
                                    batch.push(RawRenderedPage {
                                        width_pt,
                                        height_pt,
                                        pw,
                                        ph,
                                        rgb,
                                    });
                                }
                                Err(e) => {
                                    return TaskOutcome::Error(format!(
                                        "'{}' sayfa {} işlenemedi: {e}",
                                        file.name,
                                        idx + 1
                                    ))
                                }
                            }
                            idx += 1;
                            completed += 1;
                        }

                        if is_cancelled(&cancel) {
                            return TaskOutcome::Cancelled;
                        }

                        let encoded_batch: Result<Vec<JpegPage>, String> = batch
                            .into_par_iter()
                            .map(|raw| {
                                let jpeg_bytes = encode_rgb_to_jpeg(
                                    &raw.rgb,
                                    raw.pw,
                                    raw.ph,
                                    settings.quality,
                                )?;
                                Ok(JpegPage {
                                    width_pt: raw.width_pt,
                                    height_pt: raw.height_pt,
                                    pixel_width: raw.pw,
                                    pixel_height: raw.ph,
                                    jpeg_bytes,
                                })
                            })
                            .collect();

                        match encoded_batch {
                            Ok(mut pages_out) => jpeg_pages.append(&mut pages_out),
                            Err(e) => return TaskOutcome::Error(e),
                        }
                    }
                }
                FileKind::Png | FileKind::Jpeg => {
                    on_progress(ProgressUpdate {
                        current: completed,
                        total: total_units,
                        message: format!("Görsel işleniyor: {}", file.name),
                    });
                    let (w, h, rgb) = match load_image_as_rgb(&file.path) {
                        Ok(v) => v,
                        Err(e) => return TaskOutcome::Error(e),
                    };
                    let jpeg_bytes = match encode_rgb_to_jpeg(&rgb, w, h, settings.quality) {
                        Ok(b) => b,
                        Err(e) => return TaskOutcome::Error(e),
                    };
                    // Convert pixel dimensions at standard 96 DPI into PDF points (72 pt/inch)
                    let width_pt = (w as f32) * 72.0 / 96.0;
                    let height_pt = (h as f32) * 72.0 / 96.0;
                    jpeg_pages.push(JpegPage {
                        width_pt,
                        height_pt,
                        pixel_width: w,
                        pixel_height: h,
                        jpeg_bytes,
                    });
                    completed += 1;
                }
            }
        }

        if is_cancelled(&cancel) {
            return TaskOutcome::Cancelled;
        }

        on_progress(ProgressUpdate {
            current: total_units,
            total: total_units,
            message: "PDF dosyası kaydediliyor...".to_string(),
        });

        if let Err(e) = write_jpeg_pdf(output_path, &jpeg_pages) {
            return TaskOutcome::Error(format!("PDF yazılamadı: {e}"));
        }

        let final_bytes = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
        TaskOutcome::MergedPdf {
            output_path: output_path.to_path_buf(),
            initial_bytes,
            final_bytes,
            total_pages: jpeg_pages.len(),
        }
    } else {
        let mut dest_doc = match pdfium.create_new_pdf() {
            Ok(d) => d,
            Err(e) => return TaskOutcome::Error(format!("Yeni PDF oluşturulamadı: {e}")),
        };

        let mut completed = 0usize;
        let mut total_pages = 0usize;

        for file in files {
            if is_cancelled(&cancel) {
                return TaskOutcome::Cancelled;
            }

            match file.kind {
                FileKind::Pdf => {
                    let src_doc = match pdfium.load_pdf_from_file(&file.path, None) {
                        Ok(d) => d,
                        Err(e) => {
                            return TaskOutcome::Error(format!(
                                "'{}' yüklenemedi: {e}",
                                file.name
                            ))
                        }
                    };
                    let src_len = src_doc.pages().len();
                    for i in 0..src_len {
                        if is_cancelled(&cancel) {
                            return TaskOutcome::Cancelled;
                        }
                        on_progress(ProgressUpdate {
                            current: completed,
                            total: total_units,
                            message: format!(
                                "Kayıpsız birleştiriliyor: {} (Sayfa {}/{})",
                                file.name,
                                i + 1,
                                src_len
                            ),
                        });
                        let dest_idx = dest_doc.pages().len();
                        if let Err(e) = dest_doc
                            .pages_mut()
                            .copy_page_from_document(&src_doc, i, dest_idx)
                        {
                            return TaskOutcome::Error(format!(
                                "'{}' sayfa {} kopyalanamadı: {e}",
                                file.name,
                                i + 1
                            ));
                        }
                        completed += 1;
                        total_pages += 1;
                    }
                }
                FileKind::Png | FileKind::Jpeg => {
                    on_progress(ProgressUpdate {
                        current: completed,
                        total: total_units,
                        message: format!("Görsel PDF'e ekleniyor: {}", file.name),
                    });

                    let (w, h, rgb) = match load_image_as_rgb(&file.path) {
                        Ok(v) => v,
                        Err(e) => return TaskOutcome::Error(e),
                    };
                    let jpeg_bytes =
                        match encode_rgb_to_jpeg(&rgb, w, h, settings.quality.max(90)) {
                            Ok(b) => b,
                            Err(e) => return TaskOutcome::Error(e),
                        };
                    let single_pdf = build_jpeg_pdf_bytes(&[JpegPage {
                        width_pt: (w as f32) * 72.0 / 96.0,
                        height_pt: (h as f32) * 72.0 / 96.0,
                        pixel_width: w,
                        pixel_height: h,
                        jpeg_bytes,
                    }]);
                    let img_doc = match pdfium.load_pdf_from_byte_slice(&single_pdf, None) {
                        Ok(d) => d,
                        Err(e) => {
                            return TaskOutcome::Error(format!(
                                "Görsel PDF sayfasına dönüştürülemedi: {e}"
                            ))
                        }
                    };
                    let dest_idx = dest_doc.pages().len();
                    if let Err(e) = dest_doc
                        .pages_mut()
                        .copy_page_from_document(&img_doc, 0, dest_idx)
                    {
                        return TaskOutcome::Error(format!("Görsel sayfası eklenemedi: {e}"));
                    }
                    completed += 1;
                    total_pages += 1;
                }
            }
        }

        if is_cancelled(&cancel) {
            return TaskOutcome::Cancelled;
        }

        on_progress(ProgressUpdate {
            current: total_units,
            total: total_units,
            message: "PDF dosyası kaydediliyor...".to_string(),
        });

        if let Err(e) = dest_doc.save_to_file(output_path) {
            return TaskOutcome::Error(format!("PDF kaydedilemedi: {e}"));
        }

        let final_bytes = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
        TaskOutcome::MergedPdf {
            output_path: output_path.to_path_buf(),
            initial_bytes,
            final_bytes,
            total_pages,
        }
    }
}

pub fn export_pdfs_to_images<F>(
    files: &[InputFile],
    output_dir: &Path,
    format: ExportFormat,
    settings: ProcessSettings,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> TaskOutcome
where
    F: FnMut(ProgressUpdate),
{
    let pdf_files: Vec<&InputFile> = files
        .iter()
        .filter(|f| f.kind == FileKind::Pdf)
        .collect();

    if pdf_files.is_empty() {
        return TaskOutcome::Error("Listede dönüştürülecek PDF dosyası bulunamadı.".to_string());
    }

    let pdfium = match lock_pdfium() {
        Ok(p) => p,
        Err(e) => return TaskOutcome::Error(e),
    };

    let mut total_pages = 0usize;
    for f in &pdf_files {
        if let Some(cnt) = f.page_count {
            total_pages += cnt.max(1);
        } else {
            match pdfium.load_pdf_from_file(&f.path, None) {
                Ok(doc) => total_pages += (doc.pages().len() as usize).max(1),
                Err(e) => {
                    return TaskOutcome::Error(format!("'{}' dosyası açılamadı: {e}", f.name))
                }
            }
        }
    }
    let total_pages = total_pages.max(1);

    struct ExportTask {
        out_path: PathBuf,
        pw: u32,
        ph: u32,
        rgb: Vec<u8>,
    }

    let mut used_stems: HashMap<String, usize> = HashMap::new();
    let mut completed = 0usize;
    let mut exported_count = 0usize;

    for file in pdf_files {
        if is_cancelled(&cancel) {
            return TaskOutcome::Cancelled;
        }

        let base_stem = file
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "belge".to_string());

        let count = used_stems.entry(base_stem.clone()).or_insert(0);
        *count += 1;
        let unique_stem = if *count == 1 {
            base_stem
        } else {
            format!("{base_stem}_{}", *count)
        };

        let doc = match pdfium.load_pdf_from_file(&file.path, None) {
            Ok(d) => d,
            Err(e) => {
                return TaskOutcome::Error(format!("'{}' yüklenemedi: {e}", file.name))
            }
        };

        let pages = doc.pages();
        let page_count = pages.len();
        let mut idx: PdfPageIndex = 0;

        while idx < page_count {
            if is_cancelled(&cancel) {
                return TaskOutcome::Cancelled;
            }

            let mut batch: Vec<ExportTask> = Vec::with_capacity(RENDER_BATCH_SIZE);
            for _ in 0..RENDER_BATCH_SIZE {
                if idx >= page_count || is_cancelled(&cancel) {
                    break;
                }
                let page = match pages.get(idx) {
                    Ok(p) => p,
                    Err(e) => {
                        return TaskOutcome::Error(format!(
                            "'{}' sayfa {} okunamadı: {e}",
                            file.name,
                            idx + 1
                        ))
                    }
                };
                on_progress(ProgressUpdate {
                    current: completed,
                    total: total_pages,
                    message: format!(
                        "{} dönüştürülüyor: {} (Sayfa {}/{})",
                        format.label(),
                        file.name,
                        idx + 1,
                        page_count
                    ),
                });

                let (_, _, pw, ph, rgb) = match render_page_to_rgb(&page, settings.dpi) {
                    Ok(v) => v,
                    Err(e) => {
                        return TaskOutcome::Error(format!(
                            "'{}' sayfa {} render edilemedi: {e}",
                            file.name,
                            idx + 1
                        ))
                    }
                };

                let out_name = format!("{unique_stem}_sayfa_{}.{}", idx + 1, format.ext());
                batch.push(ExportTask {
                    out_path: output_dir.join(out_name),
                    pw,
                    ph,
                    rgb,
                });

                idx += 1;
                completed += 1;
            }

            if is_cancelled(&cancel) {
                return TaskOutcome::Cancelled;
            }

            let res: Result<usize, String> = batch
                .into_par_iter()
                .map(|task| {
                    let bytes = match format {
                        ExportFormat::Jpg => {
                            encode_rgb_to_jpeg(&task.rgb, task.pw, task.ph, settings.quality)?
                        }
                        ExportFormat::Png => encode_rgb_to_png(task.rgb, task.pw, task.ph)?,
                    };
                    fs::write(&task.out_path, bytes).map_err(|e| {
                        format!("Dosya yazılamadı ({}): {e}", task.out_path.display())
                    })?;
                    Ok(1usize)
                })
                .collect::<Result<Vec<usize>, String>>()
                .map(|v| v.len());

            match res {
                Ok(n) => exported_count += n,
                Err(e) => return TaskOutcome::Error(e),
            }
        }
    }

    on_progress(ProgressUpdate {
        current: total_pages,
        total: total_pages,
        message: "Tamamlandı!".to_string(),
    });

    TaskOutcome::ExportedImages {
        output_dir: output_dir.to_path_buf(),
        total_pages: exported_count,
        format,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdf_araci_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    fn create_solid_red_png(path: &Path, w: u32, h: u32) {
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = image::Rgb([230, 25, 25]);
        }
        img.save(path).expect("save red png");
    }

    fn create_sample_pdf(path: &Path, pages: usize) {
        let mut jpeg_pages = Vec::with_capacity(pages);
        for idx in 0..pages {
            let mut img = RgbImage::new(240, 320);
            let shade = ((idx * 40) % 200) as u8;
            for px in img.pixels_mut() {
                *px = image::Rgb([30 + shade / 2, 120, 210 - shade / 2]);
            }
            let jpeg_bytes = encode_rgb_to_jpeg(img.as_raw(), 240, 320, 90).expect("encode jpeg");
            jpeg_pages.push(JpegPage {
                width_pt: 240.0,
                height_pt: 320.0,
                pixel_width: 240,
                pixel_height: 320,
                jpeg_bytes,
            });
        }
        write_jpeg_pdf(path, &jpeg_pages).expect("write sample pdf");
    }

    #[test]
    fn test_rasterize_merge_export_and_color_fidelity() {
        let work = temp_dir("full_pipeline");
        let sample_pdf = work.join("sample.pdf");
        create_sample_pdf(&sample_pdf, 2);
        let red_png = work.join("red.png");
        create_solid_red_png(&red_png, 200, 120);

        let inputs = inspect_input_files(&[sample_pdf.clone(), red_png.clone()]);
        assert_eq!(inputs.len(), 2);
        let orig_pages = inputs[0].page_count.expect("pdf page count");
        assert_eq!(orig_pages, 2);

        // 1. Rasterize ON merge
        let merged_raster = work.join("merged_raster.pdf");
        let res = merge_and_save_pdf(
            &inputs,
            &merged_raster,
            ProcessSettings {
                rasterize: true,
                quality: 75,
                dpi: 96,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        );
        match res {
            TaskOutcome::MergedPdf {
                total_pages,
                final_bytes,
                ..
            } => {
                assert_eq!(total_pages, orig_pages + 1);
                assert!(final_bytes > 1000);
            }
            other => panic!("Expected MergedPdf, got {other:?}"),
        }

        // Verify red PNG page rendered back is genuinely RED (R >> B), not swapped BGR!
        let rgb = {
            let pdfium = lock_pdfium().expect("pdfium");
            let check_doc = pdfium
                .load_pdf_from_file(&merged_raster, None)
                .expect("open merged");
            assert_eq!(check_doc.pages().len() as usize, orig_pages + 1);
            let last_page = check_doc.pages().get(orig_pages as u16).expect("last page");
            let (_, _, _, _, rgb) = render_page_to_rgb(&last_page, 72).expect("render red page");
            rgb
        };
        let center_idx = (rgb.len() / 6) * 3;
        let (r, g, b) = (rgb[center_idx], rgb[center_idx + 1], rgb[center_idx + 2]);
        assert!(r > 180 && g < 80 && b < 80, "Color channel mismatch: RGB=({r},{g},{b})");

        // 2. Rasterize OFF lossless merge
        let merged_lossless = work.join("merged_lossless.pdf");
        let res2 = merge_and_save_pdf(
            &inputs,
            &merged_lossless,
            ProcessSettings {
                rasterize: false,
                quality: 80,
                dpi: 150,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        );
        match res2 {
            TaskOutcome::MergedPdf { total_pages, .. } => {
                assert_eq!(total_pages, orig_pages + 1);
            }
            other => panic!("Expected MergedPdf lossless, got {other:?}"),
        }

        // 3. Export PDF -> JPG & PNG (including duplicate stem handling across folders)
        let sub_dir = work.join("sub");
        fs::create_dir_all(&sub_dir).unwrap();
        let dup_pdf = sub_dir.join("sample.pdf");
        create_sample_pdf(&dup_pdf, 1);
        let dup_inputs = inspect_input_files(&[sample_pdf, dup_pdf]);
        assert_eq!(dup_inputs.len(), 2);

        let jpg_dir = work.join("out_jpg");
        fs::create_dir_all(&jpg_dir).unwrap();
        let res_jpg = export_pdfs_to_images(
            &dup_inputs,
            &jpg_dir,
            ExportFormat::Jpg,
            ProcessSettings {
                rasterize: true,
                quality: 80,
                dpi: 96,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        );
        match res_jpg {
            TaskOutcome::ExportedImages { total_pages, .. } => {
                assert_eq!(total_pages, 3);
                assert!(jpg_dir.join("sample_sayfa_1.jpg").is_file());
                assert!(jpg_dir.join("sample_sayfa_2.jpg").is_file());
                assert!(jpg_dir.join("sample_2_sayfa_1.jpg").is_file());
            }
            other => panic!("Expected ExportedImages JPG, got {other:?}"),
        }

        let png_dir = work.join("out_png");
        fs::create_dir_all(&png_dir).unwrap();
        let res_png = export_pdfs_to_images(
            &inputs,
            &png_dir,
            ExportFormat::Png,
            ProcessSettings {
                rasterize: true,
                quality: 80,
                dpi: 72,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        );
        match res_png {
            TaskOutcome::ExportedImages { total_pages, .. } => {
                assert_eq!(total_pages, orig_pages);
                assert!(png_dir.join("sample_sayfa_1.png").is_file());
                assert!(png_dir.join("sample_sayfa_2.png").is_file());
            }
            other => panic!("Expected ExportedImages PNG, got {other:?}"),
        }

        // 4. Cancellation check
        let cancel_flag = Arc::new(AtomicBool::new(true));
        let res_cancel = merge_and_save_pdf(
            &inputs,
            &work.join("cancelled.pdf"),
            ProcessSettings::default(),
            cancel_flag,
            |_| {},
        );
        assert!(matches!(res_cancel, TaskOutcome::Cancelled));

        let _ = fs::remove_dir_all(&work);
    }
}
