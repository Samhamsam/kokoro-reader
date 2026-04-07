use serde_json;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/kokoro-reader")
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config/kokoro-reader")
}

pub fn load_settings() -> Settings {
    let path = config_dir().join("config.json");
    if path.exists() {
        let data = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Settings::default()
    }
}

pub fn save_settings(settings: &Settings) {
    fs::create_dir_all(config_dir()).ok();
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        fs::write(config_dir().join("config.json"), json).ok();
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub data_dir: PathBuf,
    #[serde(default = "default_server_url")]
    pub server_url: String,
}

fn default_server_url() -> String {
    "http://localhost:8787".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            server_url: default_server_url(),
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BookEntry {
    pub id: String,
    pub title: String,
    pub filename: String,
    pub total_pages: usize,
    pub last_page: usize,
    pub last_sentence: usize,
    pub selected_voice: usize,
    #[serde(default)]
    pub selected_voice_id: String,
    #[serde(default)]
    pub last_accessed: u64, // unix timestamp
}

impl BookEntry {
    pub fn progress(&self) -> f32 {
        if self.total_pages == 0 {
            return 0.0;
        }
        self.last_page as f32 / self.total_pages as f32
    }

    pub fn progress_percent(&self) -> u32 {
        (self.progress() * 100.0).round() as u32
    }

    pub fn book_path(&self, data_dir: &Path) -> PathBuf {
        data_dir.join("books").join(&self.filename)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LibraryData {
    books: Vec<BookEntry>,
}

pub struct Library {
    pub books: Vec<BookEntry>,
    pub data_dir: PathBuf,
}

impl Library {
    pub fn load(data_dir: &Path) -> Self {
        let data_dir = data_dir.to_path_buf();
        let books_dir = data_dir.join("books");
        let library_path = data_dir.join("library.json");
        fs::create_dir_all(&books_dir).ok();

        let books = if library_path.exists() {
            let data = fs::read_to_string(&library_path).unwrap_or_default();
            serde_json::from_str::<LibraryData>(&data)
                .map(|d| d.books)
                .unwrap_or_default()
        } else {
            vec![]
        };

        let books = books
            .into_iter()
            .filter(|b| b.book_path(&data_dir).exists())
            .collect();

        Self { books, data_dir }
    }

    pub fn save(&self) {
        fs::create_dir_all(&self.data_dir).ok();
        let data = LibraryData {
            books: self.books.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            fs::write(self.data_dir.join("library.json"), json).ok();
        }
    }

    pub fn import(&mut self, source_path: &Path) -> Result<String, String> {
        let filename = source_path
            .file_name()
            .ok_or("Invalid filename")?
            .to_string_lossy()
            .to_string();

        if self.books.iter().any(|b| b.filename == filename) {
            return Ok(self.books.iter().find(|b| b.filename == filename).unwrap().id.clone());
        }

        let dest = self.data_dir.join("books").join(&filename);
        fs::create_dir_all(self.data_dir.join("books")).ok();
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
            last_sentence: 0,
            selected_voice: 0,
            selected_voice_id: String::new(),
            last_accessed: now(),
        };

        self.books.push(entry);
        self.save();
        Ok(id)
    }

    pub fn delete(&mut self, id: &str) {
        if let Some(pos) = self.books.iter().position(|b| b.id == id) {
            let book = self.books.remove(pos);
            fs::remove_file(book.book_path(&self.data_dir)).ok();
            self.save();
        }
    }

    pub fn update_progress(&mut self, id: &str, page: usize, sentence: usize, voice: usize, voice_id: &str) {
        if let Some(book) = self.books.iter_mut().find(|b| b.id == id) {
            book.last_page = page;
            book.last_sentence = sentence;
            book.selected_voice = voice;
            book.selected_voice_id = voice_id.to_string();
            book.last_accessed = now();
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
