package handlers

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"kokoro-server/db"

	"github.com/google/uuid"
	"github.com/pdfcpu/pdfcpu/pkg/api"
)

type BookHandler struct {
	DB       *db.DB
	BooksDir string
}

func (h *BookHandler) List(w http.ResponseWriter, r *http.Request) {
	books, err := h.DB.ListBooks()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(books)
}

func (h *BookHandler) Upload(w http.ResponseWriter, r *http.Request) {
	// Max 200MB
	r.ParseMultipartForm(200 << 20)
	file, header, err := r.FormFile("file")
	if err != nil {
		http.Error(w, "file field required", http.StatusBadRequest)
		return
	}
	defer file.Close()

	id := uuid.New().String()
	tmpPath := filepath.Join(h.BooksDir, id+".tmp")
	finalPath := filepath.Join(h.BooksDir, id+".pdf")

	// Write to temp file
	tmp, err := os.Create(tmpPath)
	if err != nil {
		http.Error(w, "failed to create temp file", http.StatusInternalServerError)
		return
	}
	if _, err := io.Copy(tmp, file); err != nil {
		tmp.Close()
		os.Remove(tmpPath)
		http.Error(w, "failed to write file", http.StatusInternalServerError)
		return
	}
	tmp.Close()

	// Validate PDF and get page count
	totalPages, err := getPDFPageCount(tmpPath)
	if err != nil {
		os.Remove(tmpPath)
		http.Error(w, fmt.Sprintf("invalid PDF: %v", err), http.StatusBadRequest)
		return
	}

	// Atomic rename
	if err := os.Rename(tmpPath, finalPath); err != nil {
		os.Remove(tmpPath)
		http.Error(w, "failed to store file", http.StatusInternalServerError)
		return
	}

	// Extract title from original filename
	title := strings.TrimSuffix(header.Filename, filepath.Ext(header.Filename))

	book := &db.Book{
		ID:              id,
		Title:           title,
		OriginalFilename: header.Filename,
		TotalPages:      totalPages,
	}

	if err := h.DB.InsertBook(book); err != nil {
		os.Remove(finalPath) // cleanup
		http.Error(w, "failed to save to database", http.StatusInternalServerError)
		return
	}

	log.Printf("Uploaded: %s (%d pages) → %s", title, totalPages, id)

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(book)
}

func (h *BookHandler) GetFile(w http.ResponseWriter, r *http.Request, id string) {
	path := filepath.Join(h.BooksDir, id+".pdf")
	if _, err := os.Stat(path); os.IsNotExist(err) {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}
	w.Header().Set("Content-Type", "application/pdf")
	http.ServeFile(w, r, path)
}

func (h *BookHandler) Get(w http.ResponseWriter, r *http.Request, id string) {
	book, err := h.DB.GetBook(id)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if book == nil {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(book)
}

func (h *BookHandler) UpdateProgress(w http.ResponseWriter, r *http.Request, id string) {
	var p db.ProgressUpdate
	if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
		http.Error(w, "invalid JSON", http.StatusBadRequest)
		return
	}
	book, err := h.DB.UpdateProgress(id, p)
	if err != nil {
		if err.Error() == "not found" {
			http.Error(w, "book not found", http.StatusNotFound)
		} else if conflict, ok := db.IsConflict(err); ok {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusConflict)
			json.NewEncoder(w).Encode(conflict.Current)
		} else if strings.HasPrefix(err.Error(), "invalid") {
			http.Error(w, err.Error(), http.StatusBadRequest)
		} else {
			http.Error(w, err.Error(), http.StatusInternalServerError)
		}
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(book)
}

func (h *BookHandler) Delete(w http.ResponseWriter, r *http.Request, id string) {
	if err := h.DB.DeleteBook(id); err != nil {
		log.Printf("Delete DB error for %s: %v", id, err)
		http.Error(w, "database error", http.StatusInternalServerError)
		return
	}
	os.Remove(filepath.Join(h.BooksDir, id+".pdf"))
	w.WriteHeader(http.StatusNoContent)
}

// CleanupOrphanedFiles removes PDF files without matching DB entries
func (h *BookHandler) CleanupOrphanedFiles() {
	ids, err := h.DB.AllBookIDs()
	if err != nil {
		return
	}
	entries, _ := os.ReadDir(h.BooksDir)
	for _, e := range entries {
		if !strings.HasSuffix(e.Name(), ".pdf") {
			continue
		}
		id := strings.TrimSuffix(e.Name(), ".pdf")
		if !ids[id] {
			log.Printf("Cleanup: removing orphaned file %s", e.Name())
			os.Remove(filepath.Join(h.BooksDir, e.Name()))
		}
	}
	// Also remove .tmp files
	for _, e := range entries {
		if strings.HasSuffix(e.Name(), ".tmp") {
			os.Remove(filepath.Join(h.BooksDir, e.Name()))
		}
	}
}

func getPDFPageCount(path string) (int, error) {
	ctx, err := api.ReadContextFile(path)
	if err != nil {
		return 0, err
	}
	if err := ctx.EnsurePageCount(); err != nil {
		return 0, err
	}
	return ctx.PageCount, nil
}
