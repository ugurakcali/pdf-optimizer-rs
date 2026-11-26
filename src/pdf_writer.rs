use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// A single rasterized JPEG page ready to be embedded directly via `/DCTDecode`.
#[derive(Debug, Clone)]
pub struct JpegPage {
    pub width_pt: f32,
    pub height_pt: f32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub jpeg_bytes: Vec<u8>,
}

/// Builds a valid PDF 1.4 document in memory where each page embeds its JPEG
/// stream directly with `/DCTDecode` (no re-encoding).
pub fn build_jpeg_pdf_bytes(pages: &[JpegPage]) -> Vec<u8> {
    let estimated_bytes: usize = pages
        .iter()
        .map(|p| p.jpeg_bytes.len() + 512)
        .sum::<usize>()
        + 512;
    let mut out = Vec::with_capacity(estimated_bytes);

    // PDF 1.4 binary header
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // Object numbering:
    // 1: Catalog
    // 2: Pages
    // For each page i (0..pages.len()):
    //   base = 3 + i * 3
    //   base + 0: Page dict
    //   base + 1: Image XObject (/DCTDecode)
    //   base + 2: Content stream
    let total_objects = 2 + pages.len() * 3;
    let mut offsets: Vec<usize> = Vec::with_capacity(total_objects + 1);
    offsets.push(0); // object 0 (free head)

    // 1 0 obj: Catalog
    offsets.push(out.len());
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // 2 0 obj: Pages
    offsets.push(out.len());
    let mut kids = String::with_capacity(pages.len() * 10);
    for i in 0..pages.len() {
        if i > 0 {
            kids.push(' ');
        }
        let page_obj = 3 + i * 3;
        kids.push_str(&format!("{page_obj} 0 R"));
    }
    out.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /Pages /Kids [ {kids} ] /Count {} >>\nendobj\n",
            pages.len()
        )
        .as_bytes(),
    );

    // Page objects
    for (i, page) in pages.iter().enumerate() {
        let page_obj = 3 + i * 3;
        let img_obj = page_obj + 1;
        let content_obj = page_obj + 2;

        let w_pt = format!("{:.2}", page.width_pt.max(1.0));
        let h_pt = format!("{:.2}", page.height_pt.max(1.0));

        // Page dictionary
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n\
                 << /Type /Page /Parent 2 0 R /MediaBox [0 0 {w_pt} {h_pt}] \
                 /Resources << /XObject << /Im0 {img_obj} 0 R >> >> \
                 /Contents {content_obj} 0 R >>\n\
                 endobj\n"
            )
            .as_bytes(),
        );

        // Image XObject
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{img_obj} 0 obj\n\
                 << /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 \
                 /Filter /DCTDecode /Length {} >>\n\
                 stream\n",
                page.pixel_width,
                page.pixel_height,
                page.jpeg_bytes.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&page.jpeg_bytes);
        out.extend_from_slice(b"\nendstream\nendobj\n");

        // Content stream
        let content_cmd = format!("q\n{w_pt} 0 0 {h_pt} 0 0 cm\n/Im0 Do\nQ\n");
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{content_obj} 0 obj\n\
                 << /Length {} >>\n\
                 stream\n\
                 {content_cmd}\
                 endstream\n\
                 endobj\n",
                content_cmd.len()
            )
            .as_bytes(),
        );
    }

    // Cross-reference table
    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total_objects + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for &off in &offsets[1..] {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }

    // Trailer
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            total_objects + 1
        )
        .as_bytes(),
    );

    out
}

/// Writes `pages` directly to `path` as a PDF 1.4 file.
pub fn write_jpeg_pdf(path: &Path, pages: &[JpegPage]) -> io::Result<()> {
    let bytes = build_jpeg_pdf_bytes(pages);
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(&bytes)?;
    writer.flush()
}
