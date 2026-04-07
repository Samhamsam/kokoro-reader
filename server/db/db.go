package db

import (
	"database/sql"
	"errors"
	"fmt"
	"time"

	_ "github.com/mattn/go-sqlite3"
)

type Book struct {
	ID              string `json:"id"`
	Title           string `json:"title"`
	OriginalFilename string `json:"-"` // not exposed in API
	TotalPages      int    `json:"total_pages"`
	LastPage        int    `json:"last_page"`
	LastSentence    int    `json:"last_sentence"`
	SelectedVoiceID string  `json:"selected_voice_id"`
	Speed           float32 `json:"speed"`
	Version         int64   `json:"version"`
	UpdatedAt       int64   `json:"updated_at"`
	LastAccessed    int64   `json:"last_accessed"`
	CreatedAt       int64   `json:"created_at,omitempty"`
}

type ProgressUpdate struct {
	LastPage        int     `json:"last_page"`
	LastSentence    int     `json:"last_sentence"`
	SelectedVoiceID string  `json:"selected_voice_id"`
	Speed           float32 `json:"speed"`
	BaseVersion     int64   `json:"base_version"`
}

type ConflictError struct {
	Current *Book
}

func (e *ConflictError) Error() string { return "version conflict" }

type DB struct {
	db *sql.DB
}

func Open(path string) (*DB, error) {
	sqlDB, err := sql.Open("sqlite3", path+"?_journal_mode=WAL")
	if err != nil {
		return nil, err
	}
	d := &DB{db: sqlDB}
	if err := d.migrate(); err != nil {
		return nil, err
	}
	return d, nil
}

func (d *DB) Close() error { return d.db.Close() }

func (d *DB) migrate() error {
	_, err := d.db.Exec(`
		CREATE TABLE IF NOT EXISTS books (
			id TEXT PRIMARY KEY,
			title TEXT NOT NULL,
			original_filename TEXT NOT NULL,
			total_pages INTEGER NOT NULL DEFAULT 0,
			last_page INTEGER DEFAULT 0,
			last_sentence INTEGER DEFAULT 0,
			selected_voice_id TEXT DEFAULT '',
			speed REAL DEFAULT 1.0,
			version INTEGER NOT NULL DEFAULT 1,
			updated_at INTEGER NOT NULL DEFAULT 0,
			last_accessed INTEGER DEFAULT 0,
			created_at INTEGER NOT NULL
		)
	`)
	if err != nil { return err }
	// Migration: add speed column if missing
	d.db.Exec(`ALTER TABLE books ADD COLUMN speed REAL DEFAULT 1.0`)
	d.db.Exec(`ALTER TABLE books ADD COLUMN version INTEGER NOT NULL DEFAULT 1`)
	d.db.Exec(`ALTER TABLE books ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0`)
	d.db.Exec(`UPDATE books SET updated_at = created_at WHERE updated_at = 0`)
	d.db.Exec(`UPDATE books SET version = 1 WHERE version <= 0`)

	_, err2 := d.db.Exec(`
		CREATE TABLE IF NOT EXISTS summaries (
			book_id TEXT NOT NULL,
			lang TEXT NOT NULL,
			summary TEXT NOT NULL,
			created_at INTEGER NOT NULL,
			PRIMARY KEY (book_id, lang),
			FOREIGN KEY (book_id) REFERENCES books(id) ON DELETE CASCADE
		)
	`)
	if err2 != nil { return err2 }

	return err
}

