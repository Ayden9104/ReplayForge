//! Short UI sound effects (clip saved cue).
use rodio::source::{SineWave, Source};
use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;
use std::thread;
use std::time::Duration;

/// Bundled clip-saved sound (export your recording to this path).
const CLIP_SAVED_WAV: &[u8] = include_bytes!("../assets/clip_saved.wav");

/// Play the clip-saved cue. Prefer the bundled WAV; fall back to a short beep.
pub fn play_clip_saved() {
    thread::spawn(|| {
        let Ok((_stream, handle)) = OutputStream::try_default() else {
            eprintln!("clip sfx: no audio output device");
            return;
        };
        let Ok(sink) = Sink::try_new(&handle) else {
            eprintln!("clip sfx: could not create sink");
            return;
        };

        if play_bundled_wav(&sink) {
            sink.sleep_until_end();
            return;
        }

        play_fallback_beep(&sink);
        sink.sleep_until_end();
    });
}

fn play_bundled_wav(sink: &Sink) -> bool {
    let cursor = Cursor::new(CLIP_SAVED_WAV);
    match Decoder::new(cursor) {
        Ok(source) => {
            sink.append(source.amplify(0.9));
            true
        }
        Err(error) => {
            eprintln!("clip sfx: failed to decode assets/clip_saved.wav: {error}");
            false
        }
    }
}

fn play_fallback_beep(sink: &Sink) {
    let first = SineWave::new(880.0)
        .take_duration(Duration::from_millis(80))
        .amplify(0.28);
    let gap = SineWave::new(1.0)
        .take_duration(Duration::from_millis(35))
        .amplify(0.0);
    let second = SineWave::new(1174.7)
        .take_duration(Duration::from_millis(130))
        .amplify(0.26);

    sink.append(first);
    sink.append(gap);
    sink.append(second);
}
