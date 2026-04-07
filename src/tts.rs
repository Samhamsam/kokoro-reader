use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio;

// ── Sentence splitting ──

fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() { ' ' }
            else if "♦♣♠♥★☆●○◆◇■□▪▫▲△▼▽•‣⁃※†‡§¶".contains(c) { ' ' }
            else { c }
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
            if chars[len - 3] == ' ' || chars[len - 3] == '.' { return true; }
        }
    }
    ABBREVIATIONS.iter().any(|a| text.ends_with(a))
}

pub fn split_into_sentences(text: &str) -> Vec<String> {
    let text = clean_text(text);
    if text.is_empty() { return vec![]; }
    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == '!' || ch == '?' || ch == ';' {
            let t = current.trim().to_string();
            if !t.is_empty() { sentences.push(t); }
            current.clear();
        } else if ch == '.' && !is_abbreviation(&current) {
            let t = current.trim().to_string();
            if !t.is_empty() { sentences.push(t); }
            current.clear();
        } else if ch == ':' && current.len() > 50 {
            let t = current.trim().to_string();
            if !t.is_empty() { sentences.push(t); }
            current.clear();
        }
        if current.len() > 400 {
            if let Some(pos) = current.rfind(' ') {
                let (left, right) = current.split_at(pos);
                let t = left.trim().to_string();
                if !t.is_empty() { sentences.push(t); }
                current = right.trim().to_string();
            }
        }
    }
    let t = current.trim().to_string();
    if !t.is_empty() { sentences.push(t); }
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
    reqwest::blocking::get(format!("{}/api/voices", server_url))
        .ok()
        .and_then(|r| r.json().ok())
        .unwrap_or_default()
}

// ── TTS State ──

#[derive(Clone, PartialEq)]
pub enum TtsState {
    Idle,
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
    pause_start: Option<Instant>,
    paused_elapsed: Duration,
}

pub struct TtsEngine {
    state: Arc<Mutex<TtsState>>,
    sink: Arc<Mutex<Option<rodio::Sink>>>,
    _stream: Arc<Mutex<Option<rodio::OutputStream>>>,
    generation_id: Arc<AtomicU64>, // incremented on each speak(), threads check this
    playback: Arc<Mutex<PlaybackInfo>>,
    sentences: Arc<Mutex<Vec<String>>>,
    server_url: Arc<Mutex<String>>,
}

impl TtsEngine {
    pub fn new(server_url: &str) -> Self {
        let stream = rodio::OutputStreamBuilder::open_default_stream()
            .expect("Failed to open audio output");
        {
            let warmup = rodio::Sink::connect_new(stream.mixer());
            let silence = rodio::buffer::SamplesBuffer::new(1, 24000, vec![0.0f32; 2400]);
            warmup.append(silence);
            warmup.sleep_until_end();
        }

        Self {
            state: Arc::new(Mutex::new(TtsState::Idle)),
            sink: Arc::new(Mutex::new(None)),
            _stream: Arc::new(Mutex::new(Some(stream))),
            generation_id: Arc::new(AtomicU64::new(0)),
            playback: Arc::new(Mutex::new(PlaybackInfo {
                generating_idx: 0, playing_idx: 0, total: 0,
                durations: vec![], play_start: None, pause_start: None, paused_elapsed: Duration::ZERO,
            })),
            sentences: Arc::new(Mutex::new(vec![])),
            server_url: Arc::new(Mutex::new(server_url.to_string())),
        }
    }

    pub fn set_server_url(&self, url: &str) {
        *self.server_url.lock().unwrap() = url.to_string();
    }

    pub fn state(&self) -> TtsState { self.state.lock().unwrap().clone() }

    pub fn progress(&self) -> (usize, usize, usize) {
        let pb = self.playback.lock().unwrap();
        (pb.generating_idx, pb.playing_idx + 1, pb.total)
    }

    pub fn current_sentences(&self) -> (Vec<String>, usize) {
        self.update_playing_index();
        let sents = self.sentences.lock().unwrap().clone();
        let playing = self.playback.lock().unwrap().playing_idx;
        (sents, playing)
    }

    fn update_playing_index(&self) {
        let mut pb = self.playback.lock().unwrap();
        if pb.durations.is_empty() || pb.play_start.is_none() { return; }
        // Subtract paused time from elapsed (including current ongoing pause)
        let mut total_paused = pb.paused_elapsed;
        if let Some(pause_start) = pb.pause_start {
            total_paused += pause_start.elapsed();
        }
        let elapsed = pb.play_start.unwrap().elapsed().saturating_sub(total_paused);
        let elapsed = elapsed.as_secs_f32();
        let mut cumulative = 0.0f32;
        for (i, &dur) in pb.durations.iter().enumerate() {
            cumulative += dur;
            if elapsed < cumulative { pb.playing_idx = i; return; }
        }
        pb.playing_idx = pb.durations.len().saturating_sub(1);
    }

