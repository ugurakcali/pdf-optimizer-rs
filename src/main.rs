#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod pdf_engine;
mod pdf_writer;
mod theme;

use app::PdfAraciApp;
use eframe::egui::{IconData, ViewportBuilder};
use std::sync::Arc;

fn load_window_icon() -> Arc<IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes)
        .expect("embedded icon.png must decode")
        .to_rgba8();
    let (width, height) = img.dimensions();
    Arc::new(IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("PDF Optimize & Dönüştürme Aracı")
            .with_inner_size([920.0, 660.0])
            .with_min_inner_size([760.0, 560.0])
            .with_icon(load_window_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "PDF Optimize & Dönüştürme Aracı",
        options,
        Box::new(|cc| Ok(Box::new(PdfAraciApp::new(cc)))),
    )
}
