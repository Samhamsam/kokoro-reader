use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;

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

/// Clean PDF text: remove control chars, collapse whitespace
fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split text into sentences at natural boundaries
fn split_into_sentences(text: &str) -> Vec<String> {
    let text = clean_text(text);
    if text.is_empty() {
        return vec![];
    }

    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if (ch == '.' || ch == '!' || ch == '?' || ch == ';')
            && current.len() > 5
        {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
        // Force split at ~400 chars if no sentence boundary found
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
    sentences
}

/// Audio compressor — evens out volume so quiet words become audible
/// and loud peaks are tamed. Same principle as broadcast radio.
fn compress_audio(samples: &mut [f32]) {
    if samples.is_empty() {
        return;
    }

    let threshold = 0.15;   // start compressing above this level
    let ratio = 4.0;        // 4:1 compression ratio
    let attack = 0.002;     // 2ms attack (fast, catches transients)
    let release = 0.05;     // 50ms release (smooth decay)
    let makeup_gain = 2.5;  // boost everything up after compression

    let attack_coeff = 1.0 - (-1.0 / (SAMPLE_RATE as f32 * attack)).exp();
    let release_coeff = 1.0 - (-1.0 / (SAMPLE_RATE as f32 * release)).exp();

    let mut envelope = 0.0f32;

    for sample in samples.iter_mut() {
        let abs = sample.abs();

        // Envelope follower: fast attack, slow release
        if abs > envelope {
            envelope += attack_coeff * (abs - envelope);
        } else {
            envelope += release_coeff * (abs - envelope);
        }

        // Compute gain reduction
        let gain = if envelope > threshold {
            let over = envelope / threshold;
            let compressed = over.powf(1.0 / ratio);
            (threshold * compressed) / envelope
        } else {
            1.0
        };

        *sample = (*sample * gain * makeup_gain).clamp(-0.95, 0.95);
    }
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
    Generating,
    Playing,
    Paused,
    Error(String),
}

pub struct TtsEngine {
    state: Arc<Mutex<TtsState>>,
    sink: Arc<Mutex<Option<rodio::Sink>>>,
    _stream: Arc<Mutex<Option<rodio::OutputStream>>>,
    stop_flag: Arc<Mutex<bool>>,
    progress: Arc<Mutex<(usize, usize)>>, // (current_sentence, total_sentences)
}

impl TtsEngine {
    pub fn new() -> Self {
        let stream = rodio::OutputStreamBuilder::open_default_stream()
            .expect("Failed to open audio output");

        Self {
            state: Arc::new(Mutex::new(TtsState::Idle)),
            sink: Arc::new(Mutex::new(None)),
            _stream: Arc::new(Mutex::new(Some(stream))),
            stop_flag: Arc::new(Mutex::new(false)),
            progress: Arc::new(Mutex::new((0, 0))),
        }
    }

    pub fn state(&self) -> TtsState {
        self.state.lock().unwrap().clone()
    }

    pub fn progress(&self) -> (usize, usize) {
        *self.progress.lock().unwrap()
    }

    pub fn speak(&self, text: String, voice: String, speed: f32) {
        self.stop();

        let state = self.state.clone();
        let sink_holder = self.sink.clone();
        let stream_holder = self._stream.clone();
        let stop_flag = self.stop_flag.clone();
        let progress = self.progress.clone();

        *stop_flag.lock().unwrap() = false;
        *state.lock().unwrap() = TtsState::Generating;
        *progress.lock().unwrap() = (0, 0);

        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tts = match crate::kokoro_engine::TtsEngine::new().await {
                    Ok(t) => t,
                    Err(e) => {
                        *state.lock().unwrap() = TtsState::Error(e);
                        return;
                    }
                };
                let tts = Arc::new(Mutex::new(tts));

                let sentences = split_into_sentences(&text);
                if sentences.is_empty() {
                    *state.lock().unwrap() = TtsState::Error("No text to speak".into());
                    return;
                }

                let total = sentences.len();
                *progress.lock().unwrap() = (0, total);

                // Create sink for queued playback
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

                // Stream: synthesize each sentence and append to sink queue
                for (i, sentence) in sentences.iter().enumerate() {
                    if *stop_flag.lock().unwrap() {
                        break;
                    }

                    *progress.lock().unwrap() = (i + 1, total);

                    // First sentence: show "Generating", after that "Playing"
                    if i == 0 {
                        *state.lock().unwrap() = TtsState::Generating;
                    }

                    let samples = {
                        let mut tts_guard = tts.lock().unwrap();
                        tts_guard.synthesize(sentence, Some(&voice))
                    };

                    match samples {
                        Ok(mut samples) => {
                            compress_audio(&mut samples);
                            let wav_data = samples_to_wav(&samples);
                            let cursor = Cursor::new(wav_data);
                            if let Ok(source) = rodio::Decoder::new(cursor) {
                                sink.append(source);
                                // After first sentence is queued, we're "playing"
                                if i == 0 {
                                    *state.lock().unwrap() = TtsState::Playing;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Sentence {}/{} error (skipping): {}", i + 1, total, e);
                        }
                    }
                }

                // Store sink so pause/stop work
                *sink_holder.lock().unwrap() = Some(sink);

                // Wait for playback to finish (unless stopped)
                loop {
                    if *stop_flag.lock().unwrap() {
                        break;
                    }
                    let empty = sink_holder
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|s| s.empty());
                    if empty {
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }

                if !*stop_flag.lock().unwrap() {
                    sink_holder.lock().unwrap().take();
                    *state.lock().unwrap() = TtsState::Idle;
                }
            });
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
}
