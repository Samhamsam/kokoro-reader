package main

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"log"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"sync"

	sherpa "github.com/k2-fsa/sherpa-onnx-go/sherpa_onnx"
)

type Voice struct {
	Name      string
	TTS       *sherpa.OfflineTts
	SpeakerID int
}

var (
	voices   map[string]*Voice
	voicesMu sync.Mutex
)

type TTSRequest struct {
	Text    string  `json:"text"`
	Voice   string  `json:"voice"`
	Speed   float32 `json:"speed"`
}

type VoiceInfo struct {
	Name string `json:"name"`
	Lang string `json:"lang"`
}

func main() {
	modelsDir := "models"
	if len(os.Args) > 1 {
		modelsDir = os.Args[1]
	}

	voices = make(map[string]*Voice)

	// Load Kokoro English
	kokoroDir := filepath.Join(modelsDir, "kokoro-en")
	if _, err := os.Stat(filepath.Join(kokoroDir, "model.onnx")); err == nil {
		log.Println("Loading Kokoro English model...")
		kokoro := loadKokoro(kokoroDir)
		if kokoro != nil {
			voices["kokoro_heart"] = &Voice{Name: "EN Heart (F)", TTS: kokoro, SpeakerID: 0}
			voices["kokoro_bella"] = &Voice{Name: "EN Bella (F)", TTS: kokoro, SpeakerID: 2}
			voices["kokoro_sarah"] = &Voice{Name: "EN Sarah (F)", TTS: kokoro, SpeakerID: 3}
			voices["kokoro_sky"] = &Voice{Name: "EN Sky (F)", TTS: kokoro, SpeakerID: 4}
			voices["kokoro_adam"] = &Voice{Name: "EN Adam (M)", TTS: kokoro, SpeakerID: 5}
			voices["kokoro_michael"] = &Voice{Name: "EN Michael (M)", TTS: kokoro, SpeakerID: 6}
			log.Printf("Kokoro loaded: %d speakers\n", kokoro.NumSpeakers())
		}
	}

	// Load Piper Thorsten (German)
	thorstenDir := filepath.Join(modelsDir, "piper-thorsten")
	if _, err := os.Stat(filepath.Join(thorstenDir, "de_DE-thorsten-high.onnx")); err == nil {
		log.Println("Loading Piper Thorsten German model...")
		thorsten := loadPiper(thorstenDir, "de_DE-thorsten-high.onnx")
		if thorsten != nil {
			voices["piper_thorsten"] = &Voice{Name: "DE Thorsten (M)", TTS: thorsten, SpeakerID: 0}
			log.Println("Piper Thorsten loaded")
		}
	}

	if len(voices) == 0 {
		log.Fatal("No models found! Place models in ./models/ directory")
	}

	http.HandleFunc("/tts", handleTTS)
	http.HandleFunc("/voices", handleVoices)
	http.HandleFunc("/health", handleHealth)

	addr := ":8787"
	log.Printf("Kokoro TTS Server listening on %s (%d voices)\n", addr, len(voices))
	log.Fatal(http.ListenAndServe(addr, nil))
}

func loadKokoro(dir string) *sherpa.OfflineTts {
	config := sherpa.OfflineTtsConfig{}
	config.Model.Kokoro.Model = filepath.Join(dir, "model.onnx")
	config.Model.Kokoro.Voices = filepath.Join(dir, "voices.bin")
	config.Model.Kokoro.Tokens = filepath.Join(dir, "tokens.txt")
	config.Model.Kokoro.DataDir = filepath.Join(dir, "espeak-ng-data")
	config.Model.Kokoro.LengthScale = 1.0
	config.Model.NumThreads = 4

	tts := sherpa.NewOfflineTts(&config)
	if tts == nil {
		log.Println("Failed to load Kokoro model")
		return nil
	}
	return tts
}

func loadPiper(dir, modelFile string) *sherpa.OfflineTts {
	config := sherpa.OfflineTtsConfig{}
	config.Model.Vits.Model = filepath.Join(dir, modelFile)
	config.Model.Vits.Tokens = filepath.Join(dir, "tokens.txt")
	config.Model.Vits.DataDir = filepath.Join(dir, "espeak-ng-data")
	config.Model.Vits.LengthScale = 1.0
	config.Model.NumThreads = 4

	tts := sherpa.NewOfflineTts(&config)
	if tts == nil {
		log.Println("Failed to load Piper model")
		return nil
	}
	return tts
}

func handleTTS(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}

	var req TTSRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON", http.StatusBadRequest)
		return
	}

	if req.Text == "" {
		http.Error(w, "text is required", http.StatusBadRequest)
		return
	}
	if req.Voice == "" {
		req.Voice = "kokoro_heart"
	}
	if req.Speed <= 0 {
		req.Speed = 1.0
	}

	voicesMu.Lock()
	voice, ok := voices[req.Voice]
	voicesMu.Unlock()

	if !ok {
		http.Error(w, fmt.Sprintf("voice '%s' not found", req.Voice), http.StatusBadRequest)
		return
	}

	// Generate audio
	audio := voice.TTS.Generate(req.Text, voice.SpeakerID, req.Speed)
	if len(audio.Samples) == 0 {
		http.Error(w, "failed to generate audio", http.StatusInternalServerError)
		return
	}

	// Convert to WAV
	wav := samplesToWAV(audio.Samples, audio.SampleRate)

	w.Header().Set("Content-Type", "audio/wav")
	w.Header().Set("X-Sample-Rate", fmt.Sprintf("%d", audio.SampleRate))
	w.Header().Set("X-Samples", fmt.Sprintf("%d", len(audio.Samples)))
	w.Write(wav)
}

func handleVoices(w http.ResponseWriter, r *http.Request) {
	voicesMu.Lock()
	defer voicesMu.Unlock()

	var list []VoiceInfo
	for id, v := range voices {
		lang := "en"
		if id == "piper_thorsten" {
			lang = "de"
		}
		list = append(list, VoiceInfo{Name: v.Name, Lang: lang})
	}

	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(list)
}

func handleHealth(w http.ResponseWriter, r *http.Request) {
	w.Write([]byte("ok"))
}

func samplesToWAV(samples []float32, sampleRate int) []byte {
	numSamples := len(samples)
	dataSize := numSamples * 2 // 16-bit
	fileSize := 44 + dataSize

	buf := make([]byte, fileSize)

	// RIFF header
	copy(buf[0:4], "RIFF")
	binary.LittleEndian.PutUint32(buf[4:8], uint32(fileSize-8))
	copy(buf[8:12], "WAVE")

	// fmt chunk
	copy(buf[12:16], "fmt ")
	binary.LittleEndian.PutUint32(buf[16:20], 16) // chunk size
	binary.LittleEndian.PutUint16(buf[20:22], 1)  // PCM
	binary.LittleEndian.PutUint16(buf[22:24], 1)  // mono
	binary.LittleEndian.PutUint32(buf[24:28], uint32(sampleRate))
	binary.LittleEndian.PutUint32(buf[28:32], uint32(sampleRate*2)) // byte rate
	binary.LittleEndian.PutUint16(buf[32:34], 2)                   // block align
	binary.LittleEndian.PutUint16(buf[34:36], 16)                  // bits per sample

	// data chunk
	copy(buf[36:40], "data")
	binary.LittleEndian.PutUint32(buf[40:44], uint32(dataSize))

	for i, s := range samples {
		val := int16(math.Max(-32768, math.Min(32767, float64(s*32767))))
		binary.LittleEndian.PutUint16(buf[44+i*2:46+i*2], uint16(val))
	}

	return buf
}
