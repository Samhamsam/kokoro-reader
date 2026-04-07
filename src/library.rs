use serde_json;
use std::fs;
use std::path::{Path, PathBuf};

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config/kokoro-reader")
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".cache/kokoro-reader")
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default = "default_server_url")]
    pub server_url: String,
}

fn default_server_url() -> String { "http://localhost:8787".to_string() }

impl Default for Settings {
    fn default() -> Self { Self { server_url: default_server_url() } }
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

// ── Book Entry (matches server JSON) ──

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BookEntry {
    pub id: String,
    pub title: String,
    pub total_pages: usize,
    pub last_page: usize,
    pub last_sentence: usize,
    pub selected_voice_id: String,
    pub last_accessed: u64,
}

impl BookEntry {
    pub fn progress(&self) -> f32 {
        if self.total_pages == 0 { return 0.0; }
        self.last_page as f32 / self.total_pages as f32
    }

    pub fn progress_percent(&self) -> u32 {
        (self.progress() * 100.0).round() as u32
    }
}

// ── Library (HTTP-backed) ──

pub struct Library {
    pub server_url: String,
    pub books: Vec<BookEntry>,
    cache_dir: PathBuf,
}

impl Library {
    pub fn new(server_url: &str) -> Self {
        let cache = cache_dir();
        fs::create_dir_all(&cache).ok();
        let mut lib = Self {
            server_url: server_url.to_string(),
            books: vec![],
            cache_dir: cache,
        };
        lib.refresh();
        lib
    }

    pub fn refresh(&mut self) {
        let url = format!("{}/api/books", self.server_url);
        if let Ok(resp) = reqwest::blocking::get(&url) {
            if let Ok(books) = resp.json::<Vec<BookEntry>>() {
                // Prune cache for deleted books
                let ids: std::collections::HashSet<String> = books.iter().map(|b| b.id.clone()).collect();
                self.prune_cache(&ids);
                self.books = books;
            }
        }
    }

    pub fn import(&mut self, path: &Path) -> Result<String, String> {
        let client = reqwest::blocking::Client::new();
        let file_bytes = fs::read(path).map_err(|e| format!("Read error: {}", e))?;
        let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        let part = reqwest::blocking::multipart::Part::bytes(file_bytes)
            .file_name(filename)
            .mime_str("application/pdf")
            .map_err(|e| format!("{}", e))?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part);

        let resp = client
            .post(format!("{}/api/books", self.server_url))
            .multipart(form)
            .send()
            .map_err(|e| format!("Upload error: {}", e))?;

        if !resp.status().is_success() {
            let msg = resp.text().unwrap_or_default();
            return Err(format!("Server error: {}", msg));
        }

        let book: BookEntry = resp.json().map_err(|e| format!("Parse error: {}", e))?;
        let id = book.id.clone();
        self.books.insert(0, book);
        Ok(id)
    }

    pub fn delete(&mut self, id: &str) {
        let client = reqwest::blocking::Client::new();
        let ok = client.delete(format!("{}/api/books/{}", self.server_url, id))
            .send()
            .map(|r| r.status().is_success() || r.status().as_u16() == 204)
            .unwrap_or(false);
        if ok {
            self.books.retain(|b| b.id != id);
            let cache_path = self.cache_dir.join(format!("{}.pdf", id));
            fs::remove_file(cache_path).ok();
        }
    }

    pub fn update_progress(&mut self, id: &str, page: usize, sentence: usize, voice_id: &str) {
        let client = reqwest::blocking::Client::new();
        let ok = client
            .put(format!("{}/api/books/{}/progress", self.server_url, id))
            .json(&serde_json::json!({
                "last_page": page,
                "last_sentence": sentence,
                "selected_voice_id": voice_id
            }))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        // Only update local state if server confirmed
        if ok {
            if let Some(book) = self.books.iter_mut().find(|b| b.id == id) {
                book.last_page = page;
                book.last_sentence = sentence;
                book.selected_voice_id = voice_id.to_string();
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&BookEntry> {
        self.books.iter().find(|b| b.id == id)
    }

    /// Get local path to PDF, downloading if needed
    pub fn get_pdf_path(&self, id: &str) -> Result<PathBuf, String> {
        let cache_path = self.cache_dir.join(format!("{}.pdf", id));
        if cache_path.exists() {
            return Ok(cache_path);
        }

        // Download from server
        let url = format!("{}/api/books/{}/file", self.server_url, id);
        let resp = reqwest::blocking::get(&url).map_err(|e| format!("{}", e))?;
        if !resp.status().is_success() {
            return Err("Download failed".into());
        }
        let bytes = resp.bytes().map_err(|e| format!("{}", e))?;
        fs::write(&cache_path, &bytes).map_err(|e| format!("{}", e))?;
        Ok(cache_path)
    }

    fn prune_cache(&self, valid_ids: &std::collections::HashSet<String>) {
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".pdf") {
                    let id = name.trim_end_matches(".pdf");
                    if !valid_ids.contains(id) {
                        fs::remove_file(entry.path()).ok();
                    }
                }
            }
        }
    }
}
