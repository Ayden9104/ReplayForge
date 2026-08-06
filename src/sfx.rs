//! Short UI sound effects (clip saved cue).
use rodio::source::{SineWave, Source};
use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;
use std::io::Cursor;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Bundled clip-saved sound (export your recording to this path).
const CLIP_SAVED_WAV: &[u8] = include_bytes!("../assets/clip_saved.wav");

/// Base gain applied before user `sfx_volume`.
const BASE_GAIN: f32 = 0.9;

/// Play the clip-saved cue. Prefer a custom file, then the bundled WAV, then a beep.
pub fn play_clip_saved(custom_path: Option<&Path>, volume: f32) {
    let path = custom_path.map(|p| p.to_path_buf());
    let volume = volume.clamp(0.0, 2.0);
    thread::spawn(move || {
        let Ok((_stream, handle)) = OutputStream::try_default() else {
            eprintln!("clip sfx: no audio output device");
            return;
        };
        let Ok(sink) = Sink::try_new(&handle) else {
            eprintln!("clip sfx: could not create sink");
            return;
        };

        let gain = BASE_GAIN * volume;
        if let Some(ref path) = path {
            if play_file_wav(&sink, path, gain) {
                sink.sleep_until_end();
                return;
            }
        }

        if play_bundled_wav(&sink, gain) {
            sink.sleep_until_end();
            return;
        }

        play_fallback_beep(&sink, volume);
        sink.sleep_until_end();
    });
}

fn play_file_wav(sink: &Sink, path: &Path, gain: f32) -> bool {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("clip sfx: failed to open {}: {error}", path.display());
            return false;
        }
    };
    match Decoder::new(BufReader::new(file)) {
        Ok(source) => {
            sink.append(source.amplify(gain));
            true
        }
        Err(error) => {
            eprintln!("clip sfx: failed to decode {}: {error}", path.display());
            false
        }
    }
}

fn play_bundled_wav(sink: &Sink, gain: f32) -> bool {
    let cursor = Cursor::new(CLIP_SAVED_WAV);
    match Decoder::new(cursor) {
        Ok(source) => {
            sink.append(source.amplify(gain));
            true
        }
        Err(error) => {
            eprintln!("clip sfx: failed to decode assets/clip_saved.wav: {error}");
            false
        }
    }
}

fn play_fallback_beep(sink: &Sink, volume: f32) {
    let first = SineWave::new(880.0)
        .take_duration(Duration::from_millis(80))
        .amplify(0.28 * volume);
    let gap = SineWave::new(1.0)
        .take_duration(Duration::from_millis(35))
        .amplify(0.0);
    let second = SineWave::new(1174.7)
        .take_duration(Duration::from_millis(130))
        .amplify(0.26 * volume);

    sink.append(first);
    sink.append(gap);
    sink.append(second);
}
