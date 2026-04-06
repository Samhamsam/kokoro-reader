use piper_rs::Piper;
use std::path::Path;

fn main() {
    let model = Path::new(&std::env::var("HOME").unwrap())
        .join(".cache/piper-voices/de_DE-thorsten-high.onnx");
    let config = Path::new(&std::env::var("HOME").unwrap())
        .join(".cache/piper-voices/de_DE-thorsten-high.onnx.json");

    println!("Loading Piper model...");
    let mut piper = Piper::new(&model, &config).unwrap();

    let text = "Der Regenbogen ist ein atmosphaerisch optisches Phaenomen, das als kreisbogenfoermiges farbiges Lichtband wahrgenommen wird.";
    println!("Synthesizing: {}", text);
    let (samples, sample_rate) = piper.create(text, false, None, None, None, None).unwrap();
    println!("Got {} samples at {}Hz, duration: {:.1}s", samples.len(), sample_rate, samples.len() as f32 / sample_rate as f32);

    println!("Playing...");
    let stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.append(rodio::buffer::SamplesBuffer::new(1, sample_rate, samples));
    sink.sleep_until_end();
    println!("Done.");
}
