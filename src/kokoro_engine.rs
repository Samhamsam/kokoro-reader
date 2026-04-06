//! Kokoro TTS engine — based on kokoro-tiny with fixes for correct phonemization
//! and style vector selection matching the Python reference implementation.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use espeak_rs::text_to_phonemes;
use ndarray::{ArrayBase, IxDyn, OwnedRepr};
use ndarray_npy::NpzReader;
use ort::{
    session::{builder::GraphOptimizationLevel, Session, SessionInputs, SessionInputValue},
    value::{Tensor, Value},
};

const MODEL_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";
const VOICES_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";
const SAMPLE_RATE: u32 = 24000;
const DEFAULT_VOICE: &str = "af_sky";
const DEFAULT_SPEED: f32 = 1.0;

fn get_cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".cache").join("kokoros")
}

pub struct TtsEngine {
    session: Arc<Mutex<Session>>,
    voices: HashMap<String, Vec<f32>>,
    vocab: HashMap<char, i64>,
}

impl TtsEngine {
    pub async fn new() -> Result<Self, String> {
        let cache_dir = get_cache_dir();
        let model_path = cache_dir.join("kokoro-v1.0.onnx");
        let voices_path = cache_dir.join("voices-v1.0.bin");

        let model_str = model_path.to_str().unwrap_or("kokoro-v1.0.onnx");
        let voices_str = voices_path.to_str().unwrap_or("voices-v1.0.bin");

        if let Some(parent) = model_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {}", e))?;
        }

        if !model_path.exists() {
            download_file(MODEL_URL, model_str).await
                .map_err(|e| format!("Failed to download model: {}", e))?;
        }
        if !voices_path.exists() {
            download_file(VOICES_URL, voices_str).await
                .map_err(|e| format!("Failed to download voices: {}", e))?;
        }

