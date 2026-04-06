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

impl PdfDoc {
    pub fn open(path: &Path) -> Result<Self, PdfiumError> {
        let lib_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib/lib");
        let pdfium = Pdfium::new(
            Pdfium::bind_to_library(
                Pdfium::pdfium_platform_library_name_at_path(lib_path.to_str().unwrap_or("./lib/lib"))
            )
            .or_else(|_| Pdfium::bind_to_system_library())?,
        );

        let doc_bytes = std::fs::read(path)
            .map_err(|e| PdfiumError::IoError(e))?;
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

    pub fn render_page(&self, index: usize, target_width: u32) -> Result<PageRender, PdfiumError> {
        let doc = self.pdfium.load_pdf_from_byte_slice(&self.doc_bytes, None)?;
        let page = doc.pages().get(index as u16)?;

        let text = page
            .text()
            .map(|t| t.all())
            .unwrap_or_default();

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
}
