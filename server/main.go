package main

import (
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"kokoro-server/db"
	"kokoro-server/handlers"
	"kokoro-server/tts"
)

func main() {
	dataDir := "data"
	modelsDir := "models-tts"

	// Allow override via args
	if len(os.Args) > 1 {
		modelsDir = os.Args[1]
	}
	if len(os.Args) > 2 {
		dataDir = os.Args[2]
	}

	// Ensure dirs
	os.MkdirAll(filepath.Join(dataDir, "books"), 0755)

	// Open database
	database, err := db.Open(filepath.Join(dataDir, "kokoro.db"))
	if err != nil {
		log.Fatalf("Failed to open database: %v", err)
	}
	defer database.Close()

	// Download TTS models if missing
	if err := tts.EnsureModels(modelsDir); err != nil {
		log.Printf("Warning: model download failed: %v", err)
	}

	// Load TTS models
	engine := tts.NewEngine(modelsDir)

	// Handlers
	bookHandler := &handlers.BookHandler{DB: database, BooksDir: filepath.Join(dataDir, "books")}
	ttsHandler := &handlers.TTSHandler{Engine: engine}

	// Startup cleanup
	bookHandler.CleanupOrphanedFiles()

	// Routes
	mux := http.NewServeMux()

	// Books API
	mux.HandleFunc("/api/books", func(w http.ResponseWriter, r *http.Request) {
		switch r.Method {
		case http.MethodGet:
			bookHandler.List(w, r)
		case http.MethodPost:
			bookHandler.Upload(w, r)
		default:
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		}
	})

	mux.HandleFunc("/api/books/", func(w http.ResponseWriter, r *http.Request) {
		// Parse: /api/books/{id}/...
		path := strings.TrimPrefix(r.URL.Path, "/api/books/")
		parts := strings.SplitN(path, "/", 2)
		id := parts[0]
		sub := ""
		if len(parts) > 1 {
			sub = parts[1]
		}

		switch {
		case sub == "file" && r.Method == http.MethodGet:
			bookHandler.GetFile(w, r, id)
		case sub == "progress" && r.Method == http.MethodPut:
			bookHandler.UpdateProgress(w, r, id)
		case sub == "" && r.Method == http.MethodGet:
			bookHandler.Get(w, r, id)
		case sub == "" && r.Method == http.MethodDelete:
			bookHandler.Delete(w, r, id)
		default:
			http.Error(w, "not found", http.StatusNotFound)
		}
	})

	// TTS API
	mux.HandleFunc("/api/tts", ttsHandler.Synthesize)
	mux.HandleFunc("/api/voices", ttsHandler.ListVoices)

	// Legacy endpoints (backward compat)
	mux.HandleFunc("/tts", ttsHandler.Synthesize)
	mux.HandleFunc("/voices", ttsHandler.ListVoices)

	// Health
	mux.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("ok"))
	})

	// CORS middleware for Android
	handler := corsMiddleware(mux)

	addr := ":8787"
	log.Printf("Kokoro Server listening on %s", addr)
	log.Fatal(http.ListenAndServe(addr, handler))
}

func corsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusOK)
			return
		}
		next.ServeHTTP(w, r)
	})
}
