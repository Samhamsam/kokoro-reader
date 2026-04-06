# Kokoro Reader

A minimal PDF reader with built-in text-to-speech, powered by [Kokoro TTS](https://huggingface.co/hexgrad/Kokoro-82M).

Pure Rust. Runs entirely offline. No Python needed.

## Features

- PDF viewing with page navigation
- High-quality neural text-to-speech (Kokoro 82M)
- 50+ voices across multiple languages
- Streaming playback (sentence by sentence)
- Play / Pause / Stop controls
- Voice selection and speed adjustment
- Drag & drop PDF support

## Prerequisites

- Rust 1.75+
- espeak-ng (`pacman -S espeak-ng` / `apt install espeak-ng`)
- libsonic (`pacman -S libsonic`)
- pcaudiolib (`pacman -S pcaudiolib`)
- libpdfium (download from [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases))

### Pdfium setup

```bash
mkdir -p lib/lib
cd lib
wget https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar xzf pdfium-linux-x64.tgz
```

## Build

```bash
cargo build --release
```

## Run

```bash
./target/release/kokoro-reader
```

On first run, Kokoro model files (~340MB) are automatically downloaded to `~/.cache/kokoros/`.

## Usage

1. Click **Open PDF** or drag & drop a PDF file
2. Navigate pages with **<** / **>**
3. Click **Play** to read the current page aloud
4. Select a voice from the dropdown
5. Adjust speed with the slider

## Architecture

```
egui (GUI) ── pdfium-render (PDF) ── kokoro_engine (TTS) ── rodio (Audio)
```

- **PDF**: pdfium-render for page rendering and text extraction
- **TTS**: Custom Kokoro ONNX engine with correct Misaki phonemization
- **Audio**: rodio for streaming playback with play/pause/stop
- **GUI**: egui/eframe for minimal native UI

## Credits

- [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) by hexgrad
- [kokoro-onnx](https://github.com/thewh1teagle/kokoro-onnx) (Python reference)
- [kokoro-tiny](https://github.com/8h-is/kokoro-tiny) (Rust base, patched)
- [kokoroxide](https://github.com/dhruv304c2/kokoroxide) (Misaki phonemization reference)

## License

MIT
