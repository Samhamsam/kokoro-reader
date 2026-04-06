use pdfium_render::prelude::*;
use std::path::Path;

pub struct PdfDoc {
    pdfium: Pdfium,
    doc_bytes: Vec<u8>,
    page_count: usize,
}

pub struct PageRender {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub text: String,
}

/// A rectangle in image pixel coordinates (top-left origin)
#[derive(Clone, Debug)]
pub struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PdfDoc {
    pub fn open(path: &Path) -> Result<Self, PdfiumError> {
        let lib_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib/lib");
        let pdfium = Pdfium::new(
            Pdfium::bind_to_library(
                Pdfium::pdfium_platform_library_name_at_path(
                    lib_path.to_str().unwrap_or("./lib/lib"),
                ),
            )
            .or_else(|_| Pdfium::bind_to_system_library())?,
        );

        let doc_bytes = std::fs::read(path).map_err(|e| PdfiumError::IoError(e))?;
        let page_count = {
            let doc = pdfium.load_pdf_from_byte_slice(&doc_bytes, None)?;
            doc.pages().len() as usize
        };

        Ok(Self {
            pdfium,
            doc_bytes,
            page_count,
        })
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn render_page(
        &self,
        index: usize,
        target_width: u32,
    ) -> Result<PageRender, PdfiumError> {
        let doc = self.pdfium.load_pdf_from_byte_slice(&self.doc_bytes, None)?;
        let page = doc.pages().get(index as u16)?;

        let text = page.text().map(|t| t.all()).unwrap_or_default();

        let render_config = PdfRenderConfig::new()
            .set_target_width(target_width as Pixels)
            .set_maximum_height((target_width * 2) as i32);

        let bitmap = page.render_with_config(&render_config)?;
        let image = bitmap.as_image();
        let rgba_image = image.to_rgba8();
        let width = rgba_image.width() as usize;
        let height = rgba_image.height() as usize;
        let rgba = rgba_image.into_raw();

        Ok(PageRender {
            rgba,
            width,
            height,
            text,
        })
    }

    /// Find bounding rectangles for a sentence on the page, in image pixel coordinates.
    /// Returns line-level rects (merged per line) for the sentence text.
    pub fn find_sentence_rects(
        &self,
        page_index: usize,
        sentence: &str,
        image_width: usize,
        image_height: usize,
    ) -> Vec<PixelRect> {
        let doc = match self.pdfium.load_pdf_from_byte_slice(&self.doc_bytes, None) {
            Ok(d) => d,
            Err(_) => return vec![],
        };
        let page = match doc.pages().get(page_index as u16) {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        let page_width = page.width().value as f64;
        let page_height = page.height().value as f64;
        let scale_x = image_width as f64 / page_width;
        let scale_y = image_height as f64 / page_height;

        let page_text = match page.text() {
            Ok(t) => t,
            Err(_) => return vec![],
        };

        // Search for the first few words to find where the sentence starts
        let search_prefix: String = sentence.chars().take(40).collect();
        let search_prefix = search_prefix.trim();
        if search_prefix.is_empty() {
            return vec![];
        }

        let search = match page_text.search(search_prefix, &PdfSearchOptions::new()) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let found_segments = match search.find_next() {
            Some(segs) => segs,
            None => return vec![],
        };

        // Get the starting character index from the search result
        let start_char_idx = match found_segments.iter().next() {
            Some(seg) => match seg.chars() {
                Ok(chars) => chars.first_char_index().unwrap_or(0),
                Err(_) => return vec![],
            },
            None => return vec![],
        };

        // Now get rects for the full sentence length using the chars collection
        let all_chars = page_text.chars();

        // Sentence length + padding (PDF text may have slightly different whitespace)
        let sentence_char_count = sentence.chars().count() + 5;
        let end_char_idx = (start_char_idx + sentence_char_count).min(all_chars.len());

        let mut rects = Vec::new();
        for i in start_char_idx..end_char_idx {
            let ch = match all_chars.get(i) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Ok(bounds) = ch.loose_bounds() {
                let x = bounds.left.value as f64 * scale_x;
                let y = (page_height - bounds.top.value as f64) * scale_y;
                let w = (bounds.right.value - bounds.left.value) as f64 * scale_x;
                let h = (bounds.top.value - bounds.bottom.value) as f64 * scale_y;
                if w > 0.5 && h > 0.5 {
                    rects.push(PixelRect {
                        x: x as f32,
                        y: y as f32,
                        w: w as f32,
                        h: h as f32,
                    });
                }
            }
        }

        // Merge character rects into line-level rects (same y ± tolerance)
        merge_rects_by_line(&rects)
    }
}

/// Merge individual character rects into line-level rects
fn merge_rects_by_line(rects: &[PixelRect]) -> Vec<PixelRect> {
    if rects.is_empty() {
        return vec![];
    }

    let mut lines: Vec<PixelRect> = Vec::new();
    let tolerance = rects[0].h * 0.5;

    for r in rects {
        // Find a line rect with similar y position
        if let Some(line) = lines.iter_mut().find(|l| (l.y - r.y).abs() < tolerance) {
            let right = (line.x + line.w).max(r.x + r.w);
            let bottom = (line.y + line.h).max(r.y + r.h);
            line.x = line.x.min(r.x);
            line.y = line.y.min(r.y);
            line.w = right - line.x;
            line.h = bottom - line.y;
        } else {
            lines.push(r.clone());
        }
    }
    lines
}
