package handlers

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"time"
	"path/filepath"
	"strings"

	"kokoro-server/db"

	pdftext "github.com/ledongthuc/pdf"
)

type SummaryHandler struct {
	DB       *db.DB
	BooksDir string
	APIKey   string // Groq API key
}

type summaryRequest struct {
	Lang string `json:"lang"` // e.g. "de", "en"
}

type summaryResponse struct {
	Summary string `json:"summary"`
	Lang    string `json:"lang"`
}

// Summarize generates a summary of a book using Groq LLM
func (h *SummaryHandler) Summarize(w http.ResponseWriter, r *http.Request, id string) {
	if h.APIKey == "" {
		http.Error(w, "GROQ_API_KEY not configured", http.StatusServiceUnavailable)
		return
	}

	book, err := h.DB.GetBook(id)
	if err != nil || book == nil {
		http.Error(w, "book not found", http.StatusNotFound)
		return
	}

	lang := r.URL.Query().Get("lang")
	if lang == "" {
		lang = "en"
	}

	pdfPath := filepath.Join(h.BooksDir, id+".pdf")
	if _, err := os.Stat(pdfPath); os.IsNotExist(err) {
		http.Error(w, "PDF file not found", http.StatusNotFound)
		return
	}

	// Check cache first
	regenerate := r.URL.Query().Get("regenerate") == "true"
	if !regenerate {
		if cached, ok := h.DB.GetSummary(id, lang); ok {
			log.Printf("Summary: serving cached for '%s' (%s)", book.Title, lang)
			w.Header().Set("Content-Type", "application/json")
			json.NewEncoder(w).Encode(summaryResponse{Summary: cached, Lang: lang})
			return
		}
	}

	// Extract text from PDF
	log.Printf("Summary: extracting text from %s (%d pages)", book.Title, book.TotalPages)
	text, err := extractPDFText(pdfPath)
	if err != nil {
		http.Error(w, fmt.Sprintf("text extraction failed: %v", err), http.StatusInternalServerError)
		return
	}

	if len(strings.TrimSpace(text)) < 100 {
		http.Error(w, "not enough text in PDF for summary", http.StatusBadRequest)
		return
	}

	// Chunk and summarize
	log.Printf("Summary: generating for '%s' in %s (%d chars)", book.Title, lang, len(text))
	summary, err := h.generateSummary(book.Title, text, lang)
	if err != nil {
		log.Printf("Summary error: %v", err)
		http.Error(w, fmt.Sprintf("summary generation failed: %v", err), http.StatusInternalServerError)
		return
	}

	// Save to cache
	if err := h.DB.SaveSummary(id, lang, summary); err != nil {
		log.Printf("Summary: cache save error: %v", err)
	}

	log.Printf("Summary: done for '%s' (%d chars)", book.Title, len(summary))
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(summaryResponse{Summary: summary, Lang: lang})
}

func (h *SummaryHandler) generateSummary(title, text, lang string) (string, error) {
	// Split into chunks of ~6000 chars (~1500 tokens) to stay within Groq free tier limits
	chunks := splitText(text, 6000)

	langName := langFullName(lang)

	if len(chunks) <= 3 {
		// Short book: summarize directly
		prompt := fmt.Sprintf(
			"You are a professional book summarizer. Summarize the following book titled \"%s\" in %s. "+
				"Create a comprehensive summary covering all key points, arguments, and insights. "+
				"The summary should be detailed and well-structured with sections. "+
				"Write approximately 5-10 pages worth of content.\n\n%s",
			title, langName, text)
		return h.callGroq(prompt)
	}

	// Long book: chunk-wise summarization
	var chunkSummaries []string
	for i, chunk := range chunks {
		log.Printf("Summary: processing chunk %d/%d", i+1, len(chunks))
		prompt := fmt.Sprintf(
			"Summarize the following section (part %d of %d) of the book \"%s\" in %s. "+
				"Cover all key points, arguments, examples, and insights. Be thorough.\n\n%s",
			i+1, len(chunks), title, langName, chunk)
		summary, err := h.callGroq(prompt)
		if err != nil {
			return "", fmt.Errorf("chunk %d: %w", i+1, err)
		}
		chunkSummaries = append(chunkSummaries, summary)
	}

	// Final: combine chunk summaries into one coherent summary
	combined := strings.Join(chunkSummaries, "\n\n---\n\n")
	finalPrompt := fmt.Sprintf(
		"You are a professional book summarizer. Below are summaries of individual sections of the book \"%s\". "+
			"Combine them into one comprehensive, well-structured summary in %s. "+
			"Organize by themes/chapters, not by section numbers. Remove redundancy. "+
			"The final summary should cover all important points and be approximately 5-10 pages long.\n\n%s",
		title, langName, combined)

	return h.callGroq(finalPrompt)
}

