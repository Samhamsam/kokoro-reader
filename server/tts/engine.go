package tts

import (
	"log"
	"path/filepath"
	"sync"

	sherpa "github.com/k2-fsa/sherpa-onnx-go/sherpa_onnx"
)

type Voice struct {
	ID        string
	Name      string
	Lang      string
	TTS       *sherpa.OfflineTts
	SpeakerID int
}

type Engine struct {
	voices map[string]*Voice
	mu     sync.Mutex
}

func NewEngine(modelsDir string) *Engine {
	e := &Engine{voices: make(map[string]*Voice)}

	// Kokoro English
	kokoroDir := filepath.Join(modelsDir, "kokoro-en")
	if kokoro := loadKokoro(kokoroDir); kokoro != nil {
		e.addVoice("kokoro_heart", "EN Heart (F)", "en", kokoro, 0)
		e.addVoice("kokoro_bella", "EN Bella (F)", "en", kokoro, 2)
		e.addVoice("kokoro_sarah", "EN Sarah (F)", "en", kokoro, 3)
		e.addVoice("kokoro_sky", "EN Sky (F)", "en", kokoro, 4)
		e.addVoice("kokoro_adam", "EN Adam (M)", "en", kokoro, 5)
		e.addVoice("kokoro_michael", "EN Michael (M)", "en", kokoro, 6)
		log.Printf("Kokoro loaded: %d speakers", kokoro.NumSpeakers())
	}

	// Piper Thorsten (German)
	thorstenDir := filepath.Join(modelsDir, "piper-thorsten")
	if thorsten := loadPiper(thorstenDir, "de_DE-thorsten-high.onnx"); thorsten != nil {
		e.addVoice("piper_thorsten", "DE Thorsten (M)", "de", thorsten, 0)
		log.Println("Piper Thorsten loaded")
	}

	log.Printf("TTS Engine: %d voices loaded", len(e.voices))
	return e
}

func (e *Engine) addVoice(id, name, lang string, tts *sherpa.OfflineTts, speakerID int) {
	e.voices[id] = &Voice{ID: id, Name: name, Lang: lang, TTS: tts, SpeakerID: speakerID}
}

func (e *Engine) GetVoice(id string) *Voice {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.voices[id]
}

type VoiceInfo struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	Lang string `json:"lang"`
}

func (e *Engine) ListVoices() []VoiceInfo {
	e.mu.Lock()
	defer e.mu.Unlock()
	var list []VoiceInfo
	for _, v := range e.voices {
		list = append(list, VoiceInfo{ID: v.ID, Name: v.Name, Lang: v.Lang})
	}
	return list
}

func (e *Engine) Generate(voiceID string, text string, speed float32) ([]float32, int) {
	v := e.GetVoice(voiceID)
	if v == nil {
		return nil, 0
	}
	audio := v.TTS.Generate(text, v.SpeakerID, speed)
	return audio.Samples, audio.SampleRate
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
		log.Println("Failed to load Kokoro")
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
		log.Println("Failed to load Piper")
	}
	return tts
}
