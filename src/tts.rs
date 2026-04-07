use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hound;
use rodio;

/// Phonemize text for Piper with NFD Unicode normalization.
/// The official Piper C++ uses NFD-normalized IPA phonemes.
/// Without this, characters like ç (German "ch") are not recognized by the model.
fn piper_phonemize(text: &str, voice_name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    // Derive espeak voice from piper voice name: de_DE-thorsten-high → de
    let espeak_voice = if voice_name.starts_with("de_DE") {
        "de"
    } else if voice_name.starts_with("en_") {
        "en"
    } else if voice_name.starts_with("fr_") {
        "fr"
    } else {
        &voice_name[..2]
    };

    let phonemes = espeak_rs::text_to_phonemes(text, espeak_voice, None, true, false)
        .unwrap_or_default();
    let ipa = phonemes.join(" ");

    // NFD normalize — decomposes ç into c + combining cedilla, etc.
    ipa.nfd().collect::<String>()
}

fn hash_string(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Voices: (id, label). IDs starting with "piper:" use Piper TTS, others use Kokoro.
pub const VOICES: &[(&str, &str)] = &[
    ("af_heart", "EN  Heart (F)"),
    ("af_nova", "EN  Nova (F)"),
    ("af_sky", "EN  Sky (F)"),
    ("af_sarah", "EN  Sarah (F)"),
    ("af_bella", "EN  Bella (F)"),
    ("am_adam", "EN  Adam (M)"),
    ("am_michael", "EN  Michael (M)"),
    ("am_eric", "EN  Eric (M)"),
    ("bf_emma", "GB  Emma (F)"),
    ("bf_alice", "GB  Alice (F)"),
    ("bm_george", "GB  George (M)"),
    ("bm_daniel", "GB  Daniel (M)"),
    ("piper:de_DE-thorsten-high", "DE  Thorsten (M)"),
    ("ff_siwis", "FR  Siwis (F)"),
    ("if_sara", "IT  Sara (F)"),
    ("jf_alpha", "JA  Alpha (F)"),
];

const SAMPLE_RATE: u32 = 24000;

fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else if "♦♣♠♥★☆●○◆◇■□▪▫▲△▼▽•‣⁃※†‡§¶".contains(c) {
                // Replace decorative/bullet symbols with space
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const ABBREVIATIONS: &[&str] = &[
    "Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "Sr.", "Jr.", "St.", "Mt.",
    "Rev.", "Gen.", "Gov.", "Sgt.", "Cpl.", "Pvt.", "Lt.", "Col.", "Capt.",
    "Corp.", "Inc.", "Ltd.", "Co.", "vs.", "etc.", "approx.", "dept.",
    "est.", "vol.", "no.", "fig.", "Jan.", "Feb.", "Mar.", "Apr.",
    "Jun.", "Jul.", "Aug.", "Sep.", "Oct.", "Nov.", "Dec.",
    "i.e.", "e.g.", "U.S.", "U.K.", "U.N.", "a.m.", "p.m.",
];

/// Check if the period at the end of `text` is an abbreviation, not a sentence end.
fn is_abbreviation(text: &str) -> bool {
    let text = text.trim_end();
    if !text.ends_with('.') {
        return false;
    }

    // Single uppercase letter followed by dot: "D.", "J.", "F."
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len >= 2 {
        let before_dot = chars[len - 2];
        if before_dot.is_uppercase() {
            // Check it's a standalone initial: preceded by space/start or another initial
            if len == 2 {
                return true; // just "D."
            }
            let before_letter = chars[len - 3];
            if before_letter == ' ' || before_letter == '.' {
                return true; // " D." or "J.D."
            }
        }
    }

    // Known abbreviations
    for abbr in ABBREVIATIONS {
        if text.ends_with(abbr) {
            return true;
        }
    }

    false
}

pub fn split_into_sentences(text: &str) -> Vec<String> {
    let text = clean_text(text);
    if text.is_empty() {
        return vec![];
    }

    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if ch == '!' || ch == '?' || ch == ';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        } else if ch == '.' {
            if !is_abbreviation(&current) {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current.clear();
            }
        } else if ch == ':' {
            // Only split on colon if followed by enough text (avoid "Chapter 1:")
            // We'll split at colon only if current is long enough
            if current.len() > 50 {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current.clear();
            }
        }
        if current.len() > 400 {
            if let Some(pos) = current.rfind(' ') {
                let (left, right) = current.split_at(pos);
                let trimmed = left.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current = right.trim().to_string();
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }
    sentences.retain(|s| has_speakable_words(s));
    sentences
}

/// Returns true if the string contains at least one real word (letters).
/// Filters out dividers, page numbers, bullet points, etc.
fn has_speakable_words(s: &str) -> bool {
    let letter_count = s.chars().filter(|c| c.is_alphabetic()).count();
    letter_count >= 2
}

fn samples_to_wav(samples: &[f32]) -> Vec<u8> {
    let mut buf = Vec::new();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::new(Cursor::new(&mut buf), spec).unwrap();
        for &s in samples {
            let s16 = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
            writer.write_sample(s16).unwrap();
        }
        writer.finalize().unwrap();
    }
    buf
}

#[derive(Clone, PartialEq)]
pub enum TtsState {
    Idle,
    Loading,    // model loading
    Generating,
    Playing,
    Paused,
    Finished,
    Error(String),
}

struct PlaybackInfo {
    generating_idx: usize,
    playing_idx: usize,
    total: usize,
    durations: Vec<f32>,
    play_start: Option<Instant>,
}

/// Pre-synthesized audio for sentences, keyed by sentence text
struct PreCache {
    /// Map from sentence text -> audio samples
    audio: HashMap<String, Vec<f32>>,
    /// Which page text this cache is for (invalidate on page change)
    page_text_hash: u64,
}

const PIPER_VOICES_DIR: &str = ".cache/piper-voices";
const PIPER_BASE_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main";

/// Get piper model paths, downloading if needed
fn get_piper_model_paths(voice_name: &str) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::Path::new(&home).join(PIPER_VOICES_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;

    let onnx_file = dir.join(format!("{}.onnx", voice_name));
    let config_file = dir.join(format!("{}.onnx.json", voice_name));

    // Download if not present
    if !onnx_file.exists() || !config_file.exists() {
        // Parse voice name to get URL path: de_DE-thorsten-high → de/de_DE/thorsten/high/
        let parts: Vec<&str> = voice_name.split('-').collect();
        if parts.len() >= 3 {
            let lang_region = parts[0]; // de_DE
            let lang = &lang_region[..2]; // de
            let name = parts[1]; // thorsten
            let quality = parts[2]; // high

            let base = format!("{}/{}/{}/{}/{}", PIPER_BASE_URL, lang, lang_region, name, quality);

            eprintln!("Downloading Piper voice {}...", voice_name);
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if !onnx_file.exists() {
                    let url = format!("{}/{}.onnx", base, voice_name);
                    let bytes = reqwest::get(&url).await
                        .map_err(|e| format!("Download failed: {}", e))?
                        .bytes().await
                        .map_err(|e| format!("Download failed: {}", e))?;
                    std::fs::write(&onnx_file, &bytes)
                        .map_err(|e| format!("Write failed: {}", e))?;
                    eprintln!("Downloaded {}.onnx", voice_name);
                }
                if !config_file.exists() {
                    let url = format!("{}/{}.onnx.json", base, voice_name);
                    let bytes = reqwest::get(&url).await
                        .map_err(|e| format!("Download failed: {}", e))?
                        .bytes().await
                        .map_err(|e| format!("Download failed: {}", e))?;
                    std::fs::write(&config_file, &bytes)
                        .map_err(|e| format!("Write failed: {}", e))?;
                    eprintln!("Downloaded {}.onnx.json", voice_name);
                }
                Ok::<(), String>(())
            })?;
        }
    }

    Ok((onnx_file, config_file))
}

