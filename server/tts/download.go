package tts

import (
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
)

type modelDef struct {
	name    string
	url     string
	extract string // directory name after extraction
	check   string // file to check if already downloaded
}

var models = []modelDef{
	{
		name:    "Kokoro English",
		url:     "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-en-v0_19.tar.bz2",
		extract: "kokoro-en-v0_19",
		check:   "kokoro-en/model.onnx",
	},
	{
		name:    "Piper Thorsten (German)",
		url:     "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-de_DE-thorsten-high.tar.bz2",
		extract: "vits-piper-de_DE-thorsten-high",
		check:   "piper-thorsten/de_DE-thorsten-high.onnx",
	},
}

// EnsureModels downloads missing TTS models to modelsDir.
func EnsureModels(modelsDir string) error {
	os.MkdirAll(modelsDir, 0755)

	for _, m := range models {
		checkPath := filepath.Join(modelsDir, m.check)
		if _, err := os.Stat(checkPath); err == nil {
			log.Printf("Model %s: already present", m.name)
			continue
		}

		log.Printf("Model %s: downloading...", m.name)
		if err := downloadAndExtract(m, modelsDir); err != nil {
			return fmt.Errorf("download %s: %w", m.name, err)
		}
		log.Printf("Model %s: ready", m.name)
	}
	return nil
}

func downloadAndExtract(m modelDef, modelsDir string) error {
	// Download to temp file
	tmpFile, err := os.CreateTemp(modelsDir, "download-*.tar.bz2")
	if err != nil {
		return err
	}
	tmpPath := tmpFile.Name()
	defer os.Remove(tmpPath)

	resp, err := http.Get(m.url)
	if err != nil {
		tmpFile.Close()
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != 200 {
		tmpFile.Close()
		return fmt.Errorf("HTTP %d", resp.StatusCode)
	}

	size := resp.ContentLength
	written, err := io.Copy(tmpFile, resp.Body)
	tmpFile.Close()
	if err != nil {
		return err
	}
	if size > 0 {
		log.Printf("  Downloaded %.1f MB", float64(written)/1024/1024)
	}

	// Extract with tar
	cmd := exec.Command("tar", "xf", tmpPath, "-C", modelsDir)
	if out, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("tar: %s: %w", string(out), err)
	}

	// Rename extracted dir to expected name
	// e.g. kokoro-en-v0_19 → kokoro-en
	extractedDir := filepath.Join(modelsDir, m.extract)
	targetDir := filepath.Dir(filepath.Join(modelsDir, m.check))

	if extractedDir != targetDir {
		os.RemoveAll(targetDir) // remove partial if exists
		if err := os.Rename(extractedDir, targetDir); err != nil {
			return fmt.Errorf("rename %s → %s: %w", m.extract, filepath.Base(targetDir), err)
		}
	}

	return nil
}
