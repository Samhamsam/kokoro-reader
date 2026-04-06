use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hound;
use rodio;

pub const VOICES: &[(&str, &str)] = &[
    ("af_heart", "Heart (F, US)"),
    ("af_nova", "Nova (F, US)"),
    ("af_sky", "Sky (F, US)"),
    ("af_sarah", "Sarah (F, US)"),
    ("af_bella", "Bella (F, US)"),
    ("am_adam", "Adam (M, US)"),
    ("am_michael", "Michael (M, US)"),
    ("am_eric", "Eric (M, US)"),
    ("bf_emma", "Emma (F, GB)"),
    ("bf_alice", "Alice (F, GB)"),
    ("bm_george", "George (M, GB)"),
    ("bm_daniel", "Daniel (M, GB)"),
    ("ff_siwis", "Siwis (F, FR)"),
    ("if_sara", "Sara (F, IT)"),
    ("jf_alpha", "Alpha (F, JA)"),
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

pub struct TtsEngine {
    state: Arc<Mutex<TtsState>>,
    sink: Arc<Mutex<Option<rodio::Sink>>>,
    _stream: Arc<Mutex<Option<rodio::OutputStream>>>,
    stop_flag: Arc<Mutex<bool>>,
    playback: Arc<Mutex<PlaybackInfo>>,
    sentences: Arc<Mutex<Vec<String>>>,
    /// Persistent Kokoro model — loaded once, reused across pages
    model: Arc<Mutex<Option<crate::kokoro_engine::TtsEngine>>>,
    model_ready: Arc<Mutex<bool>>,
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

    pub fn speak(&self, text: String, voice: String, speed: f32) {
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
                if *stop_flag.lock().unwrap() {
                    break;
                }

                playback.lock().unwrap().generating_idx = i + 1;

                let samples = {
                    let mut model_guard = model.lock().unwrap();
                    if let Some(ref mut tts) = *model_guard {
                        tts.synthesize(sentence, Some(&voice))
                    } else {
                        Err("Model not loaded".into())
                    }
                };

                match samples {
                    Ok(samples) => {
                        let duration = samples.len() as f32 / SAMPLE_RATE as f32 / speed;
                        playback.lock().unwrap().durations.push(duration);

                        let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, samples);
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