pub struct TtsEngine {
    state: Arc<Mutex<TtsState>>,
    sink: Arc<Mutex<Option<rodio::Sink>>>,
    _stream: Arc<Mutex<Option<rodio::OutputStream>>>,
    stop_flag: Arc<Mutex<bool>>,
    playback: Arc<Mutex<PlaybackInfo>>,
    sentences: Arc<Mutex<Vec<String>>>,
    model: Arc<Mutex<Option<crate::kokoro_engine::TtsEngine>>>,
    model_ready: Arc<Mutex<bool>>,
    piper_model: Arc<Mutex<Option<piper_rs::Piper>>>,
    precache: Arc<Mutex<PreCache>>,
}

impl TtsEngine {
    pub fn new() -> Self {
        let stream = rodio::OutputStreamBuilder::open_default_stream()
            .expect("Failed to open audio output");

        // Warm up the audio pipeline so PulseAudio is fully initialized
        {
            let warmup = rodio::Sink::connect_new(stream.mixer());
            let silence = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, vec![0.0f32; SAMPLE_RATE as usize / 10]);
            warmup.append(silence);
            warmup.sleep_until_end();
        }

        let engine = Self {
            state: Arc::new(Mutex::new(TtsState::Loading)),
            sink: Arc::new(Mutex::new(None)),
            _stream: Arc::new(Mutex::new(Some(stream))),
            stop_flag: Arc::new(Mutex::new(false)),
            playback: Arc::new(Mutex::new(PlaybackInfo {
                generating_idx: 0,
                playing_idx: 0,
                total: 0,
                durations: vec![],
                play_start: None,
            })),
            sentences: Arc::new(Mutex::new(vec![])),
            model: Arc::new(Mutex::new(None)),
            model_ready: Arc::new(Mutex::new(false)),
            piper_model: Arc::new(Mutex::new(None)),
            precache: Arc::new(Mutex::new(PreCache {
                audio: HashMap::new(),
                page_text_hash: 0,
            })),
        };

