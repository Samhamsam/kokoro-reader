package db

import (
	"database/sql"
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
	LastAccessed    int64   `json:"last_accessed"`
	CreatedAt       int64   `json:"created_at,omitempty"`
}

type ProgressUpdate struct {
	LastPage        int     `json:"last_page"`
	LastSentence    int     `json:"last_sentence"`
	SelectedVoiceID string  `json:"selected_voice_id"`
	Speed           float32 `json:"speed"`
}

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
			last_accessed INTEGER DEFAULT 0,
			created_at INTEGER NOT NULL
		)
	`)
	if err != nil { return err }
	// Migration: add speed column if missing
	d.db.Exec(`ALTER TABLE books ADD COLUMN speed REAL DEFAULT 1.0`)
	return err
}

func (d *DB) ListBooks() ([]Book, error) {
	rows, err := d.db.Query(`
		SELECT id, title, total_pages, last_page, last_sentence, selected_voice_id, speed, last_accessed
		FROM books ORDER BY last_accessed DESC
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var books []Book
	for rows.Next() {
		var b Book
		if err := rows.Scan(&b.ID, &b.Title, &b.TotalPages, &b.LastPage, &b.LastSentence, &b.SelectedVoiceID, &b.Speed, &b.LastAccessed); err != nil {
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
		SELECT id, title, original_filename, total_pages, last_page, last_sentence, selected_voice_id, speed, last_accessed
		FROM books WHERE id = ?
	`, id).Scan(&b.ID, &b.Title, &b.OriginalFilename, &b.TotalPages, &b.LastPage, &b.LastSentence, &b.SelectedVoiceID, &b.Speed, &b.LastAccessed)
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
	b.LastAccessed = b.CreatedAt
	_, err := d.db.Exec(`
		INSERT INTO books (id, title, original_filename, total_pages, last_page, last_sentence, selected_voice_id, last_accessed, created_at)
		VALUES (?, ?, ?, ?, 0, 0, '', ?, ?)
	`, b.ID, b.Title, b.OriginalFilename, b.TotalPages, b.LastAccessed, b.CreatedAt)
	return err
}

func (d *DB) UpdateProgress(id string, p ProgressUpdate) error {
	book, err := d.GetBook(id)
	if err != nil {
		return err
	}
	if book == nil {
		return fmt.Errorf("not found")
	}
	// Validate
	if p.LastPage < 0 || (book.TotalPages > 0 && p.LastPage >= book.TotalPages) {
		return fmt.Errorf("invalid last_page: %d (total: %d)", p.LastPage, book.TotalPages)
	}
	if p.LastSentence < 0 {
		return fmt.Errorf("invalid last_sentence: %d", p.LastSentence)
	}

	speed := p.Speed
	if speed <= 0 { speed = 1.0 }
	_, err = d.db.Exec(`
		UPDATE books SET last_page = ?, last_sentence = ?, selected_voice_id = ?, speed = ?, last_accessed = ?
		WHERE id = ?
	`, p.LastPage, p.LastSentence, p.SelectedVoiceID, speed, time.Now().Unix(), id)
	return err
}

func (d *DB) DeleteBook(id string) error {
	_, err := d.db.Exec(`DELETE FROM books WHERE id = ?`, id)
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
