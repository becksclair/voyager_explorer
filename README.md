# Voyager Golden Record Explorer

A Rust + Egui app that decodes and visualizes the analog image data encoded on the **Voyager Golden Record**.

> Real-time decoding of SSTV-style audio into grayscale images, inspired by the iconic legacy of humanity's interstellar message.

---

## 🎯 Goals

- ✅ Real-time decoding of Voyager Golden Record WAV audio into monochrome images.
- ✅ Visual waveform display and interaction.
- ✅ Interactive GUI with Egui and `eframe`.
- ✅ Audio playback and streaming decode using `rodio` and `hound`.
- ✅ Adjustable decoding parameters (line timing, amplitude threshold, etc).
- 🧠 Modular architecture for real-time decoding, image caching, UI components, and settings.
- 🚧 (Soon) Tiled image paging system for high-resolution viewing beyond GPU texture limits.
- 🚧 (Soon) Color image decoding support (based on image type, demuxed channels).
- 🚧 (Future) Decoding parameter presets for various known image types on the record.
- 🚧 (Future) Reverse FFT tools, modulation analysis, or analog emulation tools.

---

## 🧱 Project Structure

```text
src/
├── main.rs               # Egui app setup and window
├── app.rs                # UI state and interaction
├── decoder.rs            # Audio stream → decoded pixel rows
├── waveform.rs           # Visualizes live audio waveform
├── image_output.rs       # Grayscale image construction & tiling
├── audio.rs              # Hound + Rodio integration
└── utils.rs              # Filters, transforms, helpers
assets/
└── golden_record_*.wav   # Raw audio data from Voyager record
```

🛠 Dependencies
egui + eframe — GUI framework

rodio — Audio playback

hound — WAV file reading

[crossbeam / rayon / dashmap] — (planned) async and dataflow support

🪐 What's Special?
This project attempts to faithfully recreate how the Voyager Golden Record's image data was meant to be read, decoding from audio waveform back into viewable raster images.

By exposing the decoding steps, we hope to make this process:

🔬 Educational

🎨 Artistic

🛠 Hackable

👩‍💻 Contributing
We're building this modularly with real-time UX in mind — contributions welcome! Focus areas:

Decoding algorithms & optimization

Visual UI polish

Parameter tuning & presets

Color image support

Saved session state / caching

📡 Inspiration
NASA JPL – Voyager Golden Record

Ham radio SSTV

[Analog decoding projects like Baofeng SSTV, and QRSS]

✨ License
MIT — free to use, remix, study.
Dedicated to the spirit of curiosity, science, and messages in bottles thrown into the cosmic ocean.

“To the makers of music — all worlds, all times.”