        // Pre-load the Kokoro model in background on startup
        let model = engine.model.clone();
        let model_ready = engine.model_ready.clone();
        let state = engine.state.clone();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match crate::kokoro_engine::TtsEngine::new().await {
                    Ok(tts) => {
                        *model.lock().unwrap() = Some(tts);
                        *model_ready.lock().unwrap() = true;
                        *state.lock().unwrap() = TtsState::Idle;
                        eprintln!("Kokoro model loaded and ready.");
                    }
                    Err(e) => {
                        *state.lock().unwrap() = TtsState::Error(format!("Model load: {}", e));
                    }
                }
            });
        });

        engine
    }

    pub fn state(&self) -> TtsState {
        self.state.lock().unwrap().clone()
    }

    /// Pre-synthesize first sentences of a page in the background.
    /// Call this when a new page is rendered, before user clicks Play.
    pub fn precache_page(&self, text: &str, voice: &str) {
        if !*self.model_ready.lock().unwrap() {
            return;
        }

        // Skip precache for Piper voices (fast enough without it)
        if voice.starts_with("piper:") {
            return;
        }

        let text_hash = hash_string(text);
        {
            let cache = self.precache.lock().unwrap();
            if cache.page_text_hash == text_hash && !cache.audio.is_empty() {
                return; // already cached
            }
        }

        // Clear old cache
        {
            let mut cache = self.precache.lock().unwrap();
            cache.audio.clear();
            cache.page_text_hash = text_hash;
        }

        let sentences = split_into_sentences(text);
        if sentences.is_empty() {
            return;
        }

        let model = self.model.clone();
        let precache = self.precache.clone();
        let voice = voice.to_string();

        let stop_flag = self.stop_flag.clone();

        // Pre-generate first 3 sentences in background
        thread::spawn(move || {
            let to_cache: Vec<_> = sentences.into_iter().take(3).collect();
            for sentence in &to_cache {
                // Abort if speak() was called (stop_flag set)
                if *stop_flag.lock().unwrap() {
                    return;
                }
                // Use try_lock to avoid blocking speak()
                let samples = {
                    let model_guard = model.try_lock();
                    match model_guard {
                        Ok(mut guard) => {
                            if let Some(ref mut tts) = *guard {
                                tts.synthesize(sentence, Some(&voice))
                            } else {
                                return;
                            }
                        }
                        Err(_) => return, // model busy (speak running), abort precache
                    }
                };
                if let Ok(samples) = samples {
                    let mut cache = precache.lock().unwrap();
                    cache.audio.insert(sentence.clone(), samples);
                }
            }
            eprintln!("Pre-cached {} sentences", to_cache.len());
        });
    }

    pub fn progress(&self) -> (usize, usize, usize) {
        let pb = self.playback.lock().unwrap();
        (pb.generating_idx, pb.playing_idx + 1, pb.total)
    }

    pub fn current_sentences(&self) -> (Vec<String>, usize) {
        // Always update playing index based on elapsed time before returning
        Self::update_playing_index(&self.playback);
        let sents = self.sentences.lock().unwrap().clone();
        let playing = self.playback.lock().unwrap().playing_idx;
        (sents, playing)
    }

    fn update_playing_index(playback: &Arc<Mutex<PlaybackInfo>>) {
        let mut pb = playback.lock().unwrap();
        if pb.durations.is_empty() || pb.play_start.is_none() {
            return;
        }
        let elapsed = pb.play_start.unwrap().elapsed().as_secs_f32();
        let mut cumulative = 0.0f32;
        for (i, &dur) in pb.durations.iter().enumerate() {
            cumulative += dur;
            if elapsed < cumulative {
                pb.playing_idx = i;
                return;
            }
        }
        pb.playing_idx = pb.durations.len().saturating_sub(1);
    }

    pub fn speak(&self, text: String, voice: String, speed: f32, skip_sentences: usize) {
        self.stop();

        if !*self.model_ready.lock().unwrap() {
            *self.state.lock().unwrap() = TtsState::Error("Model still loading...".into());
            return;
        }

        let state = self.state.clone();
        let sink_holder = self.sink.clone();
        let stream_holder = self._stream.clone();
        let stop_flag = self.stop_flag.clone();
        let playback = self.playback.clone();
        let sentences_holder = self.sentences.clone();
        let model = self.model.clone();
        let piper_model = self.piper_model.clone();
        let precache = self.precache.clone();

        *stop_flag.lock().unwrap() = false;
        *state.lock().unwrap() = TtsState::Generating;

        thread::spawn(move || {
            let sentences = split_into_sentences(&text);
            if sentences.is_empty() {
                *state.lock().unwrap() = TtsState::Finished;
                return;
            }

            let total = sentences.len();
            *sentences_holder.lock().unwrap() = sentences.clone();
            {
                let mut pb = playback.lock().unwrap();
                pb.generating_idx = 0;
                pb.playing_idx = 0;
                pb.total = total;
                pb.durations.clear();
                pb.play_start = None;
            }

            let stream_guard = stream_holder.lock().unwrap();
            let stream_ref = match stream_guard.as_ref() {
                Some(s) => s,
                None => {
                    *state.lock().unwrap() = TtsState::Error("No audio device".into());
                    return;
                }
            };
            let sink = rodio::Sink::connect_new(stream_ref.mixer());
            sink.set_speed(speed);
            // Store sink immediately so pause/stop work during generation
            *sink_holder.lock().unwrap() = Some(sink);

            for (i, sentence) in sentences.iter().enumerate() {
                // Skip sentences the user already heard
                if i < skip_sentences {
                    playback.lock().unwrap().durations.push(0.0);
                    continue;
                }

                if *stop_flag.lock().unwrap() {
                    break;
                }

                playback.lock().unwrap().generating_idx = i + 1;

                // Check precache first — if already synthesized, use it instantly
                let cached = {
                    let mut cache = precache.lock().unwrap();
                    cache.audio.remove(sentence)
                };

                let is_piper = voice.starts_with("piper:");
                let samples: Result<(Vec<f32>, u32), String> = if let Some(samples) = cached {
                    Ok((samples, SAMPLE_RATE))
                } else if is_piper {
                    let piper_voice = voice.strip_prefix("piper:").unwrap();
                    // Load piper model on first use
                    let mut piper_guard = piper_model.lock().unwrap();
                    if piper_guard.is_none() {
                        match get_piper_model_paths(piper_voice) {
                            Ok((onnx, config)) => {
                                match piper_rs::Piper::new(&onnx, &config) {
                                    Ok(p) => { *piper_guard = Some(p); }
                                    Err(e) => { return eprintln!("Piper init error: {:?}", e); }
                                }
                            }
                            Err(e) => { return eprintln!("Piper download error: {}", e); }
                        }
                    }
                    if let Some(ref mut piper) = *piper_guard {
                        // Phonemize ourselves with NFD normalization (matching official Piper)
                        let phonemes = piper_phonemize(sentence, piper_voice);
                        piper.create(&phonemes, true, None, None, None, None)
                            .map_err(|e| format!("Piper error: {:?}", e))
                    } else {
                        Err("Piper not loaded".into())
                    }
                } else {
                    let mut model_guard = model.lock().unwrap();
                    if let Some(ref mut tts) = *model_guard {
                        tts.synthesize(sentence, Some(&voice)).map(|s| (s, SAMPLE_RATE))
                    } else {
                        Err("Model not loaded".into())
                    }
                };

                match samples {
                    Ok((samples, sr)) => {
                        let duration = samples.len() as f32 / sr as f32 / speed;
                        playback.lock().unwrap().durations.push(duration);

                        let source = rodio::buffer::SamplesBuffer::new(1, sr, samples);
                        if let Some(ref sink) = *sink_holder.lock().unwrap() {
                            sink.append(source);
                        }
                        if i == 0 {
                            playback.lock().unwrap().play_start = Some(Instant::now());
                            *state.lock().unwrap() = TtsState::Playing;
                        }
                    }
                    Err(e) => {
                        playback.lock().unwrap().durations.push(0.0);
                        eprintln!("Sentence {}/{} error: {}", i + 1, total, e);
                    }
                }
            }

            // Wait for playback to finish
            loop {
                if *stop_flag.lock().unwrap() {
                    break;
                }
                Self::update_playing_index(&playback);

                let empty = sink_holder
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|s| s.empty());
                if empty {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }

            if !*stop_flag.lock().unwrap() {
                sink_holder.lock().unwrap().take();
                *state.lock().unwrap() = TtsState::Finished;
            }
        });
    }

    pub fn pause(&self) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.pause();
            *self.state.lock().unwrap() = TtsState::Paused;
        }
    }

    pub fn resume(&self) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.play();
            *self.state.lock().unwrap() = TtsState::Playing;
        }
    }

    pub fn stop(&self) {
        *self.stop_flag.lock().unwrap() = true;
        if let Some(sink) = self.sink.lock().unwrap().take() {
            sink.stop();
        }
        *self.state.lock().unwrap() = TtsState::Idle;
    }

    pub fn set_speed(&self, speed: f32) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.set_speed(speed);
        }
    }

    pub fn clear_finished(&self) {
        let mut s = self.state.lock().unwrap();
        if *s == TtsState::Finished {
            *s = TtsState::Idle;
        }
    }
}