    pub fn speak(&self, text: String, voice: String, speed: f32, skip_sentences: usize) {
        // Invalidate any previous generation
        let my_gen = self.generation_id.fetch_add(1, Ordering::SeqCst) + 1;

        // Stop old playback
        if let Some(sink) = self.sink.lock().unwrap().take() { sink.stop(); }
        *self.state.lock().unwrap() = TtsState::Generating;

        let state = self.state.clone();
        let sink_holder = self.sink.clone();
        let stream_holder = self._stream.clone();
        let gen_id = self.generation_id.clone();
        let playback = self.playback.clone();
        let sentences_holder = self.sentences.clone();
        let server_url = self.server_url.lock().unwrap().clone();

        thread::spawn(move || {
            // Check if we're still the current generation
            let is_current = || gen_id.load(Ordering::SeqCst) == my_gen;

            let sentences = split_into_sentences(&text);
            if sentences.is_empty() || !is_current() {
                if is_current() { *state.lock().unwrap() = TtsState::Finished; }
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
                pb.pause_start = None;
                pb.paused_elapsed = Duration::ZERO;
            }

            let stream_guard = stream_holder.lock().unwrap();
            let stream_ref = match stream_guard.as_ref() {
                Some(s) => s,
                None => { if is_current() { *state.lock().unwrap() = TtsState::Error("No audio".into()); } return; }
            };
            let sink = rodio::Sink::connect_new(stream_ref.mixer());
            sink.set_speed(speed);
            *sink_holder.lock().unwrap() = Some(sink);

            for (i, sentence) in sentences.iter().enumerate() {
                if !is_current() { return; }
                if i < skip_sentences {
                    playback.lock().unwrap().durations.push(0.0);
                    continue;
                }

                playback.lock().unwrap().generating_idx = i + 1;

                let wav_data = request_tts(&server_url, sentence, &voice, speed);

                if !is_current() { return; }

                match wav_data {
                    Some((samples, sample_rate)) => {
                        let duration = samples.len() as f32 / sample_rate as f32 / speed;
                        playback.lock().unwrap().durations.push(duration);

                        let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
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
                        eprintln!("TTS error for sentence {}/{}", i + 1, total);
                    }
                }
            }

            // Wait for playback
            loop {
                if !is_current() { return; }
                let empty = sink_holder.lock().unwrap().as_ref().is_some_and(|s| s.empty());
                if empty { break; }
                thread::sleep(Duration::from_millis(50));
            }

            if is_current() {
                sink_holder.lock().unwrap().take();
                *state.lock().unwrap() = TtsState::Finished;
            }
        });
    }

    pub fn precache_page(&self, _text: &str, _voice: &str) {}

    pub fn pause(&self) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.pause();
            let mut pb = self.playback.lock().unwrap();
            pb.pause_start = Some(Instant::now());
            *self.state.lock().unwrap() = TtsState::Paused;
        }
    }

    pub fn resume(&self) {
        if let Some(sink) = self.sink.lock().unwrap().as_ref() {
            sink.play();
            let mut pb = self.playback.lock().unwrap();
            if let Some(pause_start) = pb.pause_start.take() {
                pb.paused_elapsed += pause_start.elapsed();
            }
            *self.state.lock().unwrap() = TtsState::Playing;
        }
    }

    pub fn stop(&self) {
        self.generation_id.fetch_add(1, Ordering::SeqCst);
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

/// Request TTS from Go server, returns (PCM f32 samples, sample_rate)
fn request_tts(server_url: &str, text: &str, voice: &str, speed: f32) -> Option<(Vec<f32>, u32)> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{}/api/tts", server_url))
        .json(&serde_json::json!({"text": text, "voice": voice, "speed": speed}))
        .timeout(Duration::from_secs(30))
        .send().ok()?;
    if !resp.status().is_success() { return None; }
    let wav = resp.bytes().ok()?;
    if wav.len() < 44 { return None; }

    // Parse sample rate from WAV header (bytes 24-27)
    let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    let num_samples = (wav.len() - 44) / 2;
    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let off = 44 + i * 2;
        let s16 = i16::from_le_bytes([wav[off], wav[off + 1]]);
        samples.push(s16 as f32 / 32767.0);
    }
    Some((samples, sample_rate))
}