type groqRequest struct {
	Model    string        `json:"model"`
	Messages []groqMessage `json:"messages"`
}

type groqMessage struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

type groqResponse struct {
	Choices []struct {
		Message struct {
			Content string `json:"content"`
		} `json:"message"`
	} `json:"choices"`
	Error *struct {
		Message string `json:"message"`
	} `json:"error"`
}

func (h *SummaryHandler) callGroq(prompt string) (string, error) {
	reqBody := groqRequest{
		Model: "llama-3.3-70b-versatile",
		Messages: []groqMessage{
			{Role: "user", Content: prompt},
		},
	}

	jsonBody, err := json.Marshal(reqBody)
	if err != nil {
		return "", err
	}

	// Retry with backoff for rate limits
	for attempt := 0; attempt < 5; attempt++ {
		req, err := http.NewRequest("POST", "https://api.groq.com/openai/v1/chat/completions", bytes.NewReader(jsonBody))
		if err != nil {
			return "", err
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Authorization", "Bearer "+h.APIKey)

		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return "", err
		}

		body, _ := io.ReadAll(resp.Body)
		resp.Body.Close()

		if resp.StatusCode == 429 {
			wait := time.Duration(15*(attempt+1)) * time.Second
			log.Printf("  Rate limited, waiting %v...", wait)
			time.Sleep(wait)
			continue
		}

		var groqResp groqResponse
		if err := json.Unmarshal(body, &groqResp); err != nil {
			return "", fmt.Errorf("parse error: %s", string(body[:min(len(body), 200)]))
		}

		if groqResp.Error != nil {
			return "", fmt.Errorf("groq: %s", groqResp.Error.Message)
		}

		if len(groqResp.Choices) == 0 {
			return "", fmt.Errorf("no response from Groq")
		}

		return groqResp.Choices[0].Message.Content, nil
	}

	return "", fmt.Errorf("rate limit: too many retries")
}

func extractPDFText(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()

	fi, err := f.Stat()
	if err != nil {
		return "", err
	}

	reader, err := pdftext.NewReader(f, fi.Size())
	if err != nil {
		return "", err
	}

	var buf bytes.Buffer
	for i := 1; i <= reader.NumPage(); i++ {
		page := reader.Page(i)
		if page.V.IsNull() {
			continue
		}
		text, err := page.GetPlainText(nil)
		if err != nil {
			continue
		}
		buf.WriteString(text)
		buf.WriteString("\n")
	}

	return buf.String(), nil
}

func splitText(text string, chunkSize int) []string {
	if len(text) <= chunkSize {
		return []string{text}
	}

	var chunks []string
	for len(text) > 0 {
		end := chunkSize
		if end > len(text) {
			end = len(text)
		} else {
			// Try to split at paragraph or sentence boundary
			for i := end; i > chunkSize*3/4; i-- {
				if text[i] == '\n' && i+1 < len(text) && text[i+1] == '\n' {
					end = i + 1
					break
				}
			}
			if end == chunkSize {
				for i := end; i > chunkSize*3/4; i-- {
					if text[i] == '.' || text[i] == '!' || text[i] == '?' {
						end = i + 1
						break
					}
				}
			}
		}
		chunks = append(chunks, strings.TrimSpace(text[:end]))
		text = text[end:]
	}
	return chunks
}

func langFullName(code string) string {
	switch strings.ToLower(code) {
	case "de":
		return "German"
	case "en":
		return "English"
	case "fr":
		return "French"
	case "es":
		return "Spanish"
	case "it":
		return "Italian"
	case "pt":
		return "Portuguese"
	case "nl":
		return "Dutch"
	case "pl":
		return "Polish"
	case "tr":
		return "Turkish"
	case "ja":
		return "Japanese"
	case "zh":
		return "Chinese"
	case "ko":
		return "Korean"
	case "ru":
		return "Russian"
	default:
		return code
	}
}
