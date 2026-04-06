use piper_rs::Piper;
use unicode_normalization::UnicodeNormalization;
use std::path::Path;

fn main() {
    let home = std::env::var("HOME").unwrap();
    let model = Path::new(&home).join(".cache/piper-voices/de_DE-thorsten-high.onnx");
    let config = Path::new(&home).join(".cache/piper-voices/de_DE-thorsten-high.onnx.json");

    println!("Loading Piper model...");
    let mut piper = Piper::new(&model, &config).unwrap();

    let text = "Das Sonnenlicht bricht sich in den Regentropfen und erzeugt einen wunderschoenen Regenbogen.";

    // Method 1: piper-rs default phonemization (no NFD)
    println!("\n=== WITHOUT NFD (old) ===");
    let (samples, sr) = piper.create(text, false, None, None, None, None).unwrap();
    let stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.append(rodio::buffer::SamplesBuffer::new(1, sr, samples));
    sink.sleep_until_end();

    std::thread::sleep(std::time::Duration::from_millis(800));

    // Method 2: our phonemization with NFD normalization
    println!("=== WITH NFD (new) ===");
    let phonemes = espeak_rs::text_to_phonemes(text, "de", None, false, false)
        .unwrap().join(" ");
    let phonemes_nfd: String = phonemes.nfd().collect();
    println!("Phonemes: {}", phonemes);
    println!("NFD:      {}", phonemes_nfd);

    let (samples, sr) = piper.create(&phonemes_nfd, true, None, None, None, None).unwrap();
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.append(rodio::buffer::SamplesBuffer::new(1, sr, samples));
    sink.sleep_until_end();

    println!("Done.");
}
