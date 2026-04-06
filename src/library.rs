use serde_json;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

fn data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/kokoro-reader")
}

fn books_dir() -> PathBuf {
    data_dir().join("books")
}

fn library_path() -> PathBuf {
    data_dir().join("library.json")
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BookEntry {
    pub id: String,
    pub title: String,
    pub filename: String,
    pub total_pages: usize,
    pub last_page: usize,
    pub selected_voice: usize,
}

impl BookEntry {
    pub fn progress(&self) -> f32 {
        if self.total_pages == 0 {
            return 0.0;
        }
        (self.last_page + 1) as f32 / self.total_pages as f32
    }

    pub fn progress_percent(&self) -> u32 {
        (self.progress() * 100.0) as u32
    }

    pub fn book_path(&self) -> PathBuf {
        books_dir().join(&self.filename)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LibraryData {
    books: Vec<BookEntry>,
}

pub struct Library {
    pub books: Vec<BookEntry>,
}

impl Library {
    pub fn load() -> Self {
        fs::create_dir_all(books_dir()).ok();

        let books = if library_path().exists() {
            let data = fs::read_to_string(library_path()).unwrap_or_default();
            serde_json::from_str::<LibraryData>(&data)
                .map(|d| d.books)
                .unwrap_or_default()
        } else {
            vec![]
        };

        // Filter out books whose files no longer exist
        let books = books
            .into_iter()
            .filter(|b| b.book_path().exists())
            .collect();

        Self { books }
    }

    pub fn save(&self) {
        fs::create_dir_all(data_dir()).ok();
        let data = LibraryData {
            books: self.books.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            fs::write(library_path(), json).ok();
        }
    }

    pub fn import(&mut self, source_path: &Path) -> Result<String, String> {
        let filename = source_path
            .file_name()
            .ok_or("Invalid filename")?
            .to_string_lossy()
            .to_string();

        // Check if already imported
        if self.books.iter().any(|b| b.filename == filename) {
            // Already exists — just return its id
            return Ok(self.books.iter().find(|b| b.filename == filename).unwrap().id.clone());
        }

        // Copy file to books dir
        let dest = books_dir().join(&filename);
        fs::copy(source_path, &dest)
            .map_err(|e| format!("Failed to copy: {}", e))?;

        // Get page count
        let total_pages = get_page_count(&dest).unwrap_or(0);

        // Generate ID
        let mut hasher = DefaultHasher::new();
        filename.hash(&mut hasher);
        let id = format!("{:x}", hasher.finish());

        // Extract title from filename (remove extension)
        let title = source_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let entry = BookEntry {
            id: id.clone(),
            title,
            filename,
            total_pages,
            last_page: 0,
            selected_voice: 0,
        };

        self.books.push(entry);
        self.save();
        Ok(id)
    }

    pub fn delete(&mut self, id: &str) {
        if let Some(pos) = self.books.iter().position(|b| b.id == id) {
            let book = self.books.remove(pos);
            fs::remove_file(book.book_path()).ok();
            self.save();
        }
    }

    pub fn update_progress(&mut self, id: &str, page: usize, voice: usize) {
        if let Some(book) = self.books.iter_mut().find(|b| b.id == id) {
            book.last_page = page;
            book.selected_voice = voice;
            self.save();
        }
    }

    pub fn get(&self, id: &str) -> Option<&BookEntry> {
        self.books.iter().find(|b| b.id == id)
    }
}

fn get_page_count(path: &Path) -> Result<usize, String> {
    use pdfium_render::prelude::*;
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib/lib");
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(
            Pdfium::pdfium_platform_library_name_at_path(lib_path.to_str().unwrap_or("./lib/lib")),
        )
        .or_else(|_| Pdfium::bind_to_system_library())
        .map_err(|e| format!("{}", e))?,
    );
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("{}", e))?;
    Ok(doc.pages().len() as usize)
}