        let model_bytes = std::fs::read(model_str)
            .map_err(|e| format!("Failed to read model file: {}", e))?;
        let session = Session::builder()
            .map_err(|e| format!("Failed to create session builder: {}", e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("Failed to set optimization level: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| format!("Failed to set intra threads: {}", e))?
            .commit_from_memory(&model_bytes)
            .map_err(|e| format!("Failed to load model: {}", e))?;

        let voices = load_voices(voices_str)?;
        let vocab = build_vocab();

        Ok(Self {
            session: Arc::new(Mutex::new(session)),
            voices,
            vocab,
        })
    }

    pub fn synthesize(&mut self, text: &str, voice: Option<&str>) -> Result<Vec<f32>, String> {
        let voice = voice.unwrap_or(DEFAULT_VOICE);

        // Get voice data (all 510 style vectors, each 256 floats)
        let voice_data = self.voices.get(voice)
            .ok_or_else(|| format!("Voice '{}' not found", voice))?;

        // Convert text to phonemes using espeak-ng (American English)
        let phonemes = text_to_phonemes(text, "en-us", None, true, true)
            .map_err(|e| format!("Failed to convert text to phonemes: {:?}", e))?;
        let phonemes_str = phonemes.join("");

        // Apply same minimal replacements as Python kokoro-onnx tokenizer
        let cleaned = phoneme_cleanup(&phonemes_str, &self.vocab);

        // Tokenize
        let tokens = self.tokenize(&cleaned);
        let token_len = tokens[0].len();

        // Select style vector based on token length (matching Python: voice[len(tokens)])
        let style_idx = token_len.min(509);
        let start = style_idx * 256;
        let end = start + 256;
        let style = if end <= voice_data.len() {
            voice_data[start..end].to_vec()
        } else {
            voice_data[0..256].to_vec()
        };

        let audio = self.infer(tokens, style, DEFAULT_SPEED)?;
        Ok(audio)
    }

    pub fn voices(&self) -> Vec<String> {
        self.voices.keys().cloned().collect()
    }

    fn tokenize(&self, phonemes: &str) -> Vec<Vec<i64>> {
        let mut tokens: Vec<i64> = Vec::with_capacity(phonemes.len() + 2);
        tokens.push(0); // BOS padding token
        tokens.extend(phonemes.chars().filter_map(|c| self.vocab.get(&c).copied()));
        tokens.push(0); // EOS padding token
        vec![tokens]
    }

    fn infer(&mut self, tokens: Vec<Vec<i64>>, style: Vec<f32>, speed: f32) -> Result<Vec<f32>, String> {
        let mut session = self.session.lock().unwrap();

        let tokens_shape = [tokens.len(), tokens[0].len()];
        let tokens_flat: Vec<i64> = tokens.into_iter().flatten().collect();
        let tokens_tensor = Tensor::from_array((tokens_shape, tokens_flat))
            .map_err(|e| format!("Failed to create tokens tensor: {}", e))?;

        let style_shape = [1, style.len()];
        let style_tensor = Tensor::from_array((style_shape, style))
            .map_err(|e| format!("Failed to create style tensor: {}", e))?;

        let speed_tensor = Tensor::from_array(([1], vec![speed]))
            .map_err(|e| format!("Failed to create speed tensor: {}", e))?;

        use std::borrow::Cow;
        let inputs = SessionInputs::from(vec![
            (Cow::Borrowed("tokens"), SessionInputValue::Owned(Value::from(tokens_tensor))),
            (Cow::Borrowed("style"), SessionInputValue::Owned(Value::from(style_tensor))),
            (Cow::Borrowed("speed"), SessionInputValue::Owned(Value::from(speed_tensor))),
        ]);

        let outputs = session.run(inputs)
            .map_err(|e| format!("Failed to run inference: {}", e))?;

        let (_shape, data) = outputs["audio"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract audio tensor: {}", e))?;

        Ok(data.to_vec())
    }
}

/// Minimal phoneme cleanup matching Python's kokoro-onnx tokenizer.
fn phoneme_cleanup(phonemes: &str, vocab: &HashMap<char, i64>) -> String {
    let mut result = phonemes
        .replace("ʲ", "j")
        .replace("r", "ɹ")
        .replace("x", "k")
        .replace("ɬ", "l");
    result.retain(|c| vocab.contains_key(&c));
    result
}

fn build_vocab() -> HashMap<char, i64> {
    let pad = "$";
    let punctuation = r#";:,.!?¡¿—…"«»"" "#;
    let letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let letters_ipa = "ɑɐɒæɓʙβɔɕçɗɖðʤəɘɚɛɜɝɞɟʄɡɠɢʛɦɧħɥʜɨɪʝɭɬɫɮʟɱɯɰŋɳɲɴøɵɸθœɶʘɹɺɾɻʀʁɽʂʃʈʧʉʊʋⱱʌɣɤʍχʎʏʑʐʒʔʡʕʢǀǁǂǃˈˌːˑʼʴʰʱʲʷˠˤ˞↓↑→↗↘'̩'ᵻ";

    let symbols: String = [pad, punctuation, letters, letters_ipa].concat();
    symbols
        .chars()
        .enumerate()
        .map(|(idx, c)| (c, idx as i64))
        .collect()
}

fn load_voices(path: &str) -> Result<HashMap<String, Vec<f32>>, String> {
    let mut npz = NpzReader::new(File::open(path).map_err(|e| format!("Failed to open voices file: {}", e))?)
        .map_err(|e| format!("Failed to read NPZ: {:?}", e))?;
    let mut voices = HashMap::new();

    for name in npz.names().map_err(|e| format!("Failed to get NPZ names: {:?}", e))? {
        let arr: ArrayBase<OwnedRepr<f32>, IxDyn> = npz.by_name(&name)
            .map_err(|e| format!("Failed to read voice {}: {:?}", name, e))?;

        let shape = arr.shape();
        if shape.len() == 3 && shape[1] == 1 && shape[2] == 256 {
            // Store ALL 510 style vectors (not just the first one!)
            // Python selects voice[len(tokens)] at inference time
            let data = arr.as_slice()
                .ok_or_else(|| format!("Failed to get slice for voice {}", name))?
                .to_vec();
            voices.insert(name.trim_end_matches(".npy").to_string(), data);
        }
    }

    Ok(voices)
}

async fn download_file(url: &str, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    println!("Downloading {} to {}...", url, path);
    let response = reqwest::get(url).await?;
    let bytes = response.bytes().await?;
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    println!("Downloaded successfully!");
    Ok(())
}
