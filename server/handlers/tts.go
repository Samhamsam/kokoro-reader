package handlers

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"log"
	"math"
	"net/http"

	"kokoro-server/tts"
)

type TTSHandler struct {
	Engine *tts.Engine
}

type TTSRequest struct {
	Text  string  `json:"text"`
	Voice string  `json:"voice"`
	Speed float32 `json:"speed"`
}

func (h *TTSHandler) Synthesize(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}

	var req TTSRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "invalid JSON", http.StatusBadRequest)
		return
	}

	if req.Text == "" {
		http.Error(w, "text required", http.StatusBadRequest)
		return
	}
	if req.Voice == "" {
		req.Voice = "kokoro_heart"
	}
	if req.Speed <= 0 {
		req.Speed = 1.0
	}

	log.Printf("TTS: voice=%s text=%.50s...", req.Voice, req.Text)

	samples, sampleRate := h.Engine.Generate(req.Voice, req.Text, req.Speed)
	if len(samples) == 0 {
		http.Error(w, fmt.Sprintf("voice '%s' not found or generation failed", req.Voice), http.StatusBadRequest)
		return
	}

	wav := samplesToWAV(samples, sampleRate)
	w.Header().Set("Content-Type", "audio/wav")
	w.Write(wav)
}

func (h *TTSHandler) ListVoices(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(h.Engine.ListVoices())
}

func samplesToWAV(samples []float32, sampleRate int) []byte {
	n := len(samples)
	dataSize := n * 2
	fileSize := 44 + dataSize
	buf := make([]byte, fileSize)
	copy(buf[0:4], "RIFF")
	binary.LittleEndian.PutUint32(buf[4:8], uint32(fileSize-8))
	copy(buf[8:12], "WAVE")
	copy(buf[12:16], "fmt ")
	binary.LittleEndian.PutUint32(buf[16:20], 16)
	binary.LittleEndian.PutUint16(buf[20:22], 1)
	binary.LittleEndian.PutUint16(buf[22:24], 1)
	binary.LittleEndian.PutUint32(buf[24:28], uint32(sampleRate))
	binary.LittleEndian.PutUint32(buf[28:32], uint32(sampleRate*2))
	binary.LittleEndian.PutUint16(buf[32:34], 2)
	binary.LittleEndian.PutUint16(buf[34:36], 16)
	copy(buf[36:40], "data")
	binary.LittleEndian.PutUint32(buf[40:44], uint32(dataSize))
	for i, s := range samples {
		val := int16(math.Max(-32768, math.Min(32767, float64(s*32767))))
		binary.LittleEndian.PutUint16(buf[44+i*2:46+i*2], uint16(val))
	}
	return buf
}
