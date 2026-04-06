use std::io::Cursor;

#[tokio::main]
async fn main() {
    let mut tts = kokoro_reader::kokoro_engine::TtsEngine::new().await.unwrap();

    // Test German text with different existing voices + German espeak
    let text = "Der Regenbogen ist ein wunderschoenes Naturphaenomen am Himmel.";
    let voices = ["af_heart", "af_nova", "bf_emma", "ff_siwis"];

    for voice in &voices {
        println!("=== {} with German phonemization ===", voice);
        match tts.synthesize(text, Some(voice), "de") {
            Ok(samples) => {
                let stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
                let sink = rodio::Sink::connect_new(stream.mixer());
                let source = rodio::buffer::SamplesBuffer::new(1, 24000, samples);
                sink.append(source);
                sink.sleep_until_end();
                println!("Done.");
            }
            Err(e) => println!("Error: {}", e),
        }
    }
}