func (d *DB) ListBooks() ([]Book, error) {
	rows, err := d.db.Query(`
		SELECT id, title, total_pages, last_page, last_sentence, selected_voice_id, speed, version, updated_at, last_accessed
		FROM books ORDER BY last_accessed DESC
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var books []Book
	for rows.Next() {
		var b Book
		if err := rows.Scan(&b.ID, &b.Title, &b.TotalPages, &b.LastPage, &b.LastSentence, &b.SelectedVoiceID, &b.Speed, &b.Version, &b.UpdatedAt, &b.LastAccessed); err != nil {
			return nil, err
		}
		books = append(books, b)
	}
	if books == nil {
		books = []Book{}
	}
	return books, nil
}

func (d *DB) GetBook(id string) (*Book, error) {
	var b Book
	err := d.db.QueryRow(`
		SELECT id, title, original_filename, total_pages, last_page, last_sentence, selected_voice_id, speed, version, updated_at, last_accessed
		FROM books WHERE id = ?
	`, id).Scan(&b.ID, &b.Title, &b.OriginalFilename, &b.TotalPages, &b.LastPage, &b.LastSentence, &b.SelectedVoiceID, &b.Speed, &b.Version, &b.UpdatedAt, &b.LastAccessed)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &b, nil
}

func (d *DB) InsertBook(b *Book) error {
	b.CreatedAt = time.Now().Unix()
	b.UpdatedAt = b.CreatedAt
	b.LastAccessed = b.CreatedAt
	b.Version = 1
	_, err := d.db.Exec(`
		INSERT INTO books (id, title, original_filename, total_pages, last_page, last_sentence, selected_voice_id, speed, version, updated_at, last_accessed, created_at)
		VALUES (?, ?, ?, ?, 0, 0, '', 1.0, ?, ?, ?, ?)
	`, b.ID, b.Title, b.OriginalFilename, b.TotalPages, b.Version, b.UpdatedAt, b.LastAccessed, b.CreatedAt)
	return err
}

func (d *DB) UpdateProgress(id string, p ProgressUpdate) (*Book, error) {
	book, err := d.GetBook(id)
	if err != nil {
		return nil, err
	}
	if book == nil {
		return nil, fmt.Errorf("not found")
	}
	// Validate
	if p.LastPage < 0 || (book.TotalPages > 0 && p.LastPage >= book.TotalPages) {
		return nil, fmt.Errorf("invalid last_page: %d (total: %d)", p.LastPage, book.TotalPages)
	}
	if p.LastSentence < 0 {
		return nil, fmt.Errorf("invalid last_sentence: %d", p.LastSentence)
	}
	if p.BaseVersion > 0 && p.BaseVersion != book.Version {
		return nil, &ConflictError{Current: book}
	}

	speed := p.Speed
	if speed <= 0 { speed = 1.0 }
	now := time.Now().Unix()
	nextVersion := book.Version + 1
	_, err = d.db.Exec(`
		UPDATE books SET last_page = ?, last_sentence = ?, selected_voice_id = ?, speed = ?, version = ?, updated_at = ?, last_accessed = ?
		WHERE id = ?
	`, p.LastPage, p.LastSentence, p.SelectedVoiceID, speed, nextVersion, now, now, id)
	if err != nil {
		return nil, err
	}
	return d.GetBook(id)
}

func IsConflict(err error) (*ConflictError, bool) {
	var conflict *ConflictError
	if errors.As(err, &conflict) {
		return conflict, true
	}
	return nil, false
}

func (d *DB) DeleteBook(id string) error {
	_, err := d.db.Exec(`DELETE FROM books WHERE id = ?`, id)
	return err
}

// GetSummary returns a cached summary or nil if not found
func (d *DB) GetSummary(bookID, lang string) (string, bool) {
	var summary string
	err := d.db.QueryRow(`SELECT summary FROM summaries WHERE book_id = ? AND lang = ?`, bookID, lang).Scan(&summary)
	if err != nil {
		return "", false
	}
	return summary, true
}

// SaveSummary stores a summary for a book and language
func (d *DB) SaveSummary(bookID, lang, summary string) error {
	_, err := d.db.Exec(`
		INSERT INTO summaries (book_id, lang, summary, created_at)
		VALUES (?, ?, ?, ?)
		ON CONFLICT (book_id, lang) DO UPDATE SET summary = ?, created_at = ?
	`, bookID, lang, summary, time.Now().Unix(), summary, time.Now().Unix())
	return err
}

// AllBookIDs returns all book IDs for cleanup purposes
func (d *DB) AllBookIDs() (map[string]bool, error) {
	rows, err := d.db.Query(`SELECT id FROM books`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	ids := make(map[string]bool)
	for rows.Next() {
		var id string
		rows.Scan(&id)
		ids[id] = true
	}
	return ids, nil
}
