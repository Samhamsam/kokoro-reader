use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio;

const SAMPLE_RATE: u32 = 24000;

// ── Sentence splitting ──

fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else if "♦♣♠♥★☆●○◆◇■□▪▫▲△▼▽•‣⁃※†‡§¶".contains(c) {
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

fn is_abbreviation(text: &str) -> bool {
    let text = text.trim_end();
    if !text.ends_with('.') { return false; }
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len >= 2 {
        let before_dot = chars[len - 2];
        if before_dot.is_uppercase() {
            if len == 2 { return true; }
            let before_letter = chars[len - 3];
            if before_letter == ' ' || before_letter == '.' { return true; }
        }
    }
    for abbr in ABBREVIATIONS { if text.ends_with(abbr) { return true; } }
    false
}

pub fn split_into_sentences(text: &str) -> Vec<String> {
    let text = clean_text(text);
    if text.is_empty() { return vec![]; }

    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if ch == '!' || ch == '?' || ch == ';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() { sentences.push(trimmed); }
            current.clear();
        } else if ch == '.' {
            if !is_abbreviation(&current) {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() { sentences.push(trimmed); }
                current.clear();
            }
        } else if ch == ':' && current.len() > 50 {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() { sentences.push(trimmed); }
            current.clear();
        }
        if current.len() > 400 {
            if let Some(pos) = current.rfind(' ') {
                let (left, right) = current.split_at(pos);
                let trimmed = left.trim().to_string();
                if !trimmed.is_empty() { sentences.push(trimmed); }
                current = right.trim().to_string();
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() { sentences.push(trimmed); }
    sentences.retain(|s| s.chars().filter(|c| c.is_alphabetic()).count() >= 2);
    sentences
}

// ── Voice list from server ──

#[derive(Clone, serde::Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub lang: String,
}

pub fn fetch_voices(server_url: &str) -> Vec<Voice> {
    let url = format!("{}/voices", server_url);
    reqwest::blocking::get(&url)
        .ok()
        .and_then(|r| r.json::<Vec<Voice>>().ok())
        .unwrap_or_default()
}

// ── TTS State ──

#[derive(Clone, PartialEq)]
pub enum TtsState {
    Idle,
    Loading,
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
    server_url: Arc<Mutex<String>>,
}

impl TtsEngine {
    pub fn new(server_url: &str) -> Self {
        let stream = rodio::OutputStreamBuilder::open_default_stream()
            .expect("Failed to open audio output");

        // Warm up audio pipeline
        {
            let warmup = rodio::Sink::connect_new(stream.mixer());
            let silence = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, vec![0.0f32; SAMPLE_RATE as usize / 10]);
            warmup.append(silence);
            warmup.sleep_until_end();
        }

        Self {
            state: Arc::new(Mutex::new(TtsState::Idle)),
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
            server_url: Arc::new(Mutex::new(server_url.to_string())),
        }
    }

    pub fn set_server_url(&self, url: &str) {
        *self.server_url.lock().unwrap() = url.to_string();
    }

    pub fn state(&self) -> TtsState {
        self.state.lock().unwrap().clone()
    }

    pub fn progress(&self) -> (usize, usize, usize) {
        let pb = self.playback.lock().unwrap();
        (pb.generating_idx, pb.playing_idx + 1, pb.total)
    }

    pub fn current_sentences(&self) -> (Vec<String>, usize) {
        Self::update_playing_index(&self.playback);
        let sents = self.sentences.lock().unwrap().clone();
        let playing = self.playback.lock().unwrap().playing_idx;
        (sents, playing)
    }

    fn update_playing_index(playback: &Arc<Mutex<PlaybackInfo>>) {
        let mut pb = playback.lock().unwrap();
        if pb.durations.is_empty() || pb.play_start.is_none() { return; }
        let elapsed = pb.play_start.unwrap().elapsed().as_secs_f32();
        let mut cumulative = 0.0f32;
        for (i, &dur) in pb.durations.iter().enumerate() {
            cumulative += dur;
            if elapsed < cumulative { pb.playing_idx = i; return; }
        }
        pb.playing_idx = pb.durations.len().saturating_sub(1);
    }

    pub fn speak(&self, text: String, voice: String, speed: f32, skip_sentences: usize) {
        self.stop();

        let state = self.state.clone();
        let sink_holder = self.sink.clone();
        let stream_holder = self._stream.clone();
        let stop_flag = self.stop_flag.clone();
        let playback = self.playback.clone();
        let sentences_holder = self.sentences.clone();
        let server_url = self.server_url.lock().unwrap().clone();

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
                None => { *state.lock().unwrap() = TtsState::Error("No audio device".into()); return; }
            };
            let sink = rodio::Sink::connect_new(stream_ref.mixer());
            sink.set_speed(speed);
            *sink_holder.lock().unwrap() = Some(sink);

            for (i, sentence) in sentences.iter().enumerate() {
                if i < skip_sentences {
                    playback.lock().unwrap().durations.push(0.0);
                    continue;
                }
                if *stop_flag.lock().unwrap() { break; }

                playback.lock().unwrap().generating_idx = i + 1;

                // Request audio from Go server
                let wav_data = request_tts(&server_url, sentence, &voice, speed);

                match wav_data {
                    Some(samples) => {
                        let duration = samples.len() as f32 / SAMPLE_RATE as f32 / speed;
                        playback.lock().unwrap().durations.push(duration);

                        let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, samples);
                        if let Some(ref sink) = *sink_holder.lock().unwrap() {
                            sink.append(source);
                        }
                        if i == skip_sentences {
                            playback.lock().unwrap().play_start = Some(Instant::now());
                            *state.lock().unwrap() = TtsState::Playing;
                        }
                    }
                    None => {
                        playback.lock().unwrap().durations.push(0.0);
                        eprintln!("Sentence {}/{} error from server", i + 1, total);
                    }
                }
            }

            // Wait for playback to finish
            loop {
                if *stop_flag.lock().unwrap() { break; }
                Self::update_playing_index(&playback);
                let empty = sink_holder.lock().unwrap().as_ref().is_some_and(|s| s.empty());
                if empty { break; }
                thread::sleep(Duration::from_millis(50));
            }

            if !*stop_flag.lock().unwrap() {
                sink_holder.lock().unwrap().take();
                *state.lock().unwrap() = TtsState::Finished;
            }
        });
    }

    pub fn precache_page(&self, _text: &str, _voice: &str) {
        // No-op: server handles caching
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
        if let Some(sink) = self.sink.lock().unwrap().take() { sink.stop(); }
        *self.state.lock().unwrap() = TtsState::Idle;
    }

    pub fn set_speed(&self, speed: f32) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() { sink.set_speed(speed); }
    }

    pub fn clear_finished(&self) {
        let mut s = self.state.lock().unwrap();
        if *s == TtsState::Finished { *s = TtsState::Idle; }
    }
}

/// Request TTS from Go server, returns PCM f32 samples
fn request_tts(server_url: &str, text: &str, voice: &str, speed: f32) -> Option<Vec<f32>> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{}/tts", server_url))
        .json(&serde_json::json!({
            "text": text,
            "voice": voice,
            "speed": speed
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .ok()?;

    if !resp.status().is_success() { return None; }

    let wav_data = resp.bytes().ok()?;
    if wav_data.len() < 44 { return None; }

    // Decode WAV → f32 samples
    let num_samples = (wav_data.len() - 44) / 2;
    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let offset = 44 + i * 2;
        let s16 = i16::from_le_bytes([wav_data[offset], wav_data[offset + 1]]);
        samples.push(s16 as f32 / 32767.0);
    }
    Some(samples)
}
